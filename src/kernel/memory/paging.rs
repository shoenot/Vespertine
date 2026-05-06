use core::arch::asm;

use crate::kernel::lock::TicketLock;
use crate::kernel::memory::pmm::*;

type PhysAlloc = TicketLock<Allocator>;

// structs 

#[repr(C, align(4096))]
pub struct PageTable {
    pub entries: [PageTableEntry; 512],
}

#[repr(transparent)]
#[derive(Copy, Clone)]
pub struct PageTableEntry(u64);

#[repr(transparent)]
#[derive(Copy, Clone)]
pub struct VirtAddress(pub u64);

pub struct Pager {
    active_l4_addr: u64,
    allocator: &'static PhysAlloc 
}

// helper functions

fn next_table(entry: &PageTableEntry, phys_offset: u64) -> Option<*const PageTable> {
    if !entry.is_present() { return None } 
    Some((entry.get_addr() + phys_offset) as *const PageTable)
}

fn get_or_create_next(entry: &mut PageTableEntry, phys_offset: u64, allocator: &'static PhysAlloc) -> Option<*mut PageTable> {
    if !entry.is_present() {
        let new_frame_phys = { allocator.lock().alloc(BlockSize::Normal)? as u64 };

        let new_table_virt = (new_frame_phys + phys_offset) as *mut PageTable;
        unsafe { (*new_table_virt).zero(); }
        
        // flags: most_accessible. we change it for the last table manually.
        *entry = PageTableEntry::new(new_frame_phys, 0x7);
    }
    Some((entry.get_addr() + phys_offset) as *mut PageTable)
}

pub fn get_flags(
    present: bool, writable: bool, 
    user_access: bool, writethru: bool, 
    no_cache: bool, accessed: bool, 
    dirty: bool, huge: bool,
    global: bool, no_execute: bool) -> u64 {
    let mut flags: u64 = 0;
    if present { flags |= 1 << 0 }
    if writable { flags |= 1 << 1 }
    if user_access { flags |= 1 << 2 }
    if writethru { flags |= 1 << 3 }
    if no_cache { flags |= 1 << 4 }
    if accessed { flags |= 1 << 5 }
    if dirty { flags |= 1 << 6 }
    if huge { flags |= 1 << 7 }
    if global { flags |= 1 << 8 }
    if no_execute { flags |= 1 << 63 }
    flags
}

fn get_cr3() -> u64 {
    let cr3: u64;
    unsafe {
        asm!(
            "movq %cr3, {0}", 
            out(reg) cr3, 
            options(att_syntax, nostack, preserves_flags)
        );
    };
    cr3
}

fn load_cr3(addr: u64) {
    unsafe {
        asm!(
            "movq {0}, %cr3",
            in(reg) addr, 
            options(att_syntax, nostack, preserves_flags)
        );
    };
}


pub fn flush_tlb(virt: u64) {
    unsafe {
        asm!("invlpg ({0})", in(reg) virt, options(att_syntax, nostack, preserves_flags))
    }
}

// struct methods 

impl PageTableEntry {
    pub fn new(phys_addr: u64, flags: u64) -> Self {
        Self((phys_addr & 0x000F_FFFF_FFFF_F000) | flags)
    }

    pub fn is_unused(&self) -> bool {
        self.0 == 0
    }

    pub fn set_unused(&mut self) {
        self.0 = 0;
    }

    pub fn get_addr(&self) -> u64 {
        self.0 & 0x000F_FFFF_FFFF_F000
    }

    pub fn is_present(&self) -> bool {
        self.0 & 1 == 1
    }

    pub fn is_huge(&self) -> bool {
        self.0 & (1 << 7) != 0
    }
}


impl PageTable {
    pub fn zero(&mut self) {
        for entry in self.entries.iter_mut() {
            entry.set_unused();
        }
    }

    pub unsafe fn translate(&self, addr: VirtAddress, physical_offset: u64) -> Option<u64> {
        let (l4, l3, l2, l1, offset) = addr.get_idxs();

        unsafe {
            let l3_table = next_table(&self.entries[l4 as usize], physical_offset)?;
            let l2_table = next_table(&(*l3_table).entries[l3 as usize], physical_offset)?;
            let l2_entry = &(*l2_table).entries[l2 as usize];

            if !l2_entry.is_present() { return None };

            if l2_entry.is_huge() { 
                return Some(l2_entry.get_addr() + addr.get_huge_offset());
            }

            let l1_table = next_table(l2_entry, physical_offset)?;
            let final_entry = &(*l1_table).entries[l1 as usize];
            if !final_entry.is_present() { return None; }

            Some(final_entry.get_addr() + offset as u64)
        }
    }

