unsafe extern "C" {
    static __extable_start: u8;
    static __extable_end: u8;
}

pub fn fixup_exception(frame: &mut super::idt::InterruptStackFrame) -> bool {
    let fault_rip = frame.instruction_pointer;
    
    let start_ptr = unsafe { &__extable_start as *const u8 as usize };
    let end_ptr = unsafe { &__extable_end as *const u8 as usize };
    
    let mut ptr = start_ptr;
    while ptr < end_ptr {
        let fault_addr = unsafe { *(ptr as *const usize) };
        let fixup_addr = unsafe { *((ptr + 8) as *const usize) };
        
        if fault_rip == fault_addr as u64 {
            frame.instruction_pointer = fixup_addr as u64;
            return true;
        }
        ptr += 16;
    }
    
    false
}
