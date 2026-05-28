use core::{hint::spin_loop, pin::Pin, ptr::{addr_of, addr_of_mut, read_volatile, write_bytes, write_volatile}, task::{Context, Poll, Waker}};

use alloc::vec::Vec;
use vespertine_common::lock::TicketLock;

use crate::{drivers::virtio::mmio::{VirtioBlockDriver, init_virtio}, memory::{ALLOCATOR, BlockSize, HHDMOFFSET}, util::bitwise::{set_bit, unset_bit}};

#[repr(C, packed)]
pub struct VqDescriptor {
    pub addr: u64,
    pub len: u32,
    pub flags: u16,
    pub next: u16,
}


#[repr(C, packed)]
pub struct VqAvailableRing {
    pub flags: *mut u16,
    pub idx: *mut u16,
    pub ring: *mut u16,
    pub _used_event: *mut u16,
}

#[repr(C, packed)]
pub struct VqUsedRing {
    pub flags: *mut u16,
    pub idx: *mut u16,
    pub ring: *mut VqUsedElem,
    pub _avail_event: *mut u16,
}

#[repr(C, packed)]
pub struct VqUsedElem {
    id: u32,
    len: u32,
}

#[repr(C)]
pub struct Virtqueue {
    pub desc: *mut VqDescriptor,
    pub available: VqAvailableRing,
    pub used: VqUsedRing,

    // phys addrs for cleanup and tracking
    pub desc_phys: usize,
    pub av_phys: usize,
    pub used_phys: usize,

    // alloc orders for deallocation
    pub desc_order: usize,
    pub av_order: usize,
    pub used_order: usize,

    pub queue_size: u16,
    pub free_head: u16,
    pub last_seen_used: u16,
    pub queue_notify_off: u16,

    pub wakers: TicketLock<Vec<Option<Waker>>>
}


impl Drop for Virtqueue {
    fn drop(&mut self) {
        // auto release when dropped
        let allocator = &crate::memory::ALLOCATOR;
        allocator.free_order(self.desc_phys, self.desc_order);
        allocator.free_order(self.av_phys, self.av_order);
        allocator.free_order(self.used_phys, self.used_order);
    }
}

pub struct VirtioBlockDevice {
    driver: VirtioBlockDriver,
    virtqueue: Virtqueue,
}

fn calculate_order(bytes: usize) -> usize {
    let mut order = 0;
    while (1 << order) * 4096 < bytes {
        order += 1;
    }
    order
}

pub fn perform_handshake(drv: &VirtioBlockDriver) {
    unsafe {
        let cfg = &mut *drv.common_cfg;
        let status_ptr = addr_of_mut!(cfg.device_status) as *mut u8;
        write_volatile(status_ptr, 0);
        let mut scratch = read_volatile(status_ptr);
        write_volatile(status_ptr, scratch | 1); // acknowledgement
        scratch = read_volatile(status_ptr);
        write_volatile(status_ptr, scratch | 2); // driver
    }
}

pub fn negotiate_features(drv: &VirtioBlockDriver) {
    unsafe {
        let cfg = &mut *drv.common_cfg;
        let dev_feat_sel_ptr = addr_of_mut!(cfg.dev_feature_select) as *mut u32;
        let dev_feat_ptr = addr_of_mut!(cfg.dev_feature) as *mut u32;
        write_volatile(dev_feat_sel_ptr, 0);
        let mut lower = read_volatile(dev_feat_ptr);
        write_volatile(dev_feat_sel_ptr, 1);
        let mut upper = read_volatile(dev_feat_ptr);
        upper = set_bit(upper, 0);         // VIRTIO_F_VERSION_1
        upper = unset_bit(upper, 2);       // VIRTIO_F_RING_PACKED
        lower = unset_bit(lower, 12);      // VIRTIO_BLK_F_MQ
        lower = unset_bit(lower, 28);      // VIRTIO_F_INDIRECT_DESC
        lower = unset_bit(lower, 29);      // VIRTIO_F_EVENT_IDX
        lower = set_bit(lower, 1);         // VIRTIO_BLK_F_SIZE_MAX 
        lower = set_bit(lower, 2);         // VIRTIO_BLK_F_SEG_MAX 
        lower = set_bit(lower, 5);         // VIRTIO_BLK_F_RO 
        lower = set_bit(lower, 6);         // VIRTIO_BLK_F_BLK_SIZE 
        lower = set_bit(lower, 9);         // VIRTIO_BLK_F_FLUSH 
        let driv_feat_sel_ptr = addr_of_mut!(cfg.driv_feature_select) as *mut u32;
        let driv_feat_ptr = addr_of_mut!(cfg.driv_feature) as *mut u32;
        write_volatile(driv_feat_sel_ptr, 0);
        write_volatile(driv_feat_ptr, lower);
        write_volatile(driv_feat_sel_ptr, 1);
        write_volatile(driv_feat_ptr, upper);
    }
}

