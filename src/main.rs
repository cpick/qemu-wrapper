#![deny(
    unsafe_op_in_unsafe_fn,
    warnings,
    clippy::all,
    clippy::as_conversions,
    clippy::multiple_unsafe_ops_per_block,
    clippy::undocumented_unsafe_blocks,
    clippy::unnecessary_safety_comment,
    clippy::unnecessary_safety_doc
)]
#![warn(clippy::pedantic)]

use anyhow::{Context, Result};
use qemu_wrapper::QemuWrapper;
use std::{convert::Infallible, env};

fn main() -> Result<Infallible> {
    qemu_wrapper::mimic_wait_status(
        QemuWrapper::new()
            .context("new qemu wrapper")?
            .run(env::args())
            .context("run")?,
    )
    .context("mimic wait status")
}
