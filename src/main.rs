use nix::sys::signal::{
    kill, raise, sigaction, SaFlags, SigAction, SigHandler, SigSet, Signal::SIGINT,
};
use std::io::{Read as _, Write as _};
use std::os::fd::{AsFd as _, AsRawFd as _, IntoRawFd as _};
use std::os::unix::net::UnixListener;
use std::sync::atomic::{AtomicI32, Ordering::Relaxed};

mod raw_guard;

static MONITOR_FD: AtomicI32 = AtomicI32::new(-1);

fn run(
    listener: UnixListener,
    child: &mut std::process::Child,
    pty: &mut pty_process::blocking::Pty,
) {
    let _raw = raw_guard::RawGuard::new();
    let mut buf = [0_u8; 4096];
    let pty_fd = pty.as_fd().as_raw_fd();
    let stdin_fd = std::io::stdin().as_raw_fd();
    let listener_fd = listener.as_raw_fd();

    loop {
        let mut set = nix::sys::select::FdSet::new();
        set.insert(pty_fd);
        set.insert(stdin_fd);
        set.insert(listener_fd);
        match nix::sys::select::select(None, Some(&mut set), None, None, None) {
            Ok(n) => {
                if n > 0 {
                    if set.contains(pty_fd) {
                        match pty.read(&mut buf) {
                            Ok(bytes) => {
                                let buf = &buf[..bytes];
                                let stdout = std::io::stdout();
                                let mut stdout = stdout.lock();
                                stdout.write_all(buf).unwrap();
                                stdout.flush().unwrap();
                            }
                            Err(e) => {
                                eprintln!("pty read failed: {e:?}");
                                break;
                            }
                        };
                    }
                    if set.contains(stdin_fd) {
                        match std::io::stdin().read(&mut buf) {
                            Ok(bytes) => {
                                let buf = &buf[..bytes];
                                pty.write_all(buf).unwrap();
                            }
                            Err(e) => {
                                eprintln!("stdin read failed: {e:?}");
                                break;
                            }
                        }
                    }
                    if set.contains(listener_fd) {
                        let (socket, _address) = listener.accept().expect("listener accept");
                        let previous = MONITOR_FD.swap(
                            socket.into_raw_fd(),
                            Relaxed, /* FIXME: correct ordering? */
                        );
                        assert_eq!(previous, -1, "monitor already accepted");
                        // listener.close().expect("listener close");
                        // FIXME: unlink socket path after accept
                    }
                }
            }
            Err(errno) if errno == nix::errno::Errno::EINTR => continue,
            Err(errno) => {
                println!("select failed: {errno:?}");
                break;
            }
        }
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {}
            Err(e) => {
                println!("wait failed: {e:?}");
                break;
            }
        }
    }
}

// called as a signal handler; must only call async-signal-safe functions
extern "C" fn handler(signal: nix::libc::c_int) {
    let _length = nix::unistd::write(2, b"signal received\n").expect("write signal received");
    let signal = signal.try_into().expect("signal try from i32");

    if signal == SIGINT {
        let monitor_fd = MONITOR_FD.swap(-1, Relaxed /* FIXME: correct ordering? */);
        if monitor_fd != -1 {
            let _length = nix::unistd::write(2, b"sending system powerdown\n")
                .expect("write sending system powerdown");
            let _length = nix::unistd::write(monitor_fd, b"system_powerdown\n")
                .expect("write system powerdown");
            // FIXME: carry on on (some kinds of?) failure
            // FIXME: close monitor_fd
            return;
        }
    }
    // FIXME: kill
}

fn main() {
    use std::os::unix::process::ExitStatusExt as _;

    unsafe {
        let _sigaction = sigaction(
            SIGINT,
            &SigAction::new(
                SigHandler::Handler(handler),
                SaFlags::empty(),
                SigSet::empty(),
            ),
        )
        .expect("SIGINT sigaction");
    }

    match std::fs::remove_file("socket" /* FIXME: extract path */) {
        Ok(()) => {}                                                     // carry on
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {} // carry on
        Err(error) => panic!("remove socket file: {error:?}"),
    }

    let listener =
        UnixListener::bind("socket" /* FIXME: extract path */).expect("bind unix listener");

    let mut pty = pty_process::blocking::Pty::new().unwrap();
    let pts = pty.pts().unwrap();
    pty.resize(pty_process::Size::new(24, 80)).unwrap();
    let mut child = pty_process::blocking::Command::new("qemu-system-x86_64")
        .args(&[
    		"-kernel", "/Users/cpick/src/nix-kernel/result/bzImage",
    		"-initrd", "/Users/cpick/src/nix-init/initramfs-overlay-local.config.cpio",
    		"-nic", "user,hostfwd=tcp:127.0.0.1:2223-:22,hostfwd=udp:127.0.0.1:3333-:3333,hostfwd=udp:127.0.0.1:3332-:3332,",
            "-chardev", "socket,id=mon0,path=socket,server=off", // FIXME: extract path
            "-mon", "chardev=mon0",
    		"-nographic",
    		"-no-reboot",
        ])
        .spawn(&pts)
        .unwrap();

    run(listener, &mut child, &mut pty);

    let status = child.wait().unwrap();
    // FIXME: unlink socket path on exit
    std::process::exit(
        status
            .code()
            .unwrap_or_else(|| status.signal().unwrap_or(0) + 128),
    );
}
