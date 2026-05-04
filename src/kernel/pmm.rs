use core::ops::Deref;
use limine::memmap::*;
use crate::{
    HHDM_REQUEST, MEMMAP_REQUEST, drivers::{graphics::writenumber, serial::{log_to_serial, log_u64_to_serial}}
};

static PAGE_SIZE: u64 = 4096;

pub struct PhysFrame {
    start_addr: u64,
}

pub struct BitmapPMM {
    bitmap: &'static mut [u8],
    total_frames: usize,
    last_scanned_idx: usize,
}

impl BitmapPMM {
    pub fn init() -> Self {
        let mem_map = if let Some(memmap_response) = MEMMAP_REQUEST.response() {
            memmap_response.deref().entries()
        } else { panic!("COULD NOT GET MEMMAP FROM LIMINE") };

        let hhdm_offset = if let Some(hhdmresp) = HHDM_REQUEST.response() {
            hhdmresp.deref().offset
        } else { panic!("COULD NOT GET HHDM OFFSET FROM LIMINE") };

        let mut highest_addr = 0;
        for entry in mem_map {
            let top = entry.base + entry.length;
            if top > highest_addr { highest_addr = top; }
        }

        let total_frames = (highest_addr / PAGE_SIZE) as usize;
        let bitmap_size_bytes = total_frames.div_ceil(8);

        let mut bitmap_phys_addr = 0;
        for entry in mem_map {
            if entry.type_ == MEMMAP_USABLE && entry.length >= bitmap_size_bytes as u64 {
                bitmap_phys_addr = entry.base;
            }
        }

        assert!(bitmap_phys_addr != 0, "COULD NOT FIND PHYS MEMORY FOR BITMAP");

        let bitmap_virt_addr = bitmap_phys_addr + hhdm_offset;

        let bitmap_slice = unsafe { core::slice::from_raw_parts_mut(bitmap_virt_addr as *mut u8, bitmap_size_bytes) };

        bitmap_slice.fill(0xFF);

        let mut pmm = BitmapPMM {
            bitmap: bitmap_slice,
            total_frames,
            last_scanned_idx: 0
        };

        for entry in mem_map {
            if entry.type_ == MEMMAP_USABLE {
                let start_frame = (entry.base / PAGE_SIZE) as usize;
                let end_frame = ((entry.base + entry.length) / PAGE_SIZE) as usize;
                for frame in start_frame..end_frame {
                    let phys_frame = PhysFrame{ start_addr: frame as u64 * PAGE_SIZE };

                    // if the frame is the location of the bitmap, we don't free it
                    if phys_frame.start_addr >= bitmap_phys_addr && phys_frame.start_addr < bitmap_phys_addr + bitmap_size_bytes as u64 {
                        continue
                    }
                    pmm.set_free(frame);
                }
            }
        }

        log_to_serial("Physical Memory Bitmap stored at ");
        log_u64_to_serial(bitmap_virt_addr);
        pmm
    }

    pub fn set_free(&mut self, frame_idx: usize) {
        let byte_idx = frame_idx / 8;
        let bit_idx = byte_idx & 8;
        self.bitmap[byte_idx] &= !(1 << bit_idx);
    }

    pub fn get_bitmap_addr(&self) -> u64 {
        self.bitmap.as_ptr() as u64
    }
}
