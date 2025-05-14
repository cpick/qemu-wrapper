use std::{
    env, fs,
    path::PathBuf,
    process::{self, Command},
};

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

#[test]
fn main() {
    let name = "noop";
    let source_dir = PathBuf::from_iter(["testbins", &name]);

    let config = {
        let mut config = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        config.extend([&*source_dir, ".cargo".as_ref(), "config.toml".as_ref()]);

        let config = fs::read_to_string(config).expect("read to string config");
        toml::from_str::<Config>(&config).expect("toml from str config")
    };

    let noop = TestBinary::relative_to_parent(name, &source_dir.join("Cargo.toml"))
        .with_target(&config.build.target)
        .build()
        .expect("build test binary noop");

    let esp_dir = scopeguard::guard(
        PathBuf::from_iter([
            env!("CARGO_TARGET_TMPDIR"),
            &format!("esp-{}", process::id()),
        ]),
        |dir| fs::remove_dir_all(dir).expect("remove dir all"),
    );
    // FIXME: warn and remove if already exists

    {
        let mut boot = esp_dir.clone();
        boot.extend(["efi", "boot"]);
        fs::create_dir_all(&boot).expect("create dir all boot");
        boot.push("bootx64.efi");
        fs::hard_link(noop, boot).expect("hard link noop boot");
    }

    const QEMU: &str = "qemu-system-x86_64";
    let mut ovmf_dir = which::which(QEMU).expect("which qemu");
    ovmf_dir.pop(); // filename
    ovmf_dir.pop(); // bin
    ovmf_dir.extend(["share", "qemu"]);
    let ovmf_dir = ovmf_dir.to_str().expect("ovmf dir to str");

    let status = Command::new(QEMU)
        .args([
            "-drive",
            &format!("if=pflash,format=raw,readonly=on,file={ovmf_dir}/edk2-x86_64-code.fd"),
            "-drive",
            &format!("if=pflash,format=raw,readonly=on,file={ovmf_dir}/edk2-i386-vars.fd"),
            "-drive",
            &format!(
                "format=raw,file=fat:rw:{}",
                esp_dir.to_str().expect("esp dir to str")
            ),
            "-device",
            "isa-debug-exit,iobase=0xf4,iosize=0x01",
            "-nic",
            "none",
            "-nographic",
            "-no-reboot",
        ])
        .status()
        .expect("command noop status");
    assert_eq!(Some(7), status.code());
}
