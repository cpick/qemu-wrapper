use crate::sigmask_guard::SigmaskGuard;
use anyhow::{Context, Error, Result};
use nix::errno::Errno;
use nix::sys::signal::{SigSet, Signal};
use nix::sys::termios::{cfmakeraw, tcgetattr, tcsetattr, LocalFlags, SetArg, Termios};
use std::io::stdin;
use std::os::fd::{AsRawFd as _, RawFd};

pub struct RawGuard {
    termios: Option<Termios>,
}

impl RawGuard {
    pub fn new() -> Result<Self> {
        let stdin = stdin().as_raw_fd();
        let termios = match tcgetattr(stdin) {
            Ok(termios) => termios,
            Err(Errno::ENODEV) => return Ok(Self { termios: None }),
            Err(error) => return Err(Error::new(error).context("tcgetattr")),
        };

        {
            let mut termios_raw = termios.clone();
            cfmakeraw(&mut termios_raw);
            termios_raw.local_flags |= termios.local_flags & LocalFlags::ISIG;
            Self::set_tty_attributes(stdin, &termios_raw).context("set_tty_attributes")?;
        }

        Ok(Self {
            termios: Some(termios),
        })
    }

    fn set_tty_attributes(fd: RawFd, termios: &Termios) -> Result<()> {
        let _sigmask = {
            let mut handled = SigSet::empty();
            handled.add(Signal::SIGTTOU);
            SigmaskGuard::new(&handled).context("new sigmask guard")?
        };

        tcsetattr(fd, SetArg::TCSANOW, termios).context("tcsetattr")?;
        Ok(())
    }
}

impl Drop for RawGuard {
    fn drop(&mut self) {
        if let Some(termios) = &self.termios {
            match Self::set_tty_attributes(stdin().as_raw_fd(), termios)
                .context("set_tty_attributes")
            {
                Ok(()) => (),
                Err(error) => eprintln!("Error: {error:?}"),
            }
        }
    }
}
