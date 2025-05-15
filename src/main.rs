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

use anyhow::{Context, Result, bail};
use nix::sys::wait::WaitStatus;
use signal_hook::low_level::emulate_default_handler;
use std::process::ExitCode;

fn main() -> Result<ExitCode> {
    match qemu_wrapper::run().context("run")? {
        WaitStatus::Exited(_process_id, code) => {
            Ok(u8::try_from(code).expect("exit code try from u8").into())
        }
        WaitStatus::Signaled(_process_id, signal, _dumped_core) => {
            #[allow(clippy::as_conversions)]
            emulate_default_handler(signal as i32).context("emulate default fatal handler")?;
            panic!("non-fatal signal: {signal}");
        }
        status => bail!("wait status unexpected: {status:?}"),
    }
}
