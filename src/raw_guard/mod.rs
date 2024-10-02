use nix::sys::termios::{cfmakeraw, tcgetattr, tcsetattr, LocalFlags, SetArg, Termios};
use std::io::stdin;
use std::os::fd::AsRawFd as _;

pub struct RawGuard {
    termios: Termios,
}

impl RawGuard {
    #[allow(dead_code)]
    pub fn new() -> Self {
        let stdin = stdin().as_raw_fd();
        let termios = tcgetattr(stdin).unwrap();
        let mut termios_raw = termios.clone();
        cfmakeraw(&mut termios_raw);
        termios_raw.local_flags |= termios.local_flags & LocalFlags::ISIG;
        tcsetattr(stdin, SetArg::TCSANOW, &termios_raw).unwrap();
        Self { termios }
    }
}

impl Drop for RawGuard {
    fn drop(&mut self) {
        let stdin = stdin().as_raw_fd();
        let _ = tcsetattr(stdin, SetArg::TCSANOW, &self.termios);
    }
}
