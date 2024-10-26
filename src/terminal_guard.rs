use crate::sigmask_guard::SigmaskGuard;
use anyhow::{Context, Error, Result};
use nix::{
    errno::Errno,
    sys::{
        signal::Signal,
        termios::{tcgetattr, tcsetattr, SetArg, Termios},
    },
    unistd::{getpgrp, tcgetpgrp, tcsetpgrp, Pid},
};
use std::{
    any::type_name,
    io::stdin,
    os::fd::{AsFd as _, OwnedFd},
};

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
        let termios = match tcgetattr(&terminal) {
            Ok(termios) => termios,
            Err(Errno::ENODEV) => return Ok(Self { state: None }),
            Err(error) => return Err(Error::new(error).context("tcgetattr")),
        };

        // mopve to foreground
        let process_group = getpgrp();
        let foreground_process_group = tcgetpgrp(&terminal).context("tcgetpgrp")?;
        if process_group != foreground_process_group {
            let _sigmask = SigmaskGuard::new([Signal::SIGTTOU].into_iter().collect())
                .context("new sigmask guard")?;
            tcsetpgrp(&terminal, process_group).context("tcsetpgrp")?;
        }

        Ok(Self {
            state: Some(State {
                terminal,
                foreground_process_group,
                termios,
            }),
        })
    }

    fn reset(&mut self) -> Result<()> {
        if let Some(State {
            terminal,
            foreground_process_group,
            termios,
        }) = &self.state
        {
            tcsetattr(terminal, SetArg::TCSANOW, termios).context("tcsetattr")?;
            tcsetpgrp(terminal, *foreground_process_group).context("tcsetpgrp")?;
        }
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        match self.reset().context("reset") {
            Ok(()) => (),
            Err(error) => eprintln!("Error: {} {error:?}", type_name::<Self>()),
        }
    }
}
