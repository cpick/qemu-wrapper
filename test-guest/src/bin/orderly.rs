#![no_main]
#![no_std]

use acpi::{
    AcpiTables,
    address::AddressSpace,
    fadt::Fadt,
    handler::{AcpiHandler, PhysicalMapping},
};
use core::{
    arch::asm,
    ptr::{self, NonNull},
};
use log::{info, trace};
use uefi::{Status, boot, entry, helpers, system, table::cfg};

const PORT_DATA: u8 = {
    // must match qemu-wrapper's EXIT_CODE
    const EXIT_CODE: u8 = include!(concat!(env!("CARGO_MANIFEST_DIR"), "/config/exit-code"));

    // QEMU takes the value written to the port and multiplies by 2 and adds 1, reverse the process:
    // https://gitlab.com/qemu-project/qemu/-/blob/019fbfa4bcd2d3a835c241295e22ab2b5b56129b/hw/misc/debugexit.c#L36-L37
    const _: () = assert!(
        (EXIT_CODE & 1) == 1,
        "EXIT_CODE must be odd since QEMU always ORs in 1"
    );

    EXIT_CODE >> 1
};

// must match QEMU's isa-debug-exit device's iobase
const PORT_EXIT: u8 = include!(concat!(env!("CARGO_MANIFEST_DIR"), "/config/port-exit"));

// must match qemu-wrapper's ready plugin's OPCODE
const PORT_READY_FOR_EXIT_SIGNAL: u8 = include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/config/port-ready-for-exit-signal"
));

const _: () = assert!(
    PORT_EXIT != PORT_READY_FOR_EXIT_SIGNAL,
    "PORT_EXIT and PORT_READY_FOR_EXIT_SIGNAL must be different"
);

#[derive(Clone)]
struct DirectHandler {}

impl DirectHandler {
    fn new() -> Self {
        Self {}
    }
}

impl AcpiHandler for DirectHandler {
    unsafe fn map_physical_region<T>(
        &self,
        physical_address: usize,
        size: usize,
    ) -> PhysicalMapping<Self, T> {
        unsafe {
            PhysicalMapping::new(
                physical_address,
                NonNull::new(physical_address as *mut _).expect("non null"),
                size,
                size,
                self.clone(),
            )
        }
    }
    fn unmap_physical_region<T>(_region: &PhysicalMapping<Self, T>) {}
}

#[entry]
fn main() -> Status {
    helpers::init().expect("uefi init");

    info!("loading acpi tables");
    let rsdp_address = system::with_config_table(|configuration_table| {
        configuration_table
            .iter()
            .find(|entry| entry.guid == cfg::ACPI2_GUID)
            .map(|entry| entry.address)
    })
    .expect("rsdp address");
    info!("rsdp address: {rsdp_address:?}");

    let acpi = unsafe { AcpiTables::from_rsdp(DirectHandler::new(), rsdp_address as usize) }
        .expect("acpi from rsdp");
    info!("acpi revision: {}", acpi.revision());

    let fadt = acpi.find_table::<Fadt>().expect("acpi find table fadt");
    {
        let flags = unsafe { ptr::read_unaligned(&raw const fadt.flags) };
        info!(
            "fadt acpi enable: {:#x} power button is control method: {}",
            fadt.acpi_enable,
            flags.power_button_is_control_method()
        );
        assert!(!flags.power_button_is_control_method());
    }

    let pm1a_event_block = fadt.pm1a_event_block().expect("fadt pm1a event block");
    info!("fadt pm1a event block: {pm1a_event_block:#x?}");
    assert_eq!(AddressSpace::SystemIo, pm1a_event_block.address_space);
    type Pm1Register = u16;
    assert_eq!(
        Pm1Register::BITS * 2, // status register and enable register
        pm1a_event_block.bit_width.into()
    );
    assert_eq!(0, pm1a_event_block.bit_offset);

    assert!(
        fadt.pm1b_event_block()
            .expect("fadt pm1b event block")
            .is_none()
    );

    let pm1a_status_port =
        u16::try_from(pm1a_event_block.address).expect("try from pm1a event block address");
    let pm1a_enable_port = pm1a_status_port
        + u16::try_from(size_of::<Pm1Register>()).expect("try from size of pm1 event");

    info!("enabling acpi power button");
    {
        // https://uefi.org/htmlspecs/ACPI_Spec_6_4_html/04_ACPI_Hardware_Specification/ACPI_Hardware_Specification.html#pm1-enable-registers-fixed-hardware-feature-enable-bits
        const PM1_ENABLE_POWER_BUTTON: Pm1Register = 0x0100;
        unsafe {
            asm!(
                "out dx, ax",
                in("dx") pm1a_enable_port,
                in("ax") PM1_ENABLE_POWER_BUTTON,
                options(nomem, nostack)
            );
        }
    }

    info!("indicating virtual machine is ready for exit signal");
    unsafe {
        // must match qemu-wrapper's ready plugin's OPCODE
        asm!(
            "out {port}, al",
            in("al") PORT_DATA,
            port = const PORT_READY_FOR_EXIT_SIGNAL,
            options(nomem, nostack)
        );
    }

    info!("awaiting acpi power button press");
    loop {
        // https://uefi.org/htmlspecs/ACPI_Spec_6_4_html/04_ACPI_Hardware_Specification/ACPI_Hardware_Specification.html#pm1-status-registers-fixed-hardware-feature-status-bits
        const PM1_STATUS_POWER_BUTTON: Pm1Register = 0x0100;

        boot::stall(1 /* sec */ * 1000 /* ms */ * 1000 /* us */);
        let pm1a_status: Pm1Register;
        unsafe {
            asm!(
                "in ax, dx",
                in("dx") pm1a_status_port,
                out("ax") pm1a_status,
                options(nomem, nostack)
            );
        }
        trace!("pm1a event: {pm1a_status:#x}");
        if (PM1_STATUS_POWER_BUTTON & pm1a_status) != 0 {
            break;
        }
    }

    info!("acpi power button pressed, exiting");
    unsafe {
        asm!(
            "out {port}, al",
            in("al") PORT_DATA,
            port = const PORT_EXIT,
            options(nomem, nostack)
        );
    }

    panic!("unexpectedly alive after attempted exit");
}
