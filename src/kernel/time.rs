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

    if has_invariant_tsc() {
        let mut tsc = timer::tsc::TSC { frequency: 0 };
        let lapic = LOCAL_APIC.lock();
        match src {
            TimeSource::HPET(hpet) => {
                let tick_interval_tgt = hpet.frequency / 100;
                
                // start timers
                let start_hpet = hpet.read_counter();
                unsafe { asm!("lfence"); }
                let start_tsc = tsc.read_counter();
                lapic.timer_setup(35, 0x0FFF_FFFF);
                let start_lapic = lapic.current_count();

                // loop 10ms 
                let end_hpet = start_hpet + tick_interval_tgt;
                while hpet.read_counter() < end_hpet {
                    spin_loop();
                }

                unsafe { asm!("lfence"); }
                let end_tsc = tsc.read_counter();
                let end_lapic = lapic.current_count();

                tsc.frequency = (end_tsc - start_tsc) * 100;
                TIME_SRC_FQ.store(tsc.frequency, Ordering::Relaxed);
                LAPIC_FQ.store((start_lapic - end_lapic) * 100, Ordering::Relaxed);
                let mut time_source = TIME_SOURCE.lock();
                *time_source = TimeSource::TSC(tsc);

                return;
            },
            TimeSource::ACPIPM(acpipm) => {
                let tick_interval_tgt = 35795; 
                
                let start_acpipm = acpipm.read_counter();
                unsafe { asm!("lfence"); }
                let start_tsc = tsc.read_counter();
                lapic.timer_setup(35, 0x0FFF_FFFF);
                let start_lapic = lapic.current_count();

                while (acpipm.read_counter().wrapping_sub(start_acpipm) & 0x00FF_FFFF) < tick_interval_tgt {
                    spin_loop();
                }

                unsafe { asm!("lfence"); }
                let end_tsc = tsc.read_counter();
                let end_lapic = lapic.current_count();

                tsc.frequency = (end_tsc - start_tsc) * 100;
                TIME_SRC_FQ.store(tsc.frequency, Ordering::Relaxed);
                LAPIC_FQ.store((start_lapic - end_lapic) * 100, Ordering::Relaxed);
                
                let mut time_source = TIME_SOURCE.lock();
                *time_source = TimeSource::TSC(tsc);
                return;
            },
            TimeSource::PIT(pit) => {
                unsafe { asm!("lfence"); }
                let start_tsc = tsc.read_counter();
                lapic.timer_setup(35, 0x0FFF_FFFF);
                let start_lapic = lapic.current_count();

                while pit.read_counter() > 0 && pit.read_counter() <= 11932 {
                    spin_loop();
                }

                unsafe { asm!("lfence"); }
                let end_tsc = tsc.read_counter();
                let end_lapic = lapic.current_count();

                tsc.frequency = (end_tsc - start_tsc) * 100;
                TIME_SRC_FQ.store(tsc.frequency, Ordering::Relaxed);
                LAPIC_FQ.store((start_lapic - end_lapic) * 100, Ordering::Relaxed);
                
                let mut time_source = TIME_SOURCE.lock();
                *time_source = TimeSource::TSC(tsc);
                return;
            },
            _ => {},
        }
    } else {
        let mut time_source = TIME_SOURCE.lock();
        match src {
            TimeSource::HPET(hpet) => {
                TIME_SRC_FQ.store(hpet.frequency(), Ordering::Relaxed);
                *time_source = src;
            },
            TimeSource::ACPIPM(acpipm) => {
                TIME_SRC_FQ.store(acpipm.frequency(), Ordering::Relaxed);
                *time_source = src;
            },
            TimeSource::PIT(_) => {
                let mut new_src = timer::pit::PIT { frequency: 0 };
                new_src.init_mode_2(1000);
                TIME_SRC_FQ.store(1000, Ordering::Relaxed);
            },
            _ => {},
        }
    }
}
