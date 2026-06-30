#![no_std]

#[cfg(target_arch = "x86_64")]
mod x86_64;

#[cfg(target_arch = "x86_64")]
pub mod cpu {
    pub use crate::x86_64::cpu::{
        activate_core,
        current_kernel_core,
        current_logical_id,
        current_hardware_id,
        set_kernel_stack,
        set_user_thread_pointer,
        kernel_core_from_cpu_local,
        idle,
        halt,
        halt_loop,
        init_bootstrap_cpu_local,
        init_cpu_local_with_hardware_id,
        init_ap_state,
        init_bsp_state,
        BootAllocFn,
        CpuLocalPtr,
        KernelCorePtr,
    };
}

#[cfg(target_arch = "x86_64")]
pub mod smp {
    pub use crate::x86_64::smp::{
        ApplicationCoreEntry,
        KernelCoreAllocator,
        KernelCoreRegistrar,
        start_application_cores,
    };
}

#[cfg(target_arch = "x86_64")]
pub mod ipi {
    pub use crate::x86_64::apic::lapic::{
        send_reschedule_ipi,
        send_tlb_shootdown_ipi,
    };
}

#[cfg(target_arch = "x86_64")]
pub mod context {
    pub use crate::x86_64::task::context::{
        SyscallFrame,
        init_bootstrap_thread_stack,
        allocate_bootstrap_extended_context,
        deallocate_extended_context,
        init_thread_stack,
        switch_from_bootstrap,
        switch_threads,
        ContextError,
    };
}

#[cfg(target_arch = "x86_64")]
pub mod boot {
    pub use crate::x86_64::boot::{
        BootFramebuffer,
        BootMemoryKind,
        BootMemoryRegion,
        check,
        direct_map_offset,
        framebuffer,
        for_each_memory_region,
    };
}

#[cfg(target_arch = "x86_64")]
pub mod platform {
    pub use crate::x86_64::platform::{
        init,
        init_early,
        PlatformInit,
    };
}

#[cfg(target_arch = "x86_64")]
pub mod timer {
    pub use crate::x86_64::timer::{
        init,
        init_local,
        read_counter,
        counter_frequency,
        ns_to_ticks,
        arm_relative_ticks,
        arm_relative_ns,
        stop,
        read_rtc,
    };
}

#[cfg(target_arch = "x86_64")]
pub mod interrupts {
    pub use crate::x86_64::interrupts::{
        init,
        enable_interrupts,
        disable_interrupts,
        interrupts_enabled,
        TrapFrame,
        load_local,
        RESCHEDULE_IPI_VECTOR,
        TLB_SHOOTDOWN_IPI_VECTOR,
        page_fault_address,
        TIMER_VECTOR,
    };
    pub use crate::x86_64::apic::lapic::compose_msi_message;
    pub use crate::x86_64::apic::ioapic::{
        init_ioapic as init_platform_interrupts,
        route_isa_irq,
    };
}

#[cfg(target_arch = "x86_64")]
pub mod mmu {
    pub use crate::x86_64::mmu::*;
}

#[cfg(target_arch = "x86_64")]
pub mod io {
    pub use crate::x86_64::io::*;
}

#[cfg(target_arch = "x86_64")]
pub mod usercopy {
    pub use crate::x86_64::usercopy::*;
}
