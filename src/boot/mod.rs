use limine::{
    BaseRevision,
    RequestsEndMarker,
    RequestsStartMarker,
    request::{
        FramebufferRequest,
        HhdmRequest,
        MemmapRequest,
        RsdpRequest,
    },
};

use crate::hcf;

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
#[unsafe(link_section = ".requests_start")]
static _START_MARKER: RequestsStartMarker = RequestsStartMarker::new();

#[used]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".requests_end")]
static _END_MARKER: RequestsEndMarker = RequestsEndMarker::new();

pub fn check() {
    if !BASE_REVISION.is_supported() {
        hcf();
    }
}
