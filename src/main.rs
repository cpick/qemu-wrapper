use nix::sys::signal::{
    kill, raise, sigaction, sigprocmask, SaFlags, SigAction, SigHandler, SigSet, SigmaskHow, Signal,
};
use nix::unistd::Pid;
use std::io::{Read as _, Write as _};
use std::os::fd::{AsFd as _, AsRawFd as _, IntoRawFd as _};
use std::os::unix::net::UnixListener;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering::Relaxed};

mod raw_guard;

const POWEROFF_SIGNAL: Signal = Signal::SIGINT; // TODO: make configurable?
static MONITOR_FD: AtomicI32 = AtomicI32::new(-1);
static CHILD_PROCESS_ID: AtomicU32 = AtomicU32::new(0);

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
                eprintln!("select failed: {errno:?}");
                break;
            }
        }
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {}
            Err(e) => {
                eprintln!("wait failed: {e:?}");
                break;
            }
        }
    }
}

// called as a signal handler; must only call async-signal-safe functions
extern "C" fn handler(signal: nix::libc::c_int) {
    let _length = nix::unistd::write(2, b"signal received\n").expect("write signal received");
    let signal = signal.try_into().expect("signal try from i32");

    if signal == POWEROFF_SIGNAL {
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

    // sigprocmask() around spawn() and store() prevent race on CHILD_PROCESS_ID
    {
        let child_process_id = CHILD_PROCESS_ID.load(Relaxed);
        if child_process_id > 0 {
            let _length = nix::unistd::write(2, b"killing child process group\n")
                .expect("write killing child process group");
            kill(
                Pid::from_raw(-(child_process_id as nix::libc::pid_t)),
                signal,
            )
            .expect("kill child process group");
            return;
        }
    }

    let _length = nix::unistd::write(2, b"resetting handler and reraising signal\n")
        .expect("resetting handler and reraising signal");
    unsafe {
        let _sigaction = sigaction(
            signal,
            &SigAction::new(SigHandler::SigDfl, SaFlags::empty(), SigSet::empty()),
        )
        .expect("default sigaction");
    }
    raise(signal).expect("raise signal");
}

fn main() {
    use std::os::unix::process::ExitStatusExt as _;

    let mut handled = SigSet::empty();
    {
        let action = SigAction::new(
            SigHandler::Handler(handler),
            SaFlags::empty(),
            SigSet::empty(),
        );
        // TODO: handle realtime signals between SIGRTMIN and SIGRTMAX?
        for signal in Signal::iterator().filter(|signal| match signal {
            Signal::SIGKILL | Signal::SIGSTOP // unactionable
            | Signal::SIGSEGV | Signal::SIGBUS // rust's stack overflow reporter
            => false,
            _ => true,
        }) {
            let _action = unsafe { sigaction(signal, &action).expect("sigaction") };
            handled.add(signal);
        }
    }

    let socket_path = format!("monitor-{}.sock", std::process::id());

    match std::fs::remove_file(&socket_path) {
        Ok(()) => {}                                                     // carry on
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {} // carry on
        Err(error) => panic!("remove socket file: {error:?}"),
    }

    let listener = UnixListener::bind(&socket_path).expect("bind unix listener");

    let mut pty = pty_process::blocking::Pty::new().unwrap();
    let pts = pty.pts().unwrap();
    pty.resize(pty_process::Size::new(24, 80)).unwrap();

    let mut command = pty_process::blocking::Command::new("qemu-system-x86_64");
    command.args(
        [
            "-chardev".to_owned(),
            format!("socket,id=mon0,path={socket_path},server=off"),
            "-mon".to_owned(),
            "chardev=mon0".to_owned(),
        ]
        .into_iter()
        .chain(std::env::args()),
    );

    // prevent race between the signal handler and setting CHILD_PROCESS_ID
    let mut previous = SigSet::empty();
    sigprocmask(SigmaskHow::SIG_BLOCK, Some(&handled), Some(&mut previous))
        .expect("block sigprocmask");
    unsafe {
        command.pre_exec(move || {
            sigprocmask(SigmaskHow::SIG_SETMASK, Some(&previous), None)
                .map_err(|errno| std::io::Error::from_raw_os_error(errno as i32))
        });
    }
    let mut child = command.spawn(&pts).expect("spawn command");
    CHILD_PROCESS_ID.store(child.id(), Relaxed);
    sigprocmask(SigmaskHow::SIG_SETMASK, Some(&previous), None).expect("unblock sigprocmask");

    run(listener, &mut child, &mut pty);

    let status = child.wait().unwrap();
    // FIXME: unlink socket path on exit
    eprintln!("exit()ing with status: {status}");
    std::process::exit(
        status
            .code()
            .unwrap_or_else(|| status.signal().unwrap_or(0) + 128),
    );
}