pub fn vq_setup(drv: &VirtioBlockDriver, q_idx: u16) -> Option<Virtqueue> {
    unsafe {
        let cfg = &mut *drv.common_cfg;
        let q_sel_ptr = addr_of_mut!(cfg.queue_select) as *mut u16;
        write_volatile(q_sel_ptr, q_idx);
        let q_size_ptr = addr_of_mut!(cfg.queue_size) as *mut u16;
        let q_size = read_volatile(q_size_ptr);
        if q_size == 0 {
            return None;
        }

        let desc_bytes = q_size as usize * 16;
        let av_bytes = 6 + (q_size as usize * 2);
        let used_bytes = 6 + (q_size as usize * 8);

        let desc_order = calculate_order(desc_bytes);
        let av_order = calculate_order(av_bytes);
        let used_order = calculate_order(used_bytes);

        let desc_phys = ALLOCATOR.alloc_order(desc_order)?;
        let av_phys = ALLOCATOR.alloc_order(av_order)?;
        let used_phys = ALLOCATOR.alloc_order(used_order)?;

        let hhdm = *HHDMOFFSET;
        let desc_virt = (desc_phys + hhdm) as *mut VqDescriptor;
        let av_virt_base = av_phys + hhdm;
        let used_virt_base = used_phys + hhdm;

        write_bytes(desc_virt as *mut u8, 0, (1 << desc_order) * 4096);
        write_bytes(av_virt_base as *mut u8, 0, (1 << av_order) * 4096);
        write_bytes(used_virt_base as *mut u8, 0, (1 << used_order) * 4096);

        // make a free list of descriptors
        for i in 0..(q_size-1) {
            let desc_ptr = desc_virt.add(i as usize);
            let next_desc = VqDescriptor {
                addr: 0,
                len: 0,
                flags: 1,
                next: i + 1,
            };
            write_volatile(desc_ptr, next_desc);
        }

        // last descriptor needs to terminate chain with 0 flag and 0xffff next
        let last_desc_ptr = desc_virt.add((q_size - 1) as usize);
        let last_desc = VqDescriptor {
            addr: 0,
            len: 0,
            flags: 0,
            next: 0xFFFF,
        };
        write_volatile(last_desc_ptr, last_desc);

        let av_flags = av_virt_base as *mut u16;
        let av_idx = (av_virt_base + 2) as *mut u16;
        let av_ring = (av_virt_base + 4) as *mut u16;
        let _used_event = (av_virt_base + 4 + (q_size as usize * 2)) as *mut u16;

        let available = VqAvailableRing {
            flags: av_flags,
            idx: av_idx,
            ring: av_ring,
            _used_event,
        };

        let used_flags = used_virt_base as *mut u16;
        let used_idx = (used_virt_base + 2) as *mut u16;
        let used_ring = (used_virt_base + 4) as *mut VqUsedElem;
        let _avail_event = (used_virt_base + 4 + (q_size as usize * 8)) as *mut u16;

        let used = VqUsedRing {
            flags: used_flags,
            idx: used_idx,
            ring: used_ring,
            _avail_event,
        };

        let queue_desc_ptr = addr_of_mut!(cfg.queue_desc) as *mut u64;
        let queue_driver_ptr = addr_of_mut!(cfg.queue_driver) as *mut u64;
        let queue_device_ptr = addr_of_mut!(cfg.queue_device) as *mut u64;

        write_volatile(queue_desc_ptr, desc_phys as u64);
        write_volatile(queue_driver_ptr, av_phys as u64);
        write_volatile(queue_device_ptr, used_phys as u64);

        let queue_enable_ptr = addr_of_mut!(cfg.queue_enable) as *mut u16;
        write_volatile(queue_enable_ptr, 1);

        let notify_off_ptr = addr_of_mut!(cfg.queue_notify_off) as *mut u16;
        let queue_notify_off = read_volatile(notify_off_ptr);

        let mut wakers = Vec::new();
        wakers.resize(q_size as usize, None);
        let wakers = TicketLock::new(wakers);

        Some(Virtqueue {
            desc: desc_virt,
            available,
            used,
            desc_phys,
            av_phys,
            used_phys,
            desc_order,
            av_order,
            used_order,
            queue_size: q_size,
            free_head: 0,
            last_seen_used: 0,
            queue_notify_off,
            wakers,
        })
    }
}

