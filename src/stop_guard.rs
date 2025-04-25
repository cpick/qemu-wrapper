use anyhow::{Context, Result};
use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use std::{any::type_name, process::Child};

pub struct StopGuard {
    id: Pid, // positive process id or negative process group id per kill(2)
}

impl StopGuard {
    pub fn new(id: Pid, stop: Signal) -> Result<StopGuard> {
        let this = Self { id };
        this.signal(stop).context("signal stop")?;
        Ok(this)
    }

    fn signal(&self, signal: Signal) -> Result<()> {
        kill(self.id, signal).context("signal")
    }
}

impl Drop for StopGuard {
    fn drop(&mut self) {
        match self.signal(Signal::SIGCONT).context("signal continue") {
            Ok(()) => (),
            Err(error) => eprintln!("Error: {} {error:?}", type_name::<Self>()),
        }
    }
}

pub struct ChildStopGuard<'child> {
    _stop: StopGuard,
    _child: &'child mut Child, // mutable reference to ensure exclusive ownership of child
}

impl<'child> ChildStopGuard<'child> {
    pub fn new(child: &'child mut Child) -> Result<ChildStopGuard<'child>> {
        Ok(Self {
            _stop: StopGuard::new(
                Pid::from_raw(child.id().try_into().expect("try into pid")),
                Signal::SIGSTOP, // SIGTSTP is blocked, force the stop
            )
            .context("stop guard")?,
            _child: child,
        })
    }
}
