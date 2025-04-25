use crate::sigmask_guard::SigmaskGuard;
use crate::stop_guard::ChildStopGuard;
use anyhow::{Context, Error, Result};
use nix::sys::signal::killpg;
use nix::{
    errno::Errno,
    sys::{
        signal::Signal,
        termios::{SetArg, Termios, tcgetattr, tcsetattr},
    },
    unistd::{Pid, getpgrp, tcgetpgrp, tcsetpgrp},
};
use std::{
    any::type_name,
    io::stdin,
    os::fd::{AsFd as _, OwnedFd},
    process::Child,
};

struct TerminalState {
    terminal: OwnedFd,
    foreground_process_group: Pid,
    termios: Termios,
}

pub struct TerminalGuard {
    state: Option<TerminalState>,
}

pub struct ResetGuard<'terminal, 'child> {
    terminal: &'terminal mut TerminalGuard, // mutable reference to ensure exclusive ownership
    termios: Option<Termios>,
    _child_stop: ChildStopGuard<'child>,
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
        let foreground_process_group = tcgetpgrp(&terminal).context("tcgetpgrp")?;

        let mut this = Self {
            state: Some(TerminalState {
                terminal,
                foreground_process_group,
                termios,
            }),
        };

        this.move_to_foreground().context("move to foreground")?;

        Ok(this)
    }

    fn move_to_foreground(&mut self) -> Result<()> {
        if let Some(TerminalState {
            terminal,
            foreground_process_group,
            ..
        }) = &self.state
        {
            let process_group = getpgrp();
            if process_group != *foreground_process_group {
                let _sigmask = SigmaskGuard::new([Signal::SIGTTOU].into_iter().collect())
                    .context("new sigmask guard")?;
                tcsetpgrp(terminal, process_group).context("tcsetpgrp")?;
            }
        }
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        if let Some(TerminalState {
            terminal,
            foreground_process_group,
            termios,
        }) = &self.state
        {
            if tcgetpgrp(terminal).context("tcgetpgrp")? == getpgrp() {
                tcsetattr(terminal, SetArg::TCSANOW, termios).context("tcsetattr")?;
                tcsetpgrp(terminal, *foreground_process_group).context("tcsetpgrp")?;
            }
        }
        Ok(())
    }

    pub fn stop_child_and_reset_guard<'terminal, 'child>(
        &'terminal mut self,
        child: &'child mut Child,
    ) -> Result<ResetGuard<'terminal, 'child>> {
        ResetGuard::new(self, child)
    }

    pub fn signal_foreground_process_group(&mut self, signal: Signal) -> Result<()> {
        if let Some(TerminalState {
            foreground_process_group,
            ..
        }) = &self.state
        {
            killpg(*foreground_process_group, signal).context("killpg")?;
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

impl<'terminal, 'child> ResetGuard<'terminal, 'child> {
    fn new(
        terminal: &'terminal mut TerminalGuard,
        child: &'child mut Child,
    ) -> Result<ResetGuard<'terminal, 'child>> {
        let stop = ChildStopGuard::new(child).context("new child stop guard")?;

        if terminal.state.is_none() {
            // this check and return allows the state.expect() calls below
            return Ok(ResetGuard {
                terminal,
                termios: None,
                _child_stop: stop,
            });
        }

        let termios = tcgetattr(
            &terminal
                .state
                .as_ref()
                .expect("terminal state as ref") // see state.is_none() above
                .terminal,
        )
        .context("tcgetattr")?;
        terminal.reset().context("terminal reset")?;

        Ok(ResetGuard {
            terminal,
            termios: Some(termios),
            _child_stop: stop,
        })
    }

    fn reapply(&mut self) -> Result<()> {
        self.terminal
            .move_to_foreground()
            .context("move to foreground")?;
        if let Some(termios) = &self.termios {
            tcsetattr(
                &self
                    .terminal
                    .state
                    .as_ref()
                    .expect("terminal state as ref")
                    .terminal,
                SetArg::TCSANOW,
                termios,
            )
            .context("tcsetattr")?;
        }
        Ok(())
    }
}

impl Drop for ResetGuard<'_, '_> {
    fn drop(&mut self) {
        match self.reapply().context("reapply") {
            Ok(()) => (),
            Err(error) => eprintln!("Error: {} {error:?}", type_name::<Self>()),
        }
    }
}
