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
    sync::{Arc, Mutex},
};

const TARGET: &str = "x86_64"; // must match OPCODE instruction set

type OutbOpcode = [u8; 2];

#[derive(Default)]
struct Ready {
    opcode: OutbOpcode,
    fd: Arc<Mutex<Option<OwnedFd>>>,
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

        let mut port = None;
        let mut fd = None;
        for (argument, value) in &arguments.parsed {
            let Value::Integer(value) = value else {
                bail!("non-integer '{argument}' argument");
            };

            match argument.as_str() {
                ARGUMENT_PORT => {
                    ensure!(port.is_none(), "duplicate '{argument}' argument");

                    port =
                        Some(u8::try_from(*value).with_context(|| {
                            format!("'{argument}' argument too large or negative")
                        })?);
                }
                ARGUMENT_FD => {
                    ensure!(fd.is_none(), "duplicate '{argument}' argument");

                    let value = i32::try_from(*value)
                        .with_context(|| format!("'{argument}' argument too large"))?;
                    ensure!(!value.is_negative(), "'{argument}' argument is negative");
                    fd = Some(value);
                }
                argument => bail!("unknown '{argument}' argument"),
            }
        }
        let port = port.context("missing '{ARGUMENT_PORT}' argument")?;
        let fd = fd.context("missing '{ARGUMENT_FD}' argument")?;

        let mut self_fd = self.fd.lock().expect("lock fd");
        ensure!(
            self.opcode[0] == 0 && self_fd.is_none(),
            "plugin already configured"
        );

        // update `self` atomically now that success is assured
        self.opcode = [0xe6 /* OUT */, port /* imm8 */]; // must match TARGET arch
        // SAFETY: caller is responsible for ensuring this is the correct file descriptor
        *self_fd = Some(unsafe { OwnedFd::from_raw_fd(fd) }); // must only take ownership on `Ok`

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
            .for_each(|instruction| {
                let fd = Arc::clone(&self.fd);
                instruction.register_execute_callback(move |_vcpu| {
                    // close fd to signal parent process
                    if fd.lock().expect("lock fd").take().is_none() {
                        return;
                    }

                    qemu_plugin_outs("VM has signaled that it is ready\n")
                        .expect("qemu plugin outs");
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
    use super::{Ready, TARGET};
    use qemu_plugin::install::{Args, Info, Value, Version};
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
                    version: Version {
                        current: 4,
                        mininum: 4,
                    },
                    system: None,
                },
            )
            .expect("configure");
    }
}
