pub mod context;
pub mod syscall;

pub fn set_kernel_stack(stack_top: usize) {
    let core_data = crate::arch::x86_64::cpu::core::get_core_data();

    core_data.core_gdt.tss.rsp[0] = stack_top as u64;
    core_data.kernel_rsp = stack_top;
}