pub fn init_block_device() -> Option<VirtioBlockDevice> {
	unsafe {
        let drv = init_virtio()?;
        let cfg = &mut *drv.common_cfg;
        let status_ptr = addr_of_mut!(cfg.device_status) as *mut u8;

        perform_handshake(&drv);
        negotiate_features(&drv);

        let status = read_volatile(status_ptr);
        write_volatile(status_ptr, status | 8); // write FEATURES_OK
        let verify = read_volatile(status_ptr);
        if (verify & 8) == 0 {
            return None;
        }

        let virtqueue = vq_setup(&drv, 0)?;

        let status = read_volatile(status_ptr);
        write_volatile(status_ptr, status | 4); // write DRIVER_OK

        Some(VirtioBlockDevice {
            driver: drv,
            virtqueue
        })
	}
}

impl Virtqueue {
    pub fn alloc_desc(&mut self) -> Result<usize, ()> {
        let brw_idx = self.free_head;
        if brw_idx == 0xFFFF { return Err(()) };
        unsafe {
            let brw = self.desc.add(brw_idx as usize);
            self.free_head = (*brw).next;
            Ok(brw_idx as usize)
        }
    }

    pub fn free_desc(&mut self, idx: u16) {
        unsafe {
            let brw = self.desc.add(idx as usize);
            (*brw).next = self.free_head;
            self.free_head = idx;
        }
    }
}

pub const VIRTIO_BLK_T_IN: u32 = 0;     // READ
pub const VIRTIO_BLK_T_OUT: u32 = 1;    // WRITE
pub const VIRTIO_BLK_T_FLUSH: u32 = 4;    // FLUSH

pub struct VirtioBlkReqHeader {
    pub req_type: u32,
    pub reserved: u32,
    pub sector: u64,
}

impl VirtioBlockDevice {
    pub fn read_sectors(&mut self, sector: u64, sectors_count: u32, buf_phys: u64) -> Result<(), ()> {
        self.transfer_sectors(sector, sectors_count, buf_phys, false)
    }

    pub fn write_sectors(&mut self, sector: u64, sectors_count: u32, buf_phys: u64) -> Result<(), ()> {
        self.transfer_sectors(sector, sectors_count, buf_phys, true)
    }

