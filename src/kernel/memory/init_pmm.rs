use core::ops::Deref;
use lazy_static::lazy_static;
use limine::memmap::*;
use crate::{
    HHDM_REQUEST, MEMMAP_REQUEST, drivers::serial::{log_to_serial, log_u64_to_serial}
};

static PAGE_SIZE: usize = 4096;

lazy_static!(
    pub static ref HHDMOFFSET: usize = if let Some(hhdmresp) = HHDM_REQUEST.response() {
        hhdmresp.deref().offset as usize 
    } else { panic!("COULD NOT GET HHDM OFFSET FROM LIMINE") };
);

pub struct PhysFrame {
    start_addr: usize,
}

pub struct BitmapPMM {
    pub bitmap: &'static mut [u8],
    pub total_frames: usize,
    pub max_addr: usize,
}

impl BitmapPMM {
    pub fn init() -> Self {
        let mem_map = if let Some(memmap_response) = MEMMAP_REQUEST.response() {
            memmap_response.deref().entries()
        } else { panic!("COULD NOT GET MEMMAP FROM LIMINE") };

        let mut highest_addr: usize = 0;
        for entry in mem_map {
            let top = entry.base + entry.length;
            if top as usize > highest_addr { highest_addr = top as usize; }
        }

        let highest_addr = (highest_addr + 4095) & !4095;

        let total_frames = highest_addr / PAGE_SIZE;
        let bitmap_size_bytes = total_frames.div_ceil(8);

        let mut bitmap_phys_addr: usize = 0;
        for entry in mem_map {
            if entry.type_ == MEMMAP_USABLE && entry.length as usize >= bitmap_size_bytes {
                bitmap_phys_addr = entry.base as usize;
            }
        }

        assert!(bitmap_phys_addr != 0, "COULD NOT FIND PHYS MEMORY FOR BITMAP");

        let bitmap_virt_addr = bitmap_phys_addr + *HHDMOFFSET;

        let bitmap_slice = unsafe { core::slice::from_raw_parts_mut(bitmap_virt_addr as *mut u8, bitmap_size_bytes) };

        bitmap_slice.fill(0xFF);

        let mut pmm = BitmapPMM {
            bitmap: bitmap_slice,
            total_frames,
            max_addr: highest_addr,
        };

        for entry in mem_map {
            if entry.type_ == MEMMAP_USABLE {
                let start_frame = (entry.base as usize / PAGE_SIZE) as usize;
                let end_frame = ((entry.base + entry.length) as usize / PAGE_SIZE) as usize;
                for frame in start_frame..end_frame {
                    let phys_frame = PhysFrame{ start_addr: frame as usize * PAGE_SIZE };

                    // if the frame is the location of the bitmap, we don't free it
                    if phys_frame.start_addr >= bitmap_phys_addr && phys_frame.start_addr < bitmap_phys_addr + bitmap_size_bytes {
                        continue
                    }
                    pmm.set_free(frame);
                }
            }
        }

        log_to_serial("Init Bitmap stored at ");
        log_u64_to_serial(bitmap_virt_addr as u64);
        log_to_serial("\n");
        pmm
    }

    pub fn set_free(&mut self, frame_idx: usize) {
        let byte_idx = frame_idx / 8;
        let bit_idx = frame_idx % 8;
        self.bitmap[byte_idx] &= !(1 << bit_idx);
    }

    pub fn set_used(&mut self, frame_idx: usize) {
        let byte_idx = frame_idx / 8;
        let bit_idx = frame_idx % 8;
        self.bitmap[byte_idx] |= 1 << bit_idx;
    }

    pub fn is_free(&self, frame_idx: usize) -> bool {
        let byte_idx = frame_idx / 8;
        let bit_idx = frame_idx % 8;
        (self.bitmap[byte_idx] & (1 << bit_idx)) == 0
    }

    pub fn get_bitmap_addr(&self) -> usize {
        self.bitmap.as_ptr() as usize
    }

    pub fn alloc(&mut self, size: usize) -> Option<usize> {
        let frames_needed = size.div_ceil(PAGE_SIZE);

        let mut count = 0;
        let mut start_frame_idx = 0;
        
        for i in 0..self.total_frames {
            if self.is_free(i) {
                if count == 0 {
                    start_frame_idx = i;
                }
                count += 1;

                if count == frames_needed {
                    for j in start_frame_idx..(start_frame_idx + frames_needed as usize) {
                        self.set_used(j);
                    }

                    return Some(start_frame_idx);
                }
            } else {
                count = 0;
            }
        }
    
        None
    }
}
