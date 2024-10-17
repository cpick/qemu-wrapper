mod monitor_listener;
mod sigmask_guard;
mod terminal_guard;

use anyhow::{Context, Error, Result};
use monitor_listener::MonitorListener;
use nix::errno::Errno;
use nix::sys::{
    select::{pselect, FdSet},
    signal::{SigSet, SigmaskHow, Signal},
};
use nix::unistd::{getpgrp, setpgid, Pid};
use signal_hook::iterator::Signals;
use std::convert::Infallible;
use std::env::args;
use std::io::Write as _;
use std::os::fd::AsRawFd as _;
use std::os::unix::process::{CommandExt, ExitStatusExt as _};
use std::process::{exit, Child, Command, ExitStatus};
use terminal_guard::TerminalGuard;

fn become_process_group_leader() -> Result<()> {
    // prevent EPERM failure if this is already session (and thus process group) leader
    if Pid::this() != getpgrp() {
        setpgid(
            Pid::from_raw(0 /* this process id */),
            Pid::from_raw(0 /* this process id as group id */),
        )
        .context("setpgid")?;
    }
    Ok(())
}

fn spawn_qemu_child(
    arguments: impl IntoIterator<Item = String>,
    listener_path: &str,
    sigmask: SigSet,
) -> Result<Child> {
    let mut command = Command::new("qemu-system-x86_64");
    command.args(
        [
            "-chardev".to_owned(),
            format!("socket,id=mon0,path={},server=off", listener_path),
            "-mon".to_owned(),
            "chardev=mon0".to_owned(),
        ]
        .into_iter()
        .chain(arguments),
    );
    let pre_exec = move || Ok(sigmask.thread_set_mask()?);
    unsafe {
        command.pre_exec(pre_exec);
    }

    Ok(command.spawn().context("spawn command")?)
}

fn run() -> Result<ExitStatus> {
    // setup that must be done before spawning child

    become_process_group_leader().context("become process group leader")?;
    let listener = MonitorListener::new().context("monitor socket")?;
    let _terminal = TerminalGuard::new().context("terminal guard")?;

    let signals_to_block_in_parent_and_child = [Signal::SIGHUP, Signal::SIGINT, Signal::SIGTERM]
        .into_iter()
        .collect::<SigSet>();
    let original_sigmask = signals_to_block_in_parent_and_child
        .thread_swap_mask(SigmaskHow::SIG_BLOCK)
        .context("thread swap mask block parent and child")?;

    let signals_to_block_in_parent = [Signal::SIGQUIT, Signal::SIGCHLD]
        .into_iter()
        .collect::<SigSet>();
    let child_sigmask = signals_to_block_in_parent
        .thread_swap_mask(SigmaskHow::SIG_BLOCK)
        .context("thread swap mask block parent")?;

    let mut signals = Signals::new(
        signals_to_block_in_parent_and_child
            .into_iter()
            .chain(signals_to_block_in_parent.into_iter())
            .map(|signal| signal as i32),
    )
    .context("signals")?;

    // spawn child
    let mut child = spawn_qemu_child(args().skip(1 /* argv[0] */), listener.path(), child_sigmask)
        .context("spawn qemu child")?;

    // handle events
    let mut listener = Some(listener);
    let mut monitor = None;
    loop {
        let mut fds = FdSet::new();
        if let Some(listener) = &listener {
            fds.insert(listener.as_raw_fd());
        }

        // wait for event
        match pselect(
            None,
            Some(&mut fds),
            None,
            None,
            None,
            Some(&original_sigmask),
        ) {
            // monitor connection
            Ok(fds_length) => {
                assert_eq!(fds_length, 1, "unexpected fds length");
                let listener = listener.take().expect("take listener");
                assert!(fds.contains(listener.as_raw_fd()));

                let previous =
                    monitor.replace(listener.accept().context("listener accept monitor")?);
                assert!(previous.is_none(), "monitor already accepted");
            }

            // signal(s)
            Err(Errno::EINTR) => {
                for signal in signals.pending() {
                    match signal.try_into().expect("signal try into") {
                        // child changed state
                        Signal::SIGCHLD => {
                            match child.try_wait().context("try wait")? {
                                None => (), // carry on
                                Some(status) => return Ok(status),
                            }
                        }

                        // powerdown or kill child
                        signal if signals_to_block_in_parent_and_child.contains(signal) => {
                            match &mut monitor {
                                Some(monitor) => monitor
                                    .write_all(b"system_powerdown\n")
                                    .context("write system powerdown")?,
                                None => child.kill().context("kill child")?,
                            }
                        }

                        signal => panic!("unexpected signal: {signal}"),
                    }
                }
            }

            Err(Errno::EAGAIN | Errno::ENOMEM) => (), // carry on
            Err(error) => return Err(Error::new(error).context("pselect")),
        }
    }
}

fn main() -> Result<Infallible> {
    let status = run()?;
    exit(
        status
            .code()
            .unwrap_or_else(|| status.signal().unwrap_or(0) + 128),
    );
}
