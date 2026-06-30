use core::sync::atomic::{
    AtomicBool,
    AtomicUsize,
    Ordering,
};

use common::once::KernelOnceCell as OnceCell;

mod hpet;
mod realtime;
mod tsc;

pub use realtime::read_rtc;

use crate::x86_64::apic::lapic::{
    LocalApicDriver,
    TimerMode,
    arm_local_timer_oneshot,
    current_local_apic,
    setup_local_timer,
    stop_local_timer,
};
use crate::x86_64::cpu::cpuid::{
    check_apic_frequency,
    check_tsc_frequency,
    has_invariant_tsc,
    has_tsc_deadline,
};
use crate::x86_64::msr::write_to_msr;
use crate::x86_64::platform;

use hpet::{
    Hpet,
    read_hpet_counter,
};
use tsc::{
    Tsc,
    read_tsc_counter,
};

const IA32_TSC_DEADLINE: u32 = 0x6E0;

const TIME_SOURCE_NONE: usize = 0;
const TIME_SOURCE_TSC: usize = 1;
const TIME_SOURCE_HPET: usize = 2;

static TIME_SOURCE: AtomicUsize = AtomicUsize::new(TIME_SOURCE_NONE);
static TIME_SRC_FQ: OnceCell<usize> = OnceCell::new();
static LAPIC_FQ: OnceCell<usize> = OnceCell::new();

static HPET_BASE_ADDR: AtomicUsize = AtomicUsize::new(0);
static HPET_FQ: OnceCell<usize> = OnceCell::new();

static USE_TSC_DEADLINE: AtomicBool = AtomicBool::new(false);

pub fn init() {
    let use_tsc = has_invariant_tsc();

    let mut tsc_fq = if use_tsc { check_tsc_frequency().unwrap_or(0) } else { 0 };
    let mut lapic_fq = check_apic_frequency().unwrap_or(0);

    let need_calibration = (use_tsc && tsc_fq == 0) || lapic_fq == 0;
    let need_hpet = need_calibration || !use_tsc;

    let mut hpet_opt = None;

    if need_hpet {
        if let Some(addr) = platform::hpet_base() {
            let mut hpet = Hpet::new(addr);
            hpet.enable();
            HPET_BASE_ADDR.store(hpet.base_addr, Ordering::Release);
            HPET_FQ.get_or_init(|| hpet.frequency);
            hpet_opt = Some(hpet);
        } else if !use_tsc {
            panic!("FATAL: No invariant TSC and no HPET found.");
        }
    }

    if need_calibration {
        let tsc = Tsc { frequency: 0 };

        let start_tsc = if use_tsc && tsc_fq == 0 {
            unsafe {
                core::arch::asm!("lfence");
            }
            tsc.read_counter()
        } else {
            0
        };

        setup_local_timer(35, 0x0FFF_FFFF, TimerMode::OneShot);
        let start_lapic = current_local_apic().current_count();

        if let Some(hpet) = &hpet_opt {
            let target = hpet.frequency / 100;
            let start = hpet.read_counter();

            while hpet.read_counter() - start < target {
                core::hint::spin_loop();
            }
        }

        let end_lapic = current_local_apic().current_count();

        let end_tsc = if use_tsc && tsc_fq == 0 {
            unsafe {
                core::arch::asm!("lfence");
            }
            tsc.read_counter()
        } else {
            0
        };

        if lapic_fq == 0 {
            let ticks_in_10ms = start_lapic.saturating_sub(end_lapic) * 100;
            lapic_fq = ticks_in_10ms * 100 * 16;
        }

        if use_tsc && tsc_fq == 0 {
            tsc_fq = end_tsc.saturating_sub(start_tsc) * 100;
        }

        if use_tsc {
            if let Some(mut hpet) = hpet_opt.take() {
                hpet.disable();
            }
        }
    }

    stop_local_timer();

    if lapic_fq == 0 {
        panic!("FATAL: Failed to obtain LAPIC frequency.");
    }

    LAPIC_FQ.get_or_init(|| lapic_fq);

    if use_tsc {
        if tsc_fq == 0 {
            panic!("FATAL: Failed to obtain TSC frequency.");
        }

        TIME_SRC_FQ.get_or_init(|| tsc_fq);
        TIME_SOURCE.store(TIME_SOURCE_TSC, Ordering::Release);
    } else {
        let hpet = hpet_opt.expect("FATAL: Hardware requirements not met (Missing TSC and HPET)");
        TIME_SRC_FQ.get_or_init(|| hpet.frequency);
        HPET_BASE_ADDR.store(hpet.base_addr, Ordering::Release);
        HPET_FQ.get_or_init(|| hpet.frequency);
        TIME_SOURCE.store(TIME_SOURCE_HPET, Ordering::Release);
    }

    USE_TSC_DEADLINE.store(has_tsc_deadline(), Ordering::Release);

    init_local();
}

pub fn init_local() {
    if USE_TSC_DEADLINE.load(Ordering::Acquire) {
        setup_local_timer(35, 0, TimerMode::TscDeadline);
    } else {
        setup_local_timer(35, 0, TimerMode::OneShot);
    }
}

pub fn read_counter() -> usize {
    match TIME_SOURCE.load(Ordering::Acquire) {
        TIME_SOURCE_TSC => read_tsc_counter(),
        TIME_SOURCE_HPET => {
            let base = HPET_BASE_ADDR.load(Ordering::Acquire);
            if base == 0 {
                0
            } else {
                read_hpet_counter(base)
            }
        }
        _ => 0,
    }
}

pub fn counter_frequency() -> usize {
    *TIME_SRC_FQ
}

pub fn ns_to_ticks(ns: usize) -> usize {
    ((ns as u128 * counter_frequency() as u128) / 1_000_000_000) as usize
}

pub fn arm_relative_ns(ns: usize) {
    arm_relative_ticks(ns_to_ticks(ns));
}

pub fn arm_relative_ticks(ticks: usize) {
    if USE_TSC_DEADLINE.load(Ordering::Acquire) {
        let current = read_counter();
        let target = current.saturating_add(ticks);

        unsafe {
            write_to_msr(target as u64, IA32_TSC_DEADLINE);
        }

        return;
    }

    let lapic_fq = *LAPIC_FQ;
    let global_fq = counter_frequency();
    let lapic_ticks = ((ticks as u128 * lapic_fq as u128) / global_fq as u128).max(1);

    arm_local_timer_oneshot(lapic_ticks as u32);
}

pub fn stop() {
    stop_local_timer();
}
