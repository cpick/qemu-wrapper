use anyhow::{bail, Context, Result};
use nix::sys::signal::{
    kill, raise, sigaction, sigprocmask, SaFlags, SigAction, SigHandler, SigSet, SigmaskHow, Signal,
};
use nix::unistd::{close, write, Pid};
use pty_process::blocking::Pty;
use std::io::{stdin, stdout, Error, ErrorKind, Read as _, Write as _};
use std::os::fd::{AsFd as _, AsRawFd as _, IntoRawFd as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::ExitStatusExt as _;
use std::process::{exit, id, Child, ExitStatus};
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering::Relaxed};

mod raw_guard;

const POWEROFF_SIGNAL: Signal = Signal::SIGINT; // TODO: make configurable?
static MONITOR_FD: AtomicI32 = AtomicI32::new(-1);
static CHILD_PROCESS_ID: AtomicU32 = AtomicU32::new(0);

// called as a signal handler; must only call async-signal-safe functions
fn stderr_write(message: &str) {
    let _ = write(2, message.as_bytes());
}

// called as a signal handler; must only call async-signal-safe functions
fn stderr_writeln(message: &str) {
    stderr_write(message);
    stderr_write("\r\n"); // in raw mode; include carriage return
}

pub trait Fallible<T> {
    // called as a signal handler; must only call async-signal-safe functions
    fn or_fail(self, message: &str) -> T;
}

impl<T, E> Fallible<T> for Result<T, E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    // called as a signal handler; must only call async-signal-safe functions
    fn or_fail(self, message: &str) -> T {
        match self {
            Ok(ok) => ok,
            Err(_error) => {
                stderr_write("Error: ");
                stderr_writeln(message);
                exit(libc::EXIT_FAILURE)
            }
        }
    }
}

// called as a signal handler; must only call async-signal-safe functions
extern "C" fn signal_handler(signal: nix::libc::c_int) {
    let signal = Signal::try_from(signal).or_fail("signal try from i32");
    stderr_write("received signal: ");
    stderr_writeln(signal.as_str());

    match signal {
        POWEROFF_SIGNAL => {
            let monitor_fd = MONITOR_FD.swap(-1, Relaxed /* FIXME: correct ordering? */);
            if monitor_fd != -1 {
                stderr_writeln("sending system powerdown");
                let _length =
                    write(monitor_fd, b"system_powerdown\n").or_fail("write system powerdown");
                close(monitor_fd).or_fail("close monitor");
                // FIXME: carry on on (some kinds of?) failure
                return;
            }
            // carry on
        }
        Signal::SIGCHLD => {
            stderr_writeln("child process died");
            // FIXME: does this race with try_wait()?
            // need to be sure CHILD_PROCESS_ID is cleared before the child is reaped so this
            // handler doesn't send signals to a reused PID
            // FIXME: do other signals need to be blocked while handling this one?
            // perhaps return early here, sigprocmask() around try_wait() and clear
            // CHILD_PROCESS_ID there?
            CHILD_PROCESS_ID.store(0, Relaxed /* FIXME: correct ordering? */);
            return;
        }
        // TODO: SIGWINCH
        _signal => (), // carry on
    }

    // sigprocmask() around spawn() and store() prevent race on CHILD_PROCESS_ID
    {
        let child_process_id = CHILD_PROCESS_ID.load(Relaxed);
        if child_process_id > 0 {
            stderr_writeln("killing child process group");
            kill(
                Pid::from_raw(-(child_process_id as nix::libc::pid_t)),
                signal,
            )
            .or_fail("kill child process group");
            return;
        }
    }

    stderr_writeln("resetting handler and reraising signal");
    unsafe {
        let _sigaction = sigaction(
            signal,
            &SigAction::new(SigHandler::SigDfl, SaFlags::empty(), SigSet::empty()),
        )
        .or_fail("default sigaction");
    }
    raise(signal).or_fail("raise signal");
}

fn handle_signals() -> Result<SigSet> {
    let mut handled = SigSet::empty();
    let action = SigAction::new(
        SigHandler::Handler(signal_handler),
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
        let _action =
            unsafe { sigaction(signal, &action) }.with_context(|| format!("{signal} sigaction"))?;
        handled.add(signal);
    }

    Ok(handled)
}

struct MonitorListener {
    path: String,
    listener: UnixListener,
}

impl MonitorListener {
    fn new() -> Result<Self> {
        let path = format!("monitor-{}.sock", id());

        match std::fs::remove_file(&path) {
            Ok(()) => {}                                            // carry on
            Err(error) if error.kind() == ErrorKind::NotFound => {} // carry on
            Err(error) => bail!("remove socket file '{path}': {error:?}"),
        }

        let listener = UnixListener::bind(&path).context("bind unix listener")?;

        Ok(Self { path, listener })
    }

