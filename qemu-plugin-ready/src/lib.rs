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

use anyhow::{Context, Result, bail, ensure};
use qemu_plugin::{
    Args, HasCallbacks, Info, PluginId, Register, TranslationBlock, Value, qemu_plugin_outs,
    qemu_plugin_uninstall,
};
use std::{
    os::fd::{FromRawFd, OwnedFd},
    sync::Mutex,
};

const TARGET: &str = "x86_64"; // must match OPCODE instruction set

pub static FD: Mutex<Option<OwnedFd>> = Mutex::new(None);

type OutbOpcode = [u8; 2];

#[derive(Default)]
struct Ready {
    opcode: OutbOpcode,
}

impl Ready {
    fn configure(&mut self, arguments: &Args, info: &Info) -> Result<()> {
        const ARGUMENT_PORT: &str = "port";
        const ARGUMENT_FD: &str = "fd";

        ensure!(
            TARGET == info.target_name,
            "expected target: '{TARGET}' got: '{}'",
            info.target_name
        );

        for (argument, value) in &arguments.parsed {
            let Value::Integer(value) = value else {
                bail!("non-integer '{argument}' argument");
            };

            match argument.as_str() {
                ARGUMENT_PORT => {
                    ensure!(self.opcode[0] == 0, "duplicate '{argument}' argument");

                    let port = u8::try_from(*value)
                        .with_context(|| format!("'{argument}' argument too large or negative"))?;
                    self.opcode = [0xe6 /* OUT */, port /* imm8 */]; // must match TARGET arch
                }
                ARGUMENT_FD => {
                    let fd = i32::try_from(*value)
                        .with_context(|| format!("'{argument}' argument too large"))?;
                    ensure!(!fd.is_negative(), "'{argument}' argument is negative");

                    // SAFETY: caller is responsible for ensuring this is the correct file descriptor
                    let fd = unsafe { OwnedFd::from_raw_fd(fd) };

                    {
                        let preexisting_fd = FD.lock().expect("lock fd").replace(fd).is_some();
                        ensure!(!preexisting_fd, "duplicate '{argument}' argument");
                    }
                }
                argument => bail!("unknown '{argument}' argument"),
            }
        }

        ensure!(self.opcode[0] != 0, "missing '{ARGUMENT_PORT}' argument");
        ensure!(
            FD.lock().expect("lock fd").is_some(),
            "missing '{ARGUMENT_FD}' argument"
        );

        Ok(())
    }
}

impl Register for Ready {
    fn register(
        &mut self,
        _id: PluginId,
        arguments: &Args,
        info: &Info,
    ) -> qemu_plugin::Result<()> {
        self.configure(arguments, info).map_err(Into::into)
    }
}

impl HasCallbacks for Ready {
    fn on_translation_block_translate(
        &mut self,
        id: PluginId,
        tb: TranslationBlock,
    ) -> qemu_plugin::Result<()> {
        tb.instructions()
            .filter(|instruction| {
                if instruction.size() != self.opcode.len() {
                    return false;
                }

                let mut data: OutbOpcode = [0; 2];
                (instruction.read_data(&mut data) == self.opcode.len()) && (data == self.opcode)
            })
            .for_each(move |instruction| {
                instruction.register_execute_callback(move |_vcpu| {
                    qemu_plugin_outs("VM has signaled that it is ready\n")
                        .expect("qemu plugin outs");

                    drop(FD.lock().expect("lock fd").take()); // close fd to signal parent process

                    qemu_plugin_uninstall(id, |_id| {}).expect("qemu plugin uninstall");
                });
            });

        Ok(())
    }
}

#[cfg(not(test))]
qemu_plugin::register!(Ready::default());

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::{fd::IntoRawFd, unix::net::UnixStream};

    #[test]
    fn configure() {
        let (_reader, writer) = UnixStream::pair().expect("socket pair");

        Ready::default()
            .configure(
                &Args {
                    raw: Vec::new(),
                    parsed: [
                        ("port".into(), Value::Integer(0xff)),
                        ("fd".into(), Value::Integer(writer.into_raw_fd().into())),
                    ]
                    .into(),
                },
                &Info {
                    target_name: TARGET.into(),
                    version: qemu_plugin::Version {
                        current: 4,
                        mininum: 4,
                    },
                    system: None,
                },
            )
            .expect("configure");
    }
}
