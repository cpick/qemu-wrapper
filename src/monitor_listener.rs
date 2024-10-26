use anyhow::{Context, Error, Result};
use std::{
    any::type_name,
    io::ErrorKind,
    os::{
        fd::{AsFd, BorrowedFd},
        unix::net::{UnixListener, UnixStream},
    },
    process::id,
};

pub struct MonitorListener {
    path: String,
    listener: UnixListener,
}

impl MonitorListener {
    pub fn new() -> Result<Self> {
        let path = format!("monitor-{}.sock", id());

        match std::fs::remove_file(&path) {
            Ok(()) => {}                                            // carry on
            Err(error) if error.kind() == ErrorKind::NotFound => {} // carry on
            Err(error) => {
                return Err(Error::new(error).context(format!("remove socket file '{path}'")))
            }
        }

        let listener = UnixListener::bind(&path).context("bind unix listener")?;

        Ok(Self { path, listener })
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn accept(self) -> Result<UnixStream> {
        let (socket, _address) = self.listener.accept().context("listener accept")?;
        Ok(socket)
    }
}

impl Drop for MonitorListener {
    fn drop(&mut self) {
        let path = self.path();
        if let Err(error) = std::fs::remove_file(path) {
            let error = Error::new(error).context(format!("remove socket file '{path}'"));
            eprintln!("Error: {} {error:?}", type_name::<Self>());
        }
    }
}

impl AsFd for MonitorListener {
    fn as_fd(&self) -> BorrowedFd {
        self.listener.as_fd()
    }
}