    fn path(&self) -> &str {
        &self.path
    }

    fn raw_fd(&self) -> i32 {
        self.listener.as_raw_fd()
    }

    fn accept(self) -> Result<UnixStream> {
        let (socket, _address) = self.listener.accept().context("listener accept")?;
        Ok(socket)
    }
}

impl Drop for MonitorListener {
    fn drop(&mut self) {
        let path = self.path();
        if let Err(error) = std::fs::remove_file(path) {
            stderr_writeln(&format!("remove file '{path}': {error:?}"));
        }
    }
}

fn spawn_qemu_child(
    arguments: impl IntoIterator<Item = String>,
    handled: SigSet,
    listener_path: &str,
) -> Result<(Pty, Child)> {
    let pty = pty_process::blocking::Pty::new().context("new pty")?;
    let pts = pty.pts().context("pty pts")?;
    pty.resize(
        pty_process::Size::new(24, 80), /* FIXME: query from parent terminal */
    )
    .context("resize ptpy")?;

    let mut command = pty_process::blocking::Command::new("qemu-system-x86_64");
    command.args(
        [
            "-chardev".to_owned(),
            format!("socket,id=mon0,path={},server=off", listener_path),
            "-mon".to_owned(),
            "chardev=mon0".to_owned(),
        ]
        .into_iter()
        .chain(arguments),
    );

    // prevent race between the signal handler and setting CHILD_PROCESS_ID
    let child = {
        let mut previous = SigSet::empty();
        sigprocmask(SigmaskHow::SIG_BLOCK, Some(&handled), Some(&mut previous))
            .context("block sigprocmask")?;

        unsafe {
            command.pre_exec(move || {
                sigprocmask(SigmaskHow::SIG_SETMASK, Some(&previous), None)
                    .map_err(|errno| Error::from_raw_os_error(errno as i32))
            });
        }
        let child = command.spawn(&pts).context("spawn command")?;
        CHILD_PROCESS_ID.store(child.id(), Relaxed);

        sigprocmask(SigmaskHow::SIG_SETMASK, Some(&previous), None)
            .context("unblock sigprocmask")?;

        child
    };

    Ok((pty, child))
}

fn run(listener: MonitorListener, mut pty: Pty, mut child: Child) -> Result<ExitStatus> {
    let _raw = raw_guard::RawGuard::new();
    let mut buf = [0_u8; 4096];
    let pty_fd = pty.as_fd().as_raw_fd();
    let stdin_fd = stdin().as_raw_fd();
    let mut listener = Some(listener);

    loop {
        let mut set = nix::sys::select::FdSet::new();
        set.insert(pty_fd);
        set.insert(stdin_fd);
        if let Some(ref listener) = listener {
            set.insert(listener.raw_fd());
        }
        match nix::sys::select::select(None, Some(&mut set), None, None, None) {
            Ok(n) => {
                if n > 0 {
                    if set.contains(pty_fd) {
                        let bytes = pty.read(&mut buf).context("read pty")?;
                        let buf = &buf[..bytes];
                        let stdout = stdout();
                        let mut stdout = stdout.lock();
                        stdout.write_all(buf).context("write all stdout")?;
                        stdout.flush().context("flush stdout")?;
                    }
                    if set.contains(stdin_fd) {
                        let bytes = stdin().read(&mut buf).context("read stdin")?;
                        let buf = &buf[..bytes];
                        pty.write_all(buf).context("write all pty")?;
                    }
                    // TODO: so ugly
                    if let Some(l) = listener {
                        listener = if set.contains(l.raw_fd()) {
                            let monitor = l.accept().context("monitor accept")?;
                            let previous = MONITOR_FD.swap(
                                monitor.into_raw_fd(),
                                Relaxed, /* FIXME: correct ordering? */
                            );
                            assert_eq!(previous, -1, "monitor already accepted");
                            None
                        } else {
                            Some(l)
                        };
                    }
                }
            }
            Err(errno) if errno == nix::errno::Errno::EINTR => (), // carry on
            Err(errno) => bail!("select failed errno: {errno:?}"),
        }
        // FIXME: ensure everything is forwarded from pty to stdout
        match child.try_wait().context("try wait")? {
            None => (), // carry on
            Some(status) => return Ok(status),
        }
    }
}

fn main() -> Result<()> {
    let handled = handle_signals().context("handle signals")?;
    let listener = MonitorListener::new().context("monitor socket")?;
    let (pty, child) = spawn_qemu_child(
        std::env::args().skip(1 /* argv[0] */),
        handled,
        listener.path(),
    )
    .context("spawn qemu")?;
    let status = run(listener, pty, child).context("run")?;

    stderr_writeln(&format!("Exiting; relaying child status: {status}"));
    exit(
        status
            .code()
            .unwrap_or_else(|| status.signal().unwrap_or(0) + 128),
    );
}
