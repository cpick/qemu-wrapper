use std::{
    env, fs,
    path::{Path, PathBuf},
    process,
};

use nix::sys::{
    signal::{self, Signal},
    wait::WaitStatus,
};
use qemu_wrapper::QemuWrapper;
use serde::Deserialize;
use test_binary::TestBinary;

#[derive(Deserialize)]
struct Build {
    target: String,
}

#[derive(Deserialize)]
struct Config {
    build: Build,
}

fn remove_dir_all_if_exists<P: AsRef<Path>>(path: P) {
    match fs::remove_dir_all(path) {
        Err(ref error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => panic!("prepare remove dir all esp {error:?}"),
        Ok(_) => (),
    }
}

#[test]
fn main() {
    let name = "orderly";
    let test_guest_dir = Path::new("test-guest");

    let config = {
        let mut config = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        config.extend([&*test_guest_dir, ".cargo".as_ref(), "config.toml".as_ref()]);

        let config = fs::read_to_string(config).expect("read to string config");
        toml::from_str::<Config>(&config).expect("toml from str config")
    };

    let orderly = TestBinary::relative_to_parent(name, &test_guest_dir.join("Cargo.toml"))
        .with_target(&config.build.target)
        .build()
        .expect("build test binary orderly");

    let esp_dir = scopeguard::guard(
        PathBuf::from_iter([
            env!("CARGO_TARGET_TMPDIR"),
            &format!("esp-{}", process::id()),
        ]),
        remove_dir_all_if_exists,
    );
    remove_dir_all_if_exists(&*esp_dir);

    {
        let mut boot = esp_dir.clone();
        boot.extend(["efi", "boot"]);
        fs::create_dir_all(&boot).expect("create dir all boot");
        boot.push("bootx64.efi");
        fs::hard_link(orderly, boot).expect("hard link orderly boot");
    }

    let mut ovmf_dir = which::which("qemu-system-x86_64").expect("which qemu");
    ovmf_dir.pop(); // filename
    ovmf_dir.pop(); // bin
    ovmf_dir.extend(["share", "qemu"]);

    // must match guest's config
    const PORT_EXIT: u8 = include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/test-guest/config/port-exit"
    ));

    let qemu_wrapper = QemuWrapper::new().expect("qemu wrapper new");
    signal::raise(Signal::SIGINT).expect("signal raise");

    let status = qemu_wrapper
        .run([
            &env::args().next().expect("env args next"), // arbitrary
            "--exit-port",
            &format!("{PORT_EXIT:#04x}"),
            "x86_64",
            "-drive",
            &format!(
                "if=pflash,format=raw,readonly=on,file={}",
                ovmf_dir
                    .join("edk2-x86_64-code.fd")
                    .to_str()
                    .expect("ovmf code to str")
            ),
            "-drive",
            &format!(
                "if=pflash,format=raw,readonly=on,file={}",
                ovmf_dir
                    .join("edk2-i386-vars.fd")
                    .to_str()
                    .expect("ovmf vars to str")
            ),
            "-drive",
            &format!(
                "format=raw,file=fat:rw:{}",
                esp_dir.to_str().expect("esp dir to str")
            ),
            "-nic",
            "none",
            "-nographic",
            "-no-reboot",
        ])
        .expect("run orderly status");

    // must match guest's config
    const EXIT_CODE: u8 = include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/test-guest/config/exit-code"
    ));
    assert!(
        matches!(status, WaitStatus::Exited(_process_id, 0)),
        "unexpected status: {status:?}"
    );
}
