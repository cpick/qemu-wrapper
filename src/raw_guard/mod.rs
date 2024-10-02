use anyhow::{Context, Error, Result};
use nix::errno::Errno;
use nix::sys::termios::{cfmakeraw, tcgetattr, tcsetattr, LocalFlags, SetArg, Termios};
use std::io::stdin;
use std::os::fd::AsRawFd as _;

pub struct RawGuard {
    termios: Option<Termios>,
}

impl RawGuard {
    #[allow(dead_code)]
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
            tcsetattr(stdin, SetArg::TCSANOW, &termios_raw).context("tcsetattr")?;
        }

        Ok(Self {
            termios: Some(termios),
        })
    }
}

impl Drop for RawGuard {
    fn drop(&mut self) {
        if let Some(termios) = &self.termios {
            let _ = tcsetattr(stdin().as_raw_fd(), SetArg::TCSANOW, termios);
        }
    }
}
