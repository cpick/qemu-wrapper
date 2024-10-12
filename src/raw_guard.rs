use crate::sigmask_guard::SigmaskGuard;
use anyhow::{Context, Error, Result};
use nix::errno::Errno;
use nix::sys::signal::{SigSet, Signal};
use nix::sys::termios::{cfmakeraw, tcgetattr, tcsetattr, LocalFlags, SetArg, Termios};
use nix::unistd::{getpgrp, tcgetpgrp, tcsetpgrp, Pid};
use std::io::stdin;
use std::os::fd::AsRawFd as _;

struct State {
    foreground_process_group: Pid,
    termios: Termios,
}

pub struct RawGuard {
    state: Option<State>,
}

impl RawGuard {
    pub fn new() -> Result<Self> {
        let stdin = stdin().as_raw_fd();
        let termios = match tcgetattr(stdin) {
            Ok(termios) => termios,
            Err(Errno::ENODEV) => return Ok(Self { state: None }),
            Err(error) => return Err(Error::new(error).context("tcgetattr")),
        };

        // mopve to foreground
        let process_group = getpgrp();
        let foreground_process_group = tcgetpgrp(stdin).context("tcgetpgrp")?;
        if process_group != foreground_process_group {
            let _sigmask = {
                let mut handled = SigSet::empty();
                handled.add(Signal::SIGTTOU);
                SigmaskGuard::new(&handled).context("new sigmask guard")?
            };

            tcsetpgrp(stdin, process_group).context("tcsetpgrp")?;
        }

        // set raw terminal, but keep detecting and sending signals
        {
            let mut termios_raw = termios.clone();
            cfmakeraw(&mut termios_raw);
            termios_raw.local_flags |= termios.local_flags & LocalFlags::ISIG;
            tcsetattr(stdin, SetArg::TCSANOW, &termios_raw).context("tcsetattr")?;
        }

        Ok(Self {
            state: Some(State {
                foreground_process_group,
                termios,
            }),
        })
    }

    fn reset(&mut self) -> Result<()> {
        if let Some(State {
            foreground_process_group,
            termios,
        }) = &self.state
        {
            let stdin = stdin().as_raw_fd();
            tcsetattr(stdin, SetArg::TCSANOW, termios).context("tcsetattr")?;
            tcsetpgrp(stdin, *foreground_process_group).context("tcsetpgrp")?;
        }
        Ok(())
    }
}

impl Drop for RawGuard {
    fn drop(&mut self) {
        match self.reset().context("reset") {
            Ok(()) => (),
            Err(error) => eprintln!("Error: {error:?}"),
        }
    }
}
