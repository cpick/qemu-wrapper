use crate::sigmask_guard::SigmaskGuard;
use anyhow::{Context, Error, Result};
use nix::errno::Errno;
use nix::sys::signal::Signal;
use nix::sys::termios::{tcgetattr, tcsetattr, LocalFlags, SetArg, Termios};
use nix::unistd::{getpgrp, tcgetpgrp, tcsetpgrp, Pid};
use std::io::stdin;
use std::os::fd::{AsFd as _, AsRawFd as _, OwnedFd};

struct State {
    terminal: OwnedFd,
    foreground_process_group: Pid,
    termios: Termios,
}

pub struct TerminalGuard {
    state: Option<State>,
}

impl TerminalGuard {
    pub fn new() -> Result<Self> {
        let terminal = stdin()
            .as_fd()
            .try_clone_to_owned()
            .context("stdin fd try clone to owned")?;
        let termios = match tcgetattr(terminal.as_raw_fd()) {
            Ok(termios) => termios,
            Err(Errno::ENODEV) => return Ok(Self { state: None }),
            Err(error) => return Err(Error::new(error).context("tcgetattr")),
        };

        // mopve to foreground
        let process_group = getpgrp();
        let foreground_process_group = tcgetpgrp(terminal.as_raw_fd()).context("tcgetpgrp")?;
        if process_group != foreground_process_group {
            let _sigmask = SigmaskGuard::new(&[Signal::SIGTTOU].into_iter().collect())
                .context("new sigmask guard")?;
            tcsetpgrp(terminal.as_raw_fd(), process_group).context("tcsetpgrp")?;
        }

        Ok(Self {
            state: Some(State {
                terminal,
                foreground_process_group,
                termios,
            }),
        })
    }

    pub fn reenable_signals(&self) -> Result<()> {
        if let Some(State {
            terminal, termios, ..
        }) = &self.state
        {
            let isig = termios.local_flags & LocalFlags::ISIG;
            let mut termios = tcgetattr(terminal.as_raw_fd()).context("tcgetattr")?;
            termios.local_flags |= isig;
            tcsetattr(terminal.as_raw_fd(), SetArg::TCSANOW, &termios).context("tcsetattr")?;
        }
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        if let Some(State {
            terminal,
            foreground_process_group,
            termios,
        }) = &self.state
        {
            tcsetattr(terminal.as_raw_fd(), SetArg::TCSANOW, termios).context("tcsetattr")?;
            tcsetpgrp(terminal.as_raw_fd(), *foreground_process_group).context("tcsetpgrp")?;
        }
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        match self.reset().context("reset") {
            Ok(()) => (),
            Err(error) => eprintln!("Error: {error:?}"),
        }
    }
}