    pub fn transfer_sectors(&mut self, sector: u64, sectors_count: u32, buf_phys: u64, is_write: bool) -> Result<(), ()> {
        let drv = &self.driver;
        let vq = &mut self.virtqueue;

        unsafe {
            let page_phys = ALLOCATOR.alloc(BlockSize::Normal);
            if page_phys == 0 { return Err(()) };
            let page_virt = page_phys + *HHDMOFFSET;

            write_bytes(page_virt as *mut u8, 0, 4096);

            let req_hdr = VirtioBlkReqHeader {
                req_type: if is_write { VIRTIO_BLK_T_OUT } else { VIRTIO_BLK_T_IN },
                reserved: 0,
                sector,
            };
            let hdr_ptr = page_virt as *mut VirtioBlkReqHeader;
            write_volatile(hdr_ptr, req_hdr);

            let status_ptr = (page_virt + 512) as *mut u8;
            write_volatile(status_ptr, 0xFF);

            let d0 = vq.alloc_desc()? as u16;
            let d1 = vq.alloc_desc()? as u16;
            let d2 = vq.alloc_desc()? as u16;

            // chain desc 0 - header
            let desc0 = vq.desc.add(d0 as usize);
            write_volatile(desc0, VqDescriptor {
                addr: page_phys as u64,
                len: 16,
                flags: 1, // next flag
                next: d1,
            });

            // chain desc 1 - data buffer
            let desc1 = vq.desc.add(d1 as usize);
            write_volatile(desc1, VqDescriptor {
                addr: buf_phys as u64,
                len: sectors_count * 512,
                flags: if is_write { 1 } else { 3 },
                next: d2,
            });

            // chain desc 2 - status byte
            let desc2 = vq.desc.add(d2 as usize);
            write_volatile(desc2, VqDescriptor {
                addr: (page_phys + 512) as u64,
                len: 1,
                flags: 2,
                next: 0xFFFF, 
            });

            let avail_idx_ptr = vq.available.idx;
            let idx = read_volatile(avail_idx_ptr);
            let slot = (idx as usize) % (vq.queue_size as usize);
            let ring_slot_ptr = vq.available.ring.add(slot);
            write_volatile(ring_slot_ptr, d0);

            write_volatile(avail_idx_ptr, idx.wrapping_add(1));

            let doorbell_offset = vq.queue_notify_off as usize * drv.notify_off_multiplier as usize;
            let doorbell_ptr = (drv.notify_base as usize + doorbell_offset) as *mut u16;
            write_volatile(doorbell_ptr, 0);

            let used_idx_ptr = vq.used.idx;
            let last_seen = vq.last_seen_used;

            while read_volatile(used_idx_ptr) == last_seen {
                spin_loop();
            }

            let status = read_volatile(status_ptr);

            vq.free_desc(d0);
            vq.free_desc(d1);
            vq.free_desc(d2);

            vq.last_seen_used = vq.last_seen_used.wrapping_add(1);

            ALLOCATOR.free(page_phys, BlockSize::Normal);

            if status == 0 {
                Ok(())
            } else {
                Err(())
            }
        }
    }

    pub fn read_sectors_async(&mut self, sector: u64, sectors_count: u32, buf_phys: u64) -> Result<BlockTransferFuture, ()> {
        self.transfer_sectors_async(sector, sectors_count, buf_phys, false)
    }

    pub fn write_sectors_async(&mut self, sector: u64, sectors_count: u32, buf_phys: u64) -> Result<BlockTransferFuture, ()> {
        self.transfer_sectors_async(sector, sectors_count, buf_phys, true)
    }

