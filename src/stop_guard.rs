use anyhow::{Context, Result};
use nix::{
    sys::signal::{kill, Signal},
    unistd::Pid,
};
use std::{any::type_name, process::Child};

pub struct StopGuard<'child> {
    child: &'child mut Child, // mutable reference to ensure exclusive ownership of child
}

impl<'child> StopGuard<'child> {
    pub fn new(child: &'child mut Child) -> Result<StopGuard<'child>> {
        let this = Self { child };
        this.signal(Signal::SIGSTOP)?;
        Ok(this)
    }

    fn signal(&self, signal: Signal) -> Result<()> {
        kill(
            Pid::from_raw(self.child.id().try_into().expect("try into pid")),
            signal,
        )
        .context("signal child")
    }
}

impl Drop for StopGuard<'_> {
    fn drop(&mut self) {
        match self.signal(Signal::SIGCONT).context("signal continue") {
            Ok(()) => (),
            Err(error) => eprintln!("Error: {} {error:?}", type_name::<Self>()),
        }
    }
}
