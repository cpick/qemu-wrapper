#![no_main]
#![no_std]

use log::{info, warn};
use uefi::{Status, entry};

const PORT_DATA: u8 = {
    const EXIT_CODE: u8 = 7; // must match qemu-wrapper's EXIT_CODE

    // QEMU takes the value written to the port and multiplies by 2 and adds 1, reverse the process:
    // https://gitlab.com/qemu-project/qemu/-/blob/019fbfa4bcd2d3a835c241295e22ab2b5b56129b/hw/misc/debugexit.c#L36-L37
    const _: () = assert!(
        (EXIT_CODE & 1) == 1,
        "EXIT_CODE must be odd since QEMU always ORs in 1"
    );

    EXIT_CODE >> 1
};

// usually unused: https://os.phil-opp.com/testing/#i-o-ports
const PORT: u8 = 0xf4;

#[entry]
fn main() -> Status {
    uefi::helpers::init().unwrap();

    info!("exiting");
    // SAFETY: writing to (low?) port seems safe?
    unsafe {
        // must match qemu-wrapper's ready plugin's OPCODE
        core::arch::asm!(
            "out {port}, al",
            in("al") PORT_DATA,
            port = const PORT,
            options(nomem, nostack)
        );
    }

    warn!("unexpectedly alive");
    Status::SUCCESS
}
