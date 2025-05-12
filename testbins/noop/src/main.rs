#![no_main]
#![no_std]

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

#[panic_handler]
fn panic_handler(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[unsafe(export_name = "efi_main")]
pub extern "C" fn main(_h: *mut core::ffi::c_void, _st: *mut core::ffi::c_void) -> usize {
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

    0
}