    pub unsafe fn map_page(&mut self, virt: VirtAddress, phys: u64, flags: u64, allocator: &'static PhysAlloc, phys_offset: u64) -> Option<()> {
        let (l4, l3, l2, l1, _) = virt.get_idxs();
    
        unsafe {
            let l3_table = get_or_create_next(&mut self.entries[l4 as usize], phys_offset, allocator)?;
            let l2_table = get_or_create_next(&mut (*l3_table).entries[l3 as usize], phys_offset, allocator)?;
            let l1_table = get_or_create_next(&mut (*l2_table).entries[l2 as usize], phys_offset, allocator)?;

            (*l1_table).entries[l1 as usize] = PageTableEntry::new(phys, flags);
        }
        Some(())
    }

    pub unsafe fn map_huge_page(&mut self, virt: VirtAddress, phys: u64, flags: u64, allocator: &'static PhysAlloc, phys_offset: u64) -> Option<()> {
        let (l4, l3, l2, _, _) = virt.get_idxs();
    
        unsafe {
            let l3_table = get_or_create_next(&mut self.entries[l4 as usize], phys_offset, allocator)?;
            let l2_table = get_or_create_next(&mut (*l3_table).entries[l3 as usize], phys_offset, allocator)?;

            let huge_flags = flags | (1 << 7);

            (*l2_table).entries[l2 as usize] = PageTableEntry::new(phys, huge_flags);
        }
        Some(())
    }
}

impl VirtAddress {
    pub fn new(l4: u64, l3: u64, l2: u64, l1: u64, offset: u64) -> Self {
        let mut addr: u64 = 0;
        addr |= (l4 & 0o777) << 39;
        addr |= (l3 & 0o777) << 30;
        addr |= (l2 & 0o777) << 21;
        addr |= (l1 & 0o777) << 12;
        addr |= offset & 0o7777;
        if (addr & (1 << 47)) != 0 {
            addr |= 0xffff_0000_0000_0000;
        }
        VirtAddress(addr)
    }

    pub fn get_idxs(&self) -> (u64, u64, u64, u64, u64) {
        let l4 = (self.0 >> 39) & 0o777;
        let l3 = (self.0 >> 30) & 0o777;
        let l2 = (self.0 >> 21) & 0o777;
        let l1 = (self.0 >> 12) & 0o777;
        let offset = self.0 & 0o7777;
        (l4, l3, l2, l1, offset)
    }

    pub fn get_huge_offset(&self) -> u64 {
        self.0 & 0x1F_FFFF
    }
}

impl Pager {
    pub fn init(allocator: &'static PhysAlloc) -> Option<Self> {
        let pml4_table_frame = { allocator.lock().alloc(BlockSize::Normal)? as u64 };

        let new_pml4_table = unsafe { &mut *((pml4_table_frame + *HHDMOFFSET as u64) as *mut PageTable) };
        new_pml4_table.zero();

        let old_pml4_table_addr = get_cr3() & 0x000F_FFFF_FFFF_F000;
        let old_pml4_table = unsafe { &*((old_pml4_table_addr + *HHDMOFFSET as u64) as *const PageTable) };

        for idx in 256..512 {
            new_pml4_table.entries[idx] = old_pml4_table.entries[idx];
        }

        load_cr3(pml4_table_frame);

        Some(Pager { active_l4_addr: pml4_table_frame, allocator })
    }

    pub fn map_page(&mut self, virt:VirtAddress, phys: u64, flags: u64, phys_offset: u64) -> Option<()> {
        unsafe {
            let active_table = &mut *((self.active_l4_addr + phys_offset) as *mut PageTable);
            active_table.map_page(virt, phys, flags, self.allocator, phys_offset)
        }
    }

    pub fn map_huge_page(&mut self, virt:VirtAddress, phys: u64, flags: u64, phys_offset: u64) -> Option<()> {
        assert!(phys & 0x1F_FFFF == 0, "Phys address not 2M aligned");
        assert!(virt.get_huge_offset() == 0, "Virt address not 2M aligned");
        unsafe {
            let active_table = &mut *((self.active_l4_addr + phys_offset) as *mut PageTable);
            active_table.map_huge_page(virt, phys, flags, self.allocator, phys_offset)
        }
    }

}

