use anyhow::{Context, Result};
use nix::sys::signal::{SigSet, SigmaskHow};
use std::io::Result as IoResult;

pub struct SigmaskGuard {
    previous: Option<SigSet>,
}

impl SigmaskGuard {
    pub fn new(block: &SigSet) -> Result<Self> {
        Ok(Self {
            previous: Some(
                block
                    .thread_swap_mask(SigmaskHow::SIG_BLOCK)
                    .context("thread swap mask block")?,
            ),
        })
    }

    pub fn unblock(&mut self) -> IoResult<()> {
        if let Some(previous) = self.previous.take() {
            previous.thread_set_mask()?;
        }
        Ok(())
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
