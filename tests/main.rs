use std::{
    env,
    ffi::OsString,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process,
};

use nix::sys::{
    signal::{self, Signal},
    stat::Mode,
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

fn test_guest_binary(name: &str) -> OsString {
    if let Some(path) = env::var_os("TEST_GUEST_PATH") {
        return PathBuf::from_iter([path, (&format!("{name}.efi")).into()]).into_os_string();
    }

    let test_guest_dir = Path::new("test-guest");

    let config = {
        let mut config = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        config.extend([&*test_guest_dir, ".cargo".as_ref(), "config.toml".as_ref()]);

        let config = fs::read_to_string(config).expect("read to string config");
        toml::from_str::<Config>(&config).expect("toml from str config")
    };

    TestBinary::relative_to_parent(name, &test_guest_dir.join("Cargo.toml"))
        .with_target(&config.build.target)
        .build()
        .expect("build test binary orderly")
}

#[test]
fn main() {
    let orderly = test_guest_binary("orderly");

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
        fs::copy(orderly, &boot).expect("copy orderly boot");

        // qemu writes to the executable for some reason
        let mut permissions = fs::metadata(&boot).expect("boot metadata").permissions();
        permissions.set_mode(permissions.mode() | u32::from(Mode::S_IWUSR.bits()));
        fs::set_permissions(&boot, permissions).expect("set boot permissions");
    }

    let mut ovmf_dir = which::which("qemu-system-x86_64").expect("which qemu");
    ovmf_dir.pop(); // filename
    ovmf_dir.pop(); // bin
    ovmf_dir.extend(["share", "qemu"]);

    // must match guest's config
    const READY_FOR_EXIT_SIGNAL_PORT: u8 = include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/test-guest/config/ready-for-exit-signal-port"
    ));

    // must match guest's config
    const EXIT_PORT: u8 = include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/test-guest/config/exit-port"
    ));

    // must match guest's config
    const EXIT_CODE: u8 = include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/test-guest/config/exit-code"
    ));

    let qemu_wrapper = QemuWrapper::new().expect("qemu wrapper new");
    signal::raise(Signal::SIGINT).expect("signal raise");

    let status = qemu_wrapper
        .run([
            &env::args().next().expect("env args next"), // arbitrary
            "--ready-for-exit-signal-port",
            &format!("{READY_FOR_EXIT_SIGNAL_PORT:#04x}"),
            "--exit-port",
            &format!("{EXIT_PORT:#04x}"),
            "--exit-code",
            &EXIT_CODE.to_string(),
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
    assert!(
        matches!(status, WaitStatus::Exited(_process_id, 0)),
        "unexpected status: {status:?}"
    );
}
