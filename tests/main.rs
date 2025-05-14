use std::path::{Path, PathBuf};

use test_binary::TestBinary;

#[test]
fn main() {
    let directory = Path::new("testbins");
    let name = "noop";
    let _noop = TestBinary::relative_to_parent(
        name,
        &PathBuf::from_iter([directory, name.as_ref(), "Cargo.toml".as_ref()]),
    )
    .with_target("x86_64-unknown-uefi")
    .build()
    .expect("build test binary noop");
}
