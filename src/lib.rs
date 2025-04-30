use anyhow::{Context, Result, anyhow, bail, ensure};
use ctor::ctor;
use itertools::Itertools;
use qemu_plugin::{
    PluginId, TranslationBlock,
    install::{Args, Info, Value},
    plugin::{HasCallbacks, PLUGIN, Plugin, Register},
    qemu_plugin_outs, qemu_plugin_uninstall,
};
use std::{
    os::fd::{FromRawFd, OwnedFd},
    sync::Mutex,
};

pub static FD: Mutex<Option<OwnedFd>> = Mutex::new(None);

struct Ready {}

impl Register for Ready {
    fn register(&mut self, _id: PluginId, arguments: &Args, _info: &Info) -> Result<()> {
        let (argument, value) = arguments
            .parsed
            .iter()
            .exactly_one()
            .map_err(|error| anyhow!("expected argument '{ARGUMENT_FD}': {error}"))?;

        const ARGUMENT_FD: &str = "fd";
        ensure!(
            ARGUMENT_FD == argument,
            "expected argument: '{ARGUMENT_FD}' got: '{argument}'"
        );

        let Value::Integer(fd) = value else {
            bail!("non-integer '{ARGUMENT_FD}' argument");
        };

        let fd = i32::try_from(*fd).with_context(|| format!("'{ARGUMENT_FD}' too large"))?;
        ensure!(!fd.is_negative(), "'{ARGUMENT_FD}' is negative");

        // SAFETY: caller is responsible for ensuring this is the correct file descriptor
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };

        {
            let preexisting_fd = FD.lock().expect("lock fd").replace(fd).is_some();
            ensure!(!preexisting_fd, "unexpected, preexisting fd");
        }

        Ok(())
    }
}

impl HasCallbacks for Ready {
    fn on_translation_block_translate(&mut self, id: PluginId, tb: TranslationBlock) -> Result<()> {
        const OPCODE: [u8; 2] = [0xe6 /* OUT */, 0xf5 /* imm8 port */];

        tb.instructions()
            .filter(|instruction| {
                (instruction.size() == OPCODE.len()) && (instruction.data() == OPCODE)
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

impl Plugin for Ready {}

#[ctor]
fn init() {
    PLUGIN
        .set(Mutex::new(Box::new(Ready {})))
        .map_err(|_| anyhow::anyhow!("Failed to set plugin"))
        .expect("Failed to set plugin");
}
