#![deny(
    unsafe_op_in_unsafe_fn,
    warnings,
    clippy::all,
    clippy::as_conversions,
    clippy::multiple_unsafe_ops_per_block,
    clippy::undocumented_unsafe_blocks,
    clippy::unnecessary_safety_comment,
    clippy::unnecessary_safety_doc
)]
#![warn(clippy::pedantic)]

mod arguments;
mod monitor_listener;
mod sigmask_guard;
mod stop_guard;
mod terminal_guard;

use anyhow::{Context, Error, Result, anyhow, bail};
use arguments::{Arguments, ChildArguments};
use log::info;
use monitor_listener::MonitorListener;
use nix::{
    errno::Errno,
    fcntl::{FcntlArg, FdFlag, fcntl},
    libc::c_int,
    sys::{
        select::{FdSet, pselect},
        signal::{Signal, kill},
        wait::{WaitPidFlag, WaitStatus, waitpid},
    },
    unistd::{ForkResult, Pid, fork, pipe, setpgid},
};
use sigmask_guard::SigmaskGuard;
use signal_hook::{
    consts::{SIGCHLD, SIGHUP, SIGINT, SIGQUIT, SIGTERM, SIGTSTP, SIGTTIN, SIGTTOU},
    iterator::{SignalsInfo, exfiltrator::WithRawSiginfo},
    low_level::emulate_default_handler,
};
use std::{
    convert::Infallible,
    env,
    io::Write as _,
    os::{
        fd::{AsFd, AsRawFd, OwnedFd},
        unix::{
            net::UnixStream,
            process::{ExitStatusExt as _, parent_id},
        },
    },
    path::{Path, PathBuf},
    process::{self, Child, Command},
};
use stop_guard::StopGuard;
use terminal_guard::TerminalGuard;

pub struct QemuWrapper {
    sigmask: SigmaskGuard,
}

type Signals = SignalsInfo<WithRawSiginfo>;

fn spawn_qemu_child(
    arguments: ChildArguments,
    listener: &Path,
    vm_close_on_ready: OwnedFd,
) -> Result<Child> {
    #[cfg(target_os = "macos")]
    const PLUGIN_EXTENSION: &str = "dylib";
    #[cfg(target_os = "windows")]
    const PLUGIN_EXTENSION: &str = "dll";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    const PLUGIN_EXTENSION: &str = "so";

    let ChildArguments {
        ready_for_exit_signal_port,
        exit_port,
        guest_architecture,
        qemu_arguments,
    } = arguments;

    let mut plugin = PathBuf::from("libqemu_plugin_ready");
    plugin.set_extension(PLUGIN_EXTENSION);

    if let Some(plugin_path) = env::var_os("QEMU_PLUGIN_PATH") {
        plugin = PathBuf::from_iter([plugin_path, plugin.into_os_string()]);
    }

    let program = format!("qemu-system-{guest_architecture}");
    let result = Command::new(&program)
        .args([
            "-chardev",
            &format!(
                "socket,id=mon0,path={},server=off",
                listener
                    .to_str()
                    .ok_or_else(|| anyhow!("invalid listener: '{listener:?}'"))?
            ),
            "-mon",
            "chardev=mon0",
            "-device",
            &format!("isa-debug-exit,iobase={exit_port:#04x},iosize=0x01"),
            "-plugin",
            &format!(
                "{},port={ready_for_exit_signal_port},fd={}",
                plugin
                    .to_str()
                    .ok_or_else(|| anyhow!("invalid plugin: '{plugin:?}'"))?,
                vm_close_on_ready.as_raw_fd()
            ),
        ])
        .args(qemu_arguments)
        .spawn()
        .with_context(|| format!("spawn command: '{program:?}' ..."));
    drop(vm_close_on_ready); // placate clippy
    result
}

/// # Errors
///
/// Will return `Err` if `status` is neither `Exited` nor `Signaled` or if
/// `Signaled` contains non-fatal `signal`.
pub fn mimic_wait_status(status: WaitStatus) -> Result<Infallible> {
    info!("mimic wait status: {} {status:?}", process::id());

    match status {
        WaitStatus::Exited(_process_id, code) => process::exit(code),
        WaitStatus::Signaled(_process_id, signal, _dumped_core) => {
            #[allow(clippy::as_conversions)]
            emulate_default_handler(signal as i32).context("emulate default handler")?;
            bail!("non-fatal signal: {signal}");
        }
        status => bail!("wait status unexpected: {status:?}"),
    }
}

