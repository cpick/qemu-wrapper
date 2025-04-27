use anyhow::Result;
use ctor::ctor;
use qemu_plugin::{
    PluginId, TranslationBlock,
    plugin::{HasCallbacks, PLUGIN, Plugin, Register},
};
use std::sync::Mutex;

struct TinyTrace {}

impl Register for TinyTrace {}

impl HasCallbacks for TinyTrace {
    fn on_translation_block_translate(
        &mut self,
        _id: PluginId,
        tb: TranslationBlock,
    ) -> Result<()> {
        tb.instructions().enumerate().try_for_each(|(idx, insn)| {
            if idx == 0 {
                println!("====TB: {:08x}", insn.vaddr());
            }

            println!("{:08x}: {}", insn.vaddr(), insn.disas()?);
            Ok::<(), anyhow::Error>(())
        })?;

        Ok(())
    }
}

impl Plugin for TinyTrace {}

#[ctor]
fn init() {
    PLUGIN
        .set(Mutex::new(Box::new(TinyTrace {})))
        .map_err(|_| anyhow::anyhow!("Failed to set plugin"))
        .expect("Failed to set plugin");
}