    pub fn transfer_sectors_async(&mut self, sector: u64, sectors_count: u32, buf_phys: u64, is_write: bool) -> Result<BlockTransferFuture, ()> {
        let drv = &self.driver;
        let vq = &mut self.virtqueue;

        unsafe {
            let page_phys = ALLOCATOR.alloc(BlockSize::Normal);
            if page_phys == 0 { return Err(()) };
            let page_virt = page_phys + *HHDMOFFSET;

            write_bytes(page_virt as *mut u8, 0, 4096);

            let req_hdr = VirtioBlkReqHeader {
                req_type: if is_write { VIRTIO_BLK_T_OUT } else { VIRTIO_BLK_T_IN },
                reserved: 0,
                sector,
            };
            let hdr_ptr = page_virt as *mut VirtioBlkReqHeader;
            write_volatile(hdr_ptr, req_hdr);

            let status_ptr = (page_virt + 512) as *mut u8;
            write_volatile(status_ptr, 0xFF);

            let d0 = vq.alloc_desc()? as u16;
            let d1 = vq.alloc_desc()? as u16;
            let d2 = vq.alloc_desc()? as u16;

            // chain desc 0 - header
            let desc0 = vq.desc.add(d0 as usize);
            write_volatile(desc0, VqDescriptor {
                addr: page_phys as u64,
                len: 16,
                flags: 1, // next flag
                next: d1,
            });

            // chain desc 1 - data buffer
            let desc1 = vq.desc.add(d1 as usize);
            write_volatile(desc1, VqDescriptor {
                addr: buf_phys as u64,
                len: sectors_count * 512,
                flags: if is_write { 1 } else { 3 },
                next: d2,
            });

            // chain desc 2 - status byte
            let desc2 = vq.desc.add(d2 as usize);
            write_volatile(desc2, VqDescriptor {
                addr: (page_phys + 512) as u64,
                len: 1,
                flags: 2,
                next: 0xFFFF, 
            });

            let avail_idx_ptr = vq.available.idx;
            let idx = read_volatile(avail_idx_ptr);
            let slot = (idx as usize) % (vq.queue_size as usize);
            let ring_slot_ptr = vq.available.ring.add(slot);
            write_volatile(ring_slot_ptr, d0);

            write_volatile(avail_idx_ptr, idx.wrapping_add(1));

            let doorbell_offset = vq.queue_notify_off as usize * drv.notify_off_multiplier as usize;
            let doorbell_ptr = (drv.notify_base as usize + doorbell_offset) as *mut u16;
            write_volatile(doorbell_ptr, 0);

            let last_seen = vq.last_seen_used;

            Ok(BlockTransferFuture {
                d0,
                d1,
                d2,
                page_phys,
                last_seen_used: last_seen,
                vq: vq as *mut Virtqueue,
            })
        }
    }
}

pub struct BlockTransferFuture {
    pub d0: u16,
    pub d1: u16,
    pub d2: u16,
    pub page_phys: usize,
    pub last_seen_used: u16,
    pub vq: *mut Virtqueue,
}

unsafe impl Send for BlockTransferFuture {}

impl Future for BlockTransferFuture {
    type Output = Result<(), ()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let vq = unsafe { &mut *self.vq };

        unsafe {
            let status_ptr = (self.page_phys + 512 + *HHDMOFFSET) as *const u8;
            let status = read_volatile(status_ptr);

            if status != 0xFF {
                // the transfer is complete
                vq.free_desc(self.d0);
                vq.free_desc(self.d1);
                vq.free_desc(self.d2);

                ALLOCATOR.free(self.page_phys, BlockSize::Normal);

                if status == 0 {
                    Poll::Ready(Ok(()))
                } else {
                    Poll::Ready(Err(()))
                }
            } else {
                // register this tasks waker so it gets woken up when io completes
                let waker = cx.waker().clone();
                vq.wakers.lock()[self.d0 as usize] = Some(waker);
                Poll::Pending
            }
        }
    }
}

pub extern "C" fn virtio_blk_poll_thread(arg: usize) -> ! {
    let blk = arg as *mut VirtioBlockDevice;
    unsafe {
        let vq = &mut (*blk).virtqueue;
        let mut last_seen = vq.last_seen_used;

        loop {
            let current_used = read_volatile(vq.used.idx);
            if current_used != last_seen {
                while last_seen != current_used {
                    let slot = (last_seen as usize) % (vq.queue_size as usize);
                    let elem_ptr = vq.used.ring.add(slot);
                    let desc_id = read_volatile(addr_of!((*elem_ptr).id)) as usize;

                    let waker_opt = {
                        let mut wakers = vq.wakers.lock();
                        if desc_id < wakers.len() {
                            wakers[desc_id].take()
                        } else {
                            None
                        }
                    };

                    if let Some(waker) = waker_opt {
                        waker.wake();
                    }

                    last_seen = last_seen.wrapping_add(1);
                }
                vq.last_seen_used = last_seen;
            }
            spin_loop();
        }
    }
}
