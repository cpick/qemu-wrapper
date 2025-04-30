use anyhow::Result;
use ctor::ctor;
use qemu_plugin::{
    PluginId, TranslationBlock,
    plugin::{HasCallbacks, PLUGIN, Plugin, Register},
    qemu_plugin_outs, qemu_plugin_uninstall,
};
use std::sync::Mutex;

struct Ready {}

impl Register for Ready {}

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
