use anyhow::{Context, Result};
use nix::sys::signal::{SigSet, SigmaskHow};
use std::io::Result as IoResult;

pub struct SigmaskGuard {
    previous: SigSet,
}

impl SigmaskGuard {
    pub fn new(block: &SigSet) -> Result<Self> {
        Ok(Self {
            previous: block
                .thread_swap_mask(SigmaskHow::SIG_BLOCK)
                .context("thread swap mask block")?,
        })
    }

    pub fn reset(&mut self) -> IoResult<()> {
        Ok(self.previous.thread_set_mask()?)
    }

    pub fn previous(&self) -> &SigSet {
        &self.previous
    }
}

impl Drop for SigmaskGuard {
    fn drop(&mut self) {
        match self.reset().context("reset") {
            Ok(()) => (),
            Err(error) => eprintln!("Error: {error:?}"),
        }
    }
}