impl QemuWrapper {
    const SIGNALS: [c_int; 8] = [
        SIGHUP, SIGINT, SIGQUIT, SIGTERM, // powerdown grandchild
        SIGCHLD, // required, handled separately by parent and child
        SIGTSTP, // handled separately by parent and child
        SIGTTIN, SIGTTOU, // handled separately by child
    ];

    #[allow(clippy::too_many_lines)] // FIXME:
    fn run_child(&self, arguments: Arguments, mut signals: Signals) -> Result<WaitStatus> {
        const EXIT_CODE_SUCCESS: i32 = 0;
        const EXIT_CODE_FAILURE: i32 = 1;
        const EXIT_CODE_QEMU_POWERDOWN: i32 = EXIT_CODE_SUCCESS;

        info!("run child: {}", process::id());

        // setup that must be done before spawning grandchild
        setpgid(
            Pid::from_raw(0 /* this process id */),
            Pid::from_raw(0 /* this process id as group id */),
        )
        .context("setpgid")?;
        let listener = MonitorListener::new().context("new monitor socket")?;
        let mut terminal = TerminalGuard::new().context("new terminal guard")?;

        let (vm_close_on_ready_reader, vm_close_on_ready_writer) = pipe().context("pipe")?;
        // set FD_CLOEXEC so child doesn't inherit file descriptor
        fcntl(
            &vm_close_on_ready_reader,
            FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC),
        )
        .map(drop)
        .context("fcntl set fd")?;

        let Arguments {
            child_arguments,
            exit_code,
        } = arguments;

        // spawn grandchild

        let mut grandchild =
            spawn_qemu_child(child_arguments, listener.path(), vm_close_on_ready_writer)
                .context("spawn qemu grandchild")?;

