use core::sync::atomic::{AtomicUsize, Ordering};
use crate::arch::x86_64::timer;
use crate::arch::x86_64::cpuid::*;
use crate::kernel::acpi::fadt::get_pm_timer_addr;
use crate::kernel::acpi::hpet::get_hpet_base_addr;
use crate::kernel::lock::TicketLock;
use crate::PAGER;
use crate::LOCAL_APIC;
use core::arch::asm;
use core::hint::spin_loop;

pub static TICKS: AtomicUsize = AtomicUsize::new(0);
pub static TIME_SRC_FQ: AtomicUsize = AtomicUsize::new(0);
pub static LAPIC_FQ: AtomicUsize = AtomicUsize::new(0);
pub static TIME_SOURCE: TicketLock<TimeSource> = TicketLock::new(TimeSource::None);

#[derive(Debug)]
pub enum TimeSource {
    None,
    PIT(timer::pit::PIT),
    ACPIPM(timer::acpi_pm::ACPI_PM_Timer),
    HPET(timer::hpet::HPET),
    TSC(timer::tsc::TSC),
}

pub fn increment_ticks() {
    TICKS.fetch_add(1, Ordering::Relaxed);
}

pub fn get_ticks() -> usize {
    TICKS.load(Ordering::Relaxed)
}

pub trait ClockSource {
    fn name(&self) -> &'static str;
    fn read_counter(&self) -> usize;
    fn frequency(&self) -> usize;
}

pub fn sleep_ticks(ticks_to_wait: usize) {
    let start = get_ticks();
    while get_ticks() - start < ticks_to_wait {
        unsafe {
            asm!("hlt", options(nomem, nostack));
        }
    }
}

pub fn sleep_ms(ms: usize) {
    sleep_ticks(ms);
}

fn calibrate_timers<F: FnOnce()>(wait_loop: F, use_tsc: bool, multiplier: usize) -> (usize, usize) {
    let lapic = LOCAL_APIC.lock();
    let mut tsc = timer::tsc::TSC { frequency: 0 };
    
    let start_tsc = if use_tsc {
        unsafe { asm!("lfence"); }
        tsc.read_counter()
    } else { 0 };
    
    lapic.timer_setup(35, 0x0FFF_FFFF);
    let start_lapic = lapic.current_count();

    wait_loop();

    let end_tsc = if use_tsc {
        unsafe { asm!("lfence"); }
        tsc.read_counter()
    } else { 0 };
    
    let end_lapic = lapic.current_count();

    let tsc_freq = (end_tsc.saturating_sub(start_tsc)) * multiplier;
    let lapic_freq = (start_lapic.saturating_sub(end_lapic)) * multiplier;

    (tsc_freq, lapic_freq)
}

pub fn init() {
    // tier 1: read tsc_fq straight from cpuid
    match check_tsc_frequency() {
        Some(tsc_fq) => {
            if has_invariant_tsc() {
                let tsc = timer::tsc::TSC { frequency: tsc_fq };
                let mut time_source = TIME_SOURCE.lock();
                *time_source = TimeSource::TSC(tsc);
                TIME_SRC_FQ.store(tsc.frequency, Ordering::Relaxed);
                LAPIC_FQ.store(check_apic_frequency().unwrap(), Ordering::Relaxed);
                
                let ticks_per_ms = (LAPIC_FQ.load(Ordering::Relaxed) / 1000) as u32;
                LOCAL_APIC.lock().timer_setup(35, ticks_per_ms);
                return;
            }
        },
        None => {},
    }

    let mut src = TimeSource::None;

    // tier 2: check if hpet exits
    match get_hpet_base_addr() {
        Some(addr) => {
            let mut pager = PAGER.lock();
            pager.map_mmio_addr((addr as u64) & !0xFFF);
            let mut hpet = timer::hpet::HPET { base_addr: 0, frequency: 0, enabled: false };
            hpet.init(addr);
            hpet.enable();
            src = TimeSource::HPET(hpet);
        },
        None => {},
    }

    // tier 3: check if acpi pm exists 
    if matches!(src, TimeSource::None) {
        let (timer_addr, is_mmio) = get_pm_timer_addr();
        if timer_addr > 0 {
            if is_mmio {
                let mut pager = PAGER.lock();
                pager.map_mmio_addr((timer_addr as u64) & !0xFFF);
            }
            let pm_timer = timer::acpi_pm::ACPI_PM_Timer { timer_addr, is_mmio };
            src = TimeSource::ACPIPM(pm_timer);
        }
    }

    // tier 4: fall back to pit
    if matches!(src, TimeSource::None) {
        let mut pit = timer::pit::PIT { frequency: 0 };
        pit.init_mode_0();
        src = TimeSource::PIT(pit);
    }

    let use_tsc = has_invariant_tsc();

    let (tsc_freq, lapic_freq, mut final_src) = match src {
        TimeSource::HPET(hpet) => {
            let target = hpet.frequency / 100; // 10ms
            let start = hpet.read_counter();
            let (t, l) = calibrate_timers(|| {
                while hpet.read_counter() < start + target { spin_loop(); }
            }, use_tsc, 100);
            (t, l, TimeSource::HPET(hpet))
        },
        TimeSource::ACPIPM(acpipm) => {
            let target = 35795; // 10ms
            let start = acpipm.read_counter();
            let (t, l) = calibrate_timers(|| {
                while (acpipm.read_counter().wrapping_sub(start) & 0x00FF_FFFF) < target { spin_loop(); }
            }, use_tsc, 100);
            (t, l, TimeSource::ACPIPM(acpipm))
        },
        TimeSource::PIT(mut pit) => {
            let (t, l) = calibrate_timers(|| {
                while pit.read_counter() > 0 && pit.read_counter() <= 11932 { spin_loop(); }
            }, use_tsc, 100);
            
            // switch the pit to mode 2 if we're gonna use it as an actual timer
            if !use_tsc { pit.init_mode_2(1000); }
            (t, l, TimeSource::PIT(pit))
        },
        _ => (0, 0, TimeSource::None),
    };

    if lapic_freq > 0 {
        LAPIC_FQ.store(lapic_freq, Ordering::Relaxed);
    }

    if use_tsc && tsc_freq > 0 {
        let tsc = timer::tsc::TSC { frequency: tsc_freq };
        TIME_SRC_FQ.store(tsc_freq, Ordering::Relaxed);
        final_src = TimeSource::TSC(tsc);
    } else {
        match final_src {
            TimeSource::HPET(h) => TIME_SRC_FQ.store(h.frequency(), Ordering::Relaxed),
            TimeSource::ACPIPM(a) => TIME_SRC_FQ.store(a.frequency(), Ordering::Relaxed),
            TimeSource::PIT(_) => TIME_SRC_FQ.store(1000, Ordering::Relaxed),
            _ => {}
        }
    }

    *TIME_SOURCE.lock() = final_src;

    // reconfigure lapic to do 1ms heartbeat
    let current_lapic_fq = LAPIC_FQ.load(Ordering::Relaxed);
    if current_lapic_fq > 0 {
        let ticks_per_ms = (current_lapic_fq / 1000) as u32;
        LOCAL_APIC.lock().timer_setup(35, ticks_per_ms);
    }
}
