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

mod monitor_listener;
mod sigmask_guard;
mod stop_guard;
mod terminal_guard;

use anyhow::{anyhow, bail, Context, Error, Result};
use monitor_listener::MonitorListener;
use nix::{
    errno::Errno,
    sys::{
        select::{pselect, FdSet},
        signal::{kill, Signal},
        wait::{waitpid, WaitPidFlag, WaitStatus},
    },
    unistd::{fork, setpgid, ForkResult, Pid},
};
use sigmask_guard::SigmaskGuard;
use signal_hook::{
    consts::{SIGCHLD, SIGHUP, SIGINT, SIGQUIT, SIGTERM, SIGTSTP, SIGTTIN, SIGTTOU},
    iterator::{exfiltrator::WithRawSiginfo, SignalsInfo},
    low_level::emulate_default_handler,
};
use std::{
    env::args,
    io::Write as _,
    os::{
        fd::AsFd,
        unix::process::{parent_id, ExitStatusExt as _},
    },
    process::{Child, Command, ExitCode},
};
use stop_guard::StopGuard;
use terminal_guard::TerminalGuard;

type Signals = SignalsInfo<WithRawSiginfo>;

fn spawn_qemu_child(
    architecture: &str,
    arguments: impl IntoIterator<Item = String>,
    listener_path: &str,
) -> Result<Child> {
    let program = format!("qemu-system-{architecture}");
    Command::new(&program)
        .args(
            [
                "-chardev".to_owned(),
                format!("socket,id=mon0,path={listener_path},server=off"),
                "-mon".to_owned(),
                "chardev=mon0".to_owned(),
            ]
            .into_iter()
            .chain(arguments),
        )
        .spawn()
        .with_context(|| format!("spawn command: '{program}' ..."))
}

fn run_child(sigmask: SigmaskGuard, mut signals: Signals) -> Result<WaitStatus> {
    // setup that must be done before spawning grandchild
    setpgid(
        Pid::from_raw(0 /* this process id */),
        Pid::from_raw(0 /* this process id as group id */),
    )
    .context("setpgid")?;
    let listener = MonitorListener::new().context("new monitor socket")?;
    let mut terminal = TerminalGuard::new().context("new terminal guard")?;

    // spawn grandchild
    let mut arguments = args().skip(1 /* argv[0] */);
    let architecture = arguments
        .next()
        .ok_or_else(|| anyhow!("missing <guest_architecture> argument"))?;
    let mut grandchild = spawn_qemu_child(&architecture, arguments, listener.path())
        .context("spawn qemu grandchild")?;

    // handle events
    let mut listener = Some(listener);
    let mut monitor = None;
    loop {
        let mut fds = FdSet::new();
        if let Some(listener) = &listener {
            fds.insert(listener.as_fd());
        }

        // wait for event
        match pselect(
            None,
            Some(&mut fds),
            None,
            None,
            None,
            Some(sigmask.previous()),
        ) {
            // monitor connection
            Ok(fds_length) => {
                assert_eq!(fds_length, 1, "unexpected fds length");
                assert!(fds.contains(listener.as_ref().expect("listener as ref").as_fd()));

                let previous = monitor.replace(
                    listener
                        .take()
                        .expect("take listener")
                        .accept()
                        .context("listener accept monitor")?,
                );
                assert!(previous.is_none(), "monitor already accepted");
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
                                    return Ok(WaitStatus::from_raw(
                                        Pid::from_raw(
                                            grandchild
                                                .id()
                                                .try_into()
                                                .expect("grandchid process id try into pid"),
                                        ),
                                        status.into_raw(),
                                    )
                                    .context("wait status from raw")?)
                                }
                            }
                        }

                        // ctr+z (paused)
                        signal @ SIGTSTP => {
                            if parent_id()
                                == siginfo.si_pid.try_into().expect("siginfo pid try into")
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
                        _signal => match &mut monitor {
                            Some(monitor) => monitor
                                .write_all(b"system_powerdown\n")
                                .context("write system powerdown")?,
                            None => grandchild.kill().context("kill grandchild")?,
                        },
                    }
                }
            }

            Err(Errno::EAGAIN | Errno::ENOMEM) => (), // carry on
            Err(error) => return Err(Error::new(error).context("pselect")),
        }
    }
}

fn run() -> Result<WaitStatus> {
    // block signals before spawning child
    let signals = [
        SIGHUP, SIGINT, SIGQUIT, SIGTERM, // powerdown grandchild
        SIGCHLD, // required, handled separately by parent and child
        SIGTSTP, // handled separately by parent and child
        SIGTTIN, SIGTTOU, // handled separately by child
    ];
    let sigmask = SigmaskGuard::new(
        signals
            .into_iter()
            .map(|signal| Signal::try_from(signal).expect("signal try from"))
            .collect(),
    )
    .context("new sigmask guard")?;
    let mut signals = Signals::new(signals).context("new signals")?;

    // SAFETY: safe in a singly-threaded process
    let child = match unsafe { fork() }.context("fork")? {
        ForkResult::Parent { child } => child,
        ForkResult::Child => return run_child(sigmask, signals).context("run child"),
    };
    drop(sigmask); // unblock

    for siginfo in &mut signals {
        match siginfo.si_signo {
            // child changed state
            SIGCHLD => {
                // child may be alive but have just stopped/continued
                match waitpid(None, Some(WaitPidFlag::WNOHANG)) {
                    Ok(WaitStatus::StillAlive) => (), // child still alive
                    Ok(status) => return Ok(status),
                    Err(error) => return Err(Error::from(error).context("wait errno unexpected")),
                }
            }

            // ctr+z (paused)
            signal @ SIGTSTP => {
                let _stop =
                    StopGuard::new(child, Signal::try_from(signal).context("signal try from")?)
                        .context("stop guard")?;
                emulate_default_handler(signal).context("emulate default terminal stop handler")?;
            }

            // forward to child
            signal => kill(child, Signal::try_from(signal).context("signal try from")?)
                .context("kill child")?,
        }
    }
    unreachable!("signals iterator ended");
}

fn main() -> Result<ExitCode> {
    match run().context("run")? {
        WaitStatus::Exited(_process_id, code) => {
            return Ok(u8::try_from(code).expect("exit code try from u8").into());
        }
        WaitStatus::Signaled(_process_id, signal, _dumped_core) => {
            emulate_default_handler((signal as i32).try_into().expect("signal try into"))
                .context("emulate default fatal handler")?;
            panic!("non-fatal signal: {signal}");
        }
        status => bail!("wait status unexpected: {status:?}"),
    }
}