        // handle events
        let mut listener = Some(listener);
        let mut monitor = Option::<UnixStream>::None;
        let mut vm_close_on_ready = Some(vm_close_on_ready_reader);
        let mut powerdown_grandchild = false;
        loop {
            // if powerdown requested and grandchild's VM is ready for powerdown message
            if powerdown_grandchild && vm_close_on_ready.is_none() {
                // and grandchild has connected to monitor
                if let Some(monitor) = &mut monitor {
                    // request powerdown
                    monitor
                        .write_all(b"system_powerdown\n")
                        .context("write system powerdown")?;
                }
            }

            let mut fds = FdSet::new();
            if let Some(listener) = &listener {
                fds.insert(listener.as_fd());
            }
            if let Some(vm_close_on_ready) = &vm_close_on_ready {
                fds.insert(vm_close_on_ready.as_fd());
            }

            // wait for event
            match pselect(
                None,
                Some(&mut fds),
                None,
                None,
                None,
                Some(self.sigmask.previous()),
            ) {
                // monitor connection
                Ok(mut fds_length) => {
                    let contains_vm_close_on_ready = vm_close_on_ready
                        .as_ref()
                        .is_some_and(|fd| fds.contains(fd.as_fd()));

                    if listener.as_ref().is_some_and(|fd| fds.contains(fd.as_fd())) {
                        fds_length -= 1;

                        let previous = monitor.replace(
                            listener
                                .take()
                                .expect("take listener")
                                .accept()
                                .context("listener accept monitor")?,
                        );
                        assert!(previous.is_none(), "monitor already accepted");
                    }

                    if contains_vm_close_on_ready {
                        fds_length -= 1;

                        info!("QEMU has signaled that VM is ready");

                        drop(vm_close_on_ready.take());
                    }

                    assert_eq!(fds_length, 0, "unexpected fds length");
                }

                // signal(s)
                Err(Errno::EINTR) => {
                    for siginfo in signals.pending() {
                        match siginfo.si_signo {
                            // grandchild changed state
                            SIGCHLD => {
                                // grandchild may be alive but have just stopped/continued
                                match grandchild.try_wait().context("try wait")? {
                                    None => (), // carry on
                                    Some(status) => {
                                        return WaitStatus::from_raw(
                                            Pid::from_raw(
                                                grandchild
                                                    .id()
                                                    .try_into()
                                                    .expect("grandchid process id try into pid"),
                                            ),
                                            status.into_raw(),
                                        )
                                        .map(|status| match status {
                                            WaitStatus::Exited(
                                                process_id,
                                                EXIT_CODE_QEMU_POWERDOWN,
                                            ) => WaitStatus::Exited(process_id, EXIT_CODE_FAILURE),
                                            WaitStatus::Exited(process_id, code)
                                                if exit_code == code =>
                                            {
                                                WaitStatus::Exited(process_id, EXIT_CODE_SUCCESS)
                                            }
                                            status => status,
                                        })
                                        .context("grandchild wait status from raw");
                                    }
                                }
                            }

                            // ctr+z (paused)
                            signal @ SIGTSTP => {
                                if parent_id()
                                // SAFETY: field should always be present
                                == unsafe { siginfo.si_pid() }
                                    .try_into()
                                    .expect("siginfo pid try into")
                                {
                                    let _reset = terminal
                                        .stop_child_and_reset_guard(&mut grandchild)
                                        .context("terminal stop grandchild and reset guard")?;

                                    emulate_default_handler(signal)
                                        .context("emulate default terminal stop handler")?;
                                } else {
                                    terminal
                                        .signal_foreground_process_group(
                                            Signal::try_from(signal).context("signal try from")?,
                                        )
                                        .context("signal foreground process group")?;
                                }
                            }

                            // read/write to terminal from background
                            SIGTTIN | SIGTTOU => (), // carry on, don't stop process

                            // powerdown or kill grandchild
                            _signal => {
                                // if subsequent request
                                if powerdown_grandchild {
                                    grandchild.kill().context("kill grandchild")?;
                                } else {
                                    powerdown_grandchild = true;
                                }
                            }
                        }
                    }
                }

                Err(Errno::EAGAIN | Errno::ENOMEM) => (), // carry on
                Err(error) => return Err(Error::new(error).context("pselect")),
            }
        }
    }

    /// # Errors
    ///
    /// Will return `Err` if signals cannot be blocked.
    ///
    /// # Panics
    ///
    /// Will panic on internal, programming error.
    pub fn new() -> Result<Self> {
        info!("new: {}", process::id());

        // block signals before spawning child
        let sigmask = SigmaskGuard::new(
            Self::SIGNALS
                .into_iter()
                .map(|signal| Signal::try_from(signal).expect("signal try from"))
                .collect(),
        )
        .context("new sigmask guard")?;

        Ok(Self { sigmask })
    }

    /// # Errors
    ///
    /// Will return `Err` if QEMU cannot be run to completion.
    pub fn run(self, arguments: impl IntoIterator<Item = impl AsRef<str>>) -> Result<WaitStatus> {
        info!("run: {}", process::id());
        let arguments = Arguments::parse(arguments).context("parse arguments")?;

        let mut signals = Signals::new(Self::SIGNALS).context("new signals")?;

        // SAFETY: safe in a singly-threaded process
        let child = match unsafe { fork() }.context("fork")? {
            ForkResult::Parent { child } => child,
            ForkResult::Child => bail!(
                mimic_wait_status(self.run_child(arguments, signals).context("run child")?)
                    .context("child mimic wait status")
                    .expect_err("mimic wait status succeeded unexpectedly")
            ),
        };
        let Self { sigmask } = self;
        drop(sigmask); // unblock

        for siginfo in &mut signals {
            match siginfo.si_signo {
                // child changed state
                SIGCHLD => {
                    // child may be alive but have just stopped/continued
                    match waitpid(None, Some(WaitPidFlag::WNOHANG)) {
                        Ok(WaitStatus::StillAlive) => (), // child still alive
                        Ok(status) => return Ok(status),
                        Err(error) => {
                            return Err(Error::from(error).context("wait errno unexpected"));
                        }
                    }
                }

                // ctr+z (paused)
                signal @ SIGTSTP => {
                    let _stop =
                        StopGuard::new(child, Signal::try_from(signal).context("signal try from")?)
                            .context("stop guard")?;
                    emulate_default_handler(signal)
                        .context("emulate default terminal stop handler")?;
                }

                // forward to child
                signal => kill(child, Signal::try_from(signal).context("signal try from")?)
                    .context("kill child")?,
            }
        }
        unreachable!("signals iterator ended");
    }
}
