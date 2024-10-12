use anyhow::{Context, Result};
use nix::sys::signal::{pthread_sigmask, SigSet, SigmaskHow};
use std::io::{Error as IoError, Result as IoResult};

pub struct SigmaskGuard {
    previous: Option<SigSet>,
}

impl SigmaskGuard {
    pub fn new(block: &SigSet) -> Result<Self> {
        let mut previous = SigSet::empty();
        pthread_sigmask(SigmaskHow::SIG_BLOCK, Some(block), Some(&mut previous))
            .context("block pthread_sigmask")?;
        Ok(Self {
            previous: Some(previous),
        })
    }

    pub fn unblock(&mut self) -> IoResult<()> {
        pthread_sigmask(SigmaskHow::SIG_SETMASK, self.previous.take().as_ref(), None)
            .map_err(|error| IoError::from_raw_os_error(error as i32))
    }
}

impl Drop for SigmaskGuard {
    fn drop(&mut self) {
        match self.unblock().context("unblock") {
            Ok(()) => (),
            Err(error) => eprintln!("Error: {error:?}"),
        }
    }
}
