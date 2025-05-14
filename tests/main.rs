use std::{env, fs, path::PathBuf};

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
    let directory = PathBuf::from_iter(["testbins", &name]);

    let config = {
        let cargo_manifest_dir =
            env::var_os("CARGO_MANIFEST_DIR").expect("env var os cargo manifest dir");

        let mut config = PathBuf::from(cargo_manifest_dir);
        config.extend([&*directory, ".cargo".as_ref(), "config.toml".as_ref()]);

        let config = fs::read_to_string(config).expect("read to string config");
        toml::from_str::<Config>(&config).expect("toml from str config")
    };

    let _noop = TestBinary::relative_to_parent(name, &directory.join("Cargo.toml"))
        .with_target(&config.build.target)
        .build()
        .expect("build test binary noop");
}
