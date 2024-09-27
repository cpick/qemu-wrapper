use std::io::{Read as _, Write as _};
use std::os::fd::{AsFd as _, AsRawFd as _};

mod raw_guard;

pub fn run(child: &mut std::process::Child, pty: &mut pty_process::blocking::Pty) {
    let _raw = raw_guard::RawGuard::new();
    let mut buf = [0_u8; 4096];
    let pty_fd = pty.as_fd().as_raw_fd();
    let stdin_fd = std::io::stdin().as_raw_fd();

    loop {
        let mut set = nix::sys::select::FdSet::new();
        set.insert(pty_fd);
        set.insert(stdin_fd);
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
                }
            }
            Err(e) => {
                println!("select failed: {e:?}");
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

fn main() {
    use std::os::unix::process::ExitStatusExt as _;

    let mut pty = pty_process::blocking::Pty::new().unwrap();
    let pts = pty.pts().unwrap();
    pty.resize(pty_process::Size::new(24, 80)).unwrap();
    let mut child = pty_process::blocking::Command::new("qemu-system-x86_64")
        .args(&[
    		"-kernel", "/Users/cpick/src/nix-kernel/result/bzImage",
    		"-initrd", "/Users/cpick/src/nix-init/initramfs-overlay-local.config.cpio",
    		"-nic", "user,hostfwd=tcp:127.0.0.1:2223-:22,hostfwd=udp:127.0.0.1:3333-:3333,hostfwd=udp:127.0.0.1:3332-:3332,",
            "-chardev", "socket,id=mon0,host=127.0.0.1,port=4444,server=on,wait=off",
            "-mon", "chardev=mon0",
    		"-nographic",
    		"-no-reboot",
        ])
        .spawn(&pts)
        .unwrap();

    run(&mut child, &mut pty);

    let status = child.wait().unwrap();
    std::process::exit(
        status
            .code()
            .unwrap_or_else(|| status.signal().unwrap_or(0) + 128),
    );
}
