use common::once::KernelOnceCell as OnceCell;
use crate::cpu::CpuLocalPtr;
use crate::x86_64::boot::{
    ApplicationProcessor,
    ApplicationProcessorEntry,
};

use limine::mp::{
    MpGotoFunction,
    MpInfo,
};
use limine::request::{
    FramebufferRequest,
    HhdmRequest,
    MemmapRequest,
    ModulesRequest,
    MpRequest,
    RsdpRequest,
};
use limine::{
    BaseRevision,
    RequestsEndMarker,
    RequestsStartMarker,
};

use crate::x86_64::boot::{
    BootFramebuffer,
    BootMemoryKind,
    BootMemoryRegion,
};

#[used]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".requests")]
pub static BASE_REVISION: BaseRevision = BaseRevision::with_revision(6 as u64);

#[used]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".requests")]
pub static FRAMEBUFFER_REQUEST: FramebufferRequest = FramebufferRequest::new();

#[used]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".requests")]
pub static MEMMAP_REQUEST: MemmapRequest = MemmapRequest::new();

#[used]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".requests")]
pub static HHDM_REQUEST: HhdmRequest = HhdmRequest::new();

#[used]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".requests")]
pub static RSDP_REQUEST: RsdpRequest = RsdpRequest::new();

#[used]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".requests")]
pub static MP_REQUEST: MpRequest = MpRequest::new(1);

#[used]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".requests")]
pub static MODULE_REQUEST: ModulesRequest = ModulesRequest::new();

#[used]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".requests_start")]
static _START_MARKER: RequestsStartMarker = RequestsStartMarker::new();

#[used]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".requests_end")]
static _END_MARKER: RequestsEndMarker = RequestsEndMarker::new();

static APPLICATION_PROCESSOR_ENTRY: OnceCell<ApplicationProcessorEntry> = OnceCell::new();

pub fn check() -> bool {
    BASE_REVISION.is_supported()
}

pub fn direct_map_offset() -> usize {
    HHDM_REQUEST.response()
        .expect("failed to get direct map offset from bootloader")
        .offset as usize
}

pub fn acpi_rsdp_addr() -> usize {
    RSDP_REQUEST.response()
        .expect("failed to get RSDP address from bootloader")
        .address as usize
}

extern "C" fn ap_trampoline(mp_info: &MpInfo) -> ! {
    let cpu_local = mp_info.extra_argument() as CpuLocalPtr;
    let entry = APPLICATION_PROCESSOR_ENTRY.get().expect("application processor entry was not installed");
    entry(cpu_local)
}

pub(crate) fn for_each_ap(mut f: impl FnMut(ApplicationProcessor)) {
    let mp_response = MP_REQUEST.response().expect("failed to get MP response from bootloader");
    let bsp_id = mp_response.bsp_lapic_id;
    for core in mp_response.cpus() {
        if core.lapic_id == bsp_id { continue; }
        f(ApplicationProcessor { hardware_id: core.lapic_id as usize });
    }
}

pub(crate) fn start_ap(processor: ApplicationProcessor, entry: ApplicationProcessorEntry, cpu_local: CpuLocalPtr) {
    APPLICATION_PROCESSOR_ENTRY.get_or_init(|| entry);
    let mp_response = MP_REQUEST.response().expect("failed to get MP response from bootloader");
    for core in mp_response.cpus() {
        if core.lapic_id as usize == processor.hardware_id {
            let trampoline = ap_trampoline as MpGotoFunction;
            core.bootstrap(trampoline, cpu_local as u64);
            return;
        }
    }
    panic!("failed to find application processor with hardware id {}", processor.hardware_id);
}

pub fn framebuffer() -> Option<BootFramebuffer> {
    let fb_response = FRAMEBUFFER_REQUEST.response()?;
    let fb = *fb_response.framebuffers().first()?;

    let virtual_address = fb.address() as usize;
    let physical_address = virtual_address - direct_map_offset();

    Some(BootFramebuffer {
        virtual_address,
        physical_address,
        width: fb.width as usize,
        height: fb.height as usize,
        pitch: fb.pitch as usize,
        bpp: fb.bpp as usize,
    })
}

fn convert_memory_kind(kind: u64) -> BootMemoryKind {
    match kind {
        limine::memmap::MEMMAP_USABLE => BootMemoryKind::Usable,
        limine::memmap::MEMMAP_BOOTLOADER_RECLAIMABLE => BootMemoryKind::BootloaderReclaimable,
        limine::memmap::MEMMAP_EXECUTABLE_AND_MODULES => BootMemoryKind::ExecutableAndModules,
        limine::memmap::MEMMAP_RESERVED => BootMemoryKind::Reserved,
        _ => BootMemoryKind::Other,
    }
}

pub fn for_each_memory_region(mut f: impl FnMut(BootMemoryRegion)) {
    let memmap_response = MEMMAP_REQUEST.response().expect("failed to get memory map from bootloader");

    for entry in memmap_response.entries() {
        f(BootMemoryRegion {
            base: entry.base,
            length: entry.length,
            kind: convert_memory_kind(entry.type_),
        });
    }
}

