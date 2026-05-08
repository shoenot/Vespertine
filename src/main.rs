#![no_std]
#![no_main]
mod arch;
mod drivers;
mod kernel;
mod boot;
mod tests;

extern crate alloc;

use core::{
    panic::PanicInfo, 
    arch::asm, 
    fmt::Write
};
use simple_psf::*;
pub use boot::*;

use drivers::serial::*;
use drivers::graphics::*;

use arch::x86_64::interrupts::gdt::init_gdt;
use arch::x86_64::interrupts::idt::init_idt;
use arch::x86_64::apic::lapic::{Local_APIC, get_apic_base};

use kernel::lock::TicketLock;

use kernel::memory::pmm::*;
use kernel::memory::paging::*;
use kernel::memory::vmm::*;
use kernel::memory::heap::KernelAllocator;

use kernel::acpi;

use tests::memory_tests::*;

#[global_allocator]
pub static KERNEL_ALLOCATOR: KernelAllocator = KernelAllocator::new();

static ALLOCATOR: TicketLock<Allocator> = TicketLock::new(Allocator::new());
static PAGER: TicketLock<Pager> = TicketLock::new(Pager::new(&ALLOCATOR));
static GLOBAL_VMM: TicketLock<VirtMemManager> = TicketLock::new(VirtMemManager::new(&PAGER, &ALLOCATOR));

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    log_to_serial("!!! KERNEL PANIC : ");
    let mut writer = SerialWriter;
    let _ = write!(&mut writer, "{}\n", info);
    hcf();
}

fn hcf() -> ! {
    loop {
        unsafe {
            #[cfg(target_arch = "x86_64")]
            asm!("hlt");
        }
    }
}

struct Logger<'a> {
    graphics_writer: &'a mut GraphicsWriter<'a>,
    serial_writer: &'a mut SerialWriter,
}

impl<'a> core::fmt::Write for Logger<'a> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.graphics_writer.write_str(s)?;
        self.serial_writer.write_str(s)?;
        Ok(())
    }
}

const FONT_DATA: &[u8] = include_bytes!("../build_deps/zap-ext-light16.psf");

#[unsafe(no_mangle)]
pub extern "C" fn kmain() -> ! {
    if !BASE_REVISION.is_supported() {
        hcf();
    }

    unsafe {
        init_serial();
        log_to_serial("\x1B[2J\x1B[H");
        log_to_serial("INITIATING GDT... ");
        init_gdt();
        log_to_serial("INITIATING IDT... ");
        init_idt();
    }

    let font = match Psf::parse(FONT_DATA) {
        Ok(f) => f,
        Err(ParseError::HeaderMissing) => { panic!("FONT LOAD FAILED: HEADER MISSING") },
        Err(ParseError::InvalidMagicBytes) => { panic!("FONT LOAD FAILED: INVALID MAGIC BYTES") },
        Err(ParseError::UnknownVersion(_)) => { panic!("FONT LOAD FAILED: UNKNOWN VERSION") },
        Err(ParseError::GlyphTableTruncated {..}) => { panic!("FONT LOAD FAILED: GLYPH TABLE TRUNCATED") },
    };
    log_to_serial("FONT LOADED\n");

    let fb = if let Some(fb_response) = FRAMEBUFFER_REQUEST.response() {
        if let Some(fb) = fb_response.framebuffers().first() {
            fb
        } else { panic!("Cannot get framebuffer") }
    } else { panic!("Cannot get framebuffer") };

    let mut graphics_writer = GraphicsWriter {
        current_line: 0,
        current_offset: 0,
        font: &font,
        fb: &fb
    };

    let mut serial_writer = SerialWriter;

    let mut logger = Logger {
        graphics_writer: &mut graphics_writer,
        serial_writer: &mut serial_writer,
    };


    write!(&mut logger, "Initiating PMM... ").unwrap();
    
    // Inititate PMM
    {
        let mut allocator = ALLOCATOR.lock();
        allocator.init();
    }

    write!(&mut logger, "Physical Memory Allocator initiated.\n").unwrap();

    // Inititate Pager
    {
        let mut pager = PAGER.lock();
        pager.init();
    }

    write!(&mut logger, "Switched CR3\n").unwrap();
    
    write!(&mut logger, "RUNNING MEMORY TESTS\n").unwrap();
    
    test_kmalloc(&mut logger);
    test_vmalloc(&mut logger);
    test_collections(&mut logger);

    write!(&mut logger, "TESTS COMPLETE!\n").unwrap();
    
    unsafe {
        let apic_phys = get_apic_base() as u64;
        let apic_virt = apic_phys + *HHDMOFFSET as u64;
        let mut pager = PAGER.lock();
        let flags = get_flags(true, true, false, true, true, false, false, false, true, true);
        pager.map_page(VirtAddress(apic_virt), apic_phys, flags, *HHDMOFFSET as u64, BlockSize::Normal).unwrap();
        drop(pager);
    }

    let lapic = Local_APIC::init();
    lapic.timer_setup(32, 0x0FFF_FFFF);

    hcf();
}
