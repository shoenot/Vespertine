use core::sync::atomic::Ordering;

pub(crate) mod hpet;
mod realtime;
pub(crate) mod tsc;

pub use realtime::read_rtc;

use crate::arch::x86_64::apic::lapic::*;
use crate::arch::x86_64::cpu::core::get_core_data;
use crate::arch::x86_64::cpuid::*;
use crate::arch::x86_64::timer::hpet::read_hpet_direct;
use crate::arch::x86_64::timer::tsc::read_tsc_direct;
use crate::core::acpi::hpet::get_hpet_base_addr;
use crate::core::time::{
    ClockSource,
    GET_TIME_FN,
    HPET_BASE_ADDR,
    LAPIC_FQ,
    TIME_SOURCE,
    TIME_SRC_FQ,
    TimeSource,
    USE_TSC_DEADLINE,
};
use crate::memory::PAGER;

pub(crate) type TimeFn = fn() -> usize;

fn uninit_time() -> usize { 0 }

pub fn init() {
    let use_tsc = has_invariant_tsc();

    let mut tsc_fq = if use_tsc { check_tsc_frequency().unwrap_or(0) } else { 0 };
    let mut lapic_fq = check_apic_frequency().unwrap_or(0);

    let need_calibration = (use_tsc && tsc_fq == 0) || lapic_fq == 0;
    let need_hpet = need_calibration || !use_tsc;

    let mut hpet_opt = None;
    if need_hpet {
        if let Some(addr) = get_hpet_base_addr() {
            HPET_BASE_ADDR.store(addr as *mut usize, Ordering::Relaxed);
            let mut pager = PAGER.lock();
            pager.map_mmio_addr((addr as u64) & !0xFFF).unwrap();
            let mut hpet = hpet::HPET { base_addr: 0, frequency: 0, enabled: false };
            hpet.init(addr);
            hpet.enable();
            hpet_opt = Some(hpet);
        } else if !use_tsc {
            panic!("FATAL: No invariant TSC and no HPET found.");
        }
    }

    let core_data = get_core_data();

    if need_calibration {
        let tsc = tsc::TSC { frequency: 0 };

        let start_tsc = if use_tsc && tsc_fq == 0 {
            unsafe {
                core::arch::asm!("lfence");
                tsc.read_counter()
            }
        } else {
            0
        };

        core_data.apic_mode.timer_setup(35, 0x0FFF_FFFF, TimerMode::OneShot);
        let start_lapic = core_data.apic_mode.current_count();

        if let Some(hpet) = &hpet_opt {
            let target = hpet.frequency / 100;
            let start = hpet.read_counter();
            while hpet.read_counter() - start < target {
                core::hint::spin_loop();
            }
        }

        let end_lapic = core_data.apic_mode.current_count();
        let end_tsc = if use_tsc && tsc_fq == 0 {
            unsafe {
                core::arch::asm!("lfence");
                tsc.read_counter()
            }
        } else {
            0
        };

        if lapic_fq == 0 {
            let ticks_in_10ms = (start_lapic.saturating_sub(end_lapic)) * 100;
            lapic_fq = ticks_in_10ms * 100 * 16;
        }
        if use_tsc && tsc_fq == 0 {
            tsc_fq = (end_tsc.saturating_sub(start_tsc)) * 100;
        }

        if use_tsc {
            if let Some(mut hpet) = hpet_opt.take() {
                hpet.disable();
            }
        }
    }

    core_data.apic_mode.stop_timer();

    if lapic_fq == 0 {
        panic!("FATAL: Failed to obtain LAPIC frequency.");
    }
    LAPIC_FQ.get_or_init(|| lapic_fq);

    if use_tsc {
        if tsc_fq == 0 {
            panic!("FATAL: Failed to obtain TSC frequency.");
        }
        let tsc = tsc::TSC { frequency: tsc_fq };
        TIME_SRC_FQ.get_or_init(|| tsc_fq);
        *TIME_SOURCE.lock() = TimeSource::TSC(tsc);
        GET_TIME_FN.store(read_tsc_direct as *mut (), Ordering::Relaxed);
    } else {
        let hpet = hpet_opt.expect("FATAL: Hardware requirements not met (Missing TSC and HPET)");
        TIME_SRC_FQ.get_or_init(|| hpet.frequency);
        *TIME_SOURCE.lock() = TimeSource::HPET(hpet);
        GET_TIME_FN.store(read_hpet_direct as *mut (), Ordering::Relaxed);
    }

    if has_tsc_deadline() {
        USE_TSC_DEADLINE.store(true, Ordering::Relaxed);
        core_data.apic_mode.timer_setup(35, 0, TimerMode::TscDeadline);
    } else {
        USE_TSC_DEADLINE.store(false, Ordering::Relaxed);
        core_data.apic_mode.timer_setup(35, 0, TimerMode::OneShot);
    }
}
