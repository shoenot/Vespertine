use core::arch::asm;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use crate::{
    arch::x86_64::{
        timer,
        apic::lapic::*,
        cpuid::*
    },
    kernel::{
        lock::TicketLock,
        acpi::hpet::get_hpet_base_addr,
    },
    PAGER,
    LOCAL_APIC,
};

pub static TIME_SRC_FQ: AtomicUsize = AtomicUsize::new(0);
pub static LAPIC_FQ: AtomicUsize = AtomicUsize::new(0);
pub static TIME_SOURCE: TicketLock<TimeSource> = TicketLock::new(TimeSource::None);
pub static USE_TSC_DEADLINE: AtomicBool = AtomicBool::new(false);

const IA32_TSC_DEADLINE: u32 = 0x6E0;

#[derive(Debug)]
pub enum TimeSource {
    None,
    PIT(timer::pit::PIT),
    HPET(timer::hpet::HPET),
    TSC(timer::tsc::TSC),
}

impl ClockSource for TimeSource {
    fn name(&self) -> &'static str {
        match self {
            Self::None => "None",
            Self::PIT(x) => x.name(),
            Self::HPET(x) => x.name(),
            Self::TSC(x) => x.name(),
        }
    }

    fn read_counter(&self) -> usize {
        match self {
            Self::None => 0,
            Self::PIT(x) => x.read_counter(),
            Self::HPET(x) => x.read_counter(),
            Self::TSC(x) => x.read_counter(),
        }
    }

    fn frequency(&self) -> usize {
        TIME_SRC_FQ.load(Ordering::Relaxed)
    }
}

pub trait ClockSource {
    fn name(&self) -> &'static str;
    fn read_counter(&self) -> usize;
    fn frequency(&self) -> usize;
}

pub fn arm_sleep_ns(ns: usize) {
    if USE_TSC_DEADLINE.load(Ordering::Relaxed) {
        let tsc_fq = TIME_SRC_FQ.load(Ordering::Relaxed);
        let tsc_ticks = (ns * tsc_fq) / 1_000_000_000;

        let mut lo: u32;
        let mut hi: u32;
        unsafe {
            // read tsc 
            asm!("rdtsc",
                out("eax") lo, out("edx") hi, options(nomem, nostack));

            let current = ((hi as usize) << 32) | (lo as usize);
            let target = current + tsc_ticks;
            let tgt_lo = (target & 0xFFFF_FFFF) as u32;
            let tgt_hi = (target >> 32) as u32;
            
            // set deadline
            asm!("wrmsr",
                in("ecx") IA32_TSC_DEADLINE, in("eax") tgt_lo, in("edx") tgt_hi, options(nomem, nostack));
        }
    } else {
        // fallback to lapic 
        let lapic_fq = LAPIC_FQ.load(Ordering::Relaxed);
        let lapic_ticks = (ns as usize * lapic_fq) / 1_000_000_000;
        LOCAL_APIC.lock().arm_oneshot(lapic_ticks as u32);
    }
}

pub fn init() {
    let use_tsc = has_invariant_tsc();

    // try to read fqs straight from cpuid
    let mut tsc_fq = if use_tsc { check_tsc_frequency().unwrap_or(0) } else { 0 };
    let mut lapic_fq = check_apic_frequency().unwrap_or(0);

    // set up the HPET if cpuid failed us or if inv tsc isn't present
    let need_calibration = (use_tsc && tsc_fq == 0) || lapic_fq == 0;
    let need_hpet = need_calibration || !use_tsc;

    let mut hpet_opt = None;

    if need_hpet {
        if let Some(addr) = get_hpet_base_addr() {
            let mut pager = PAGER.lock();
            pager.map_mmio_addr((addr as u64) & !0xFFF);
            let mut hpet = timer::hpet::HPET { base_addr: 0, frequency: 0, enabled: false };
            hpet.init(addr);
            hpet.enable();
            hpet_opt = Some(hpet);
        } else if !use_tsc {
            panic!("FATAL: No Invariant TSC and no HPET found. Cannot establish monotonic clock.");
        }
    }

    if need_calibration {
        let tsc = timer::tsc::TSC { frequency: 0 };
        
        // start tsc (if it exists)
        let start_tsc = if use_tsc && tsc_fq == 0 {
            unsafe { core::arch::asm!("lfence"); }
            tsc.read_counter()
        } else { 
            0 
        };
        
        // start lapic 
        let lapic = LOCAL_APIC.lock();
        lapic.timer_setup(35, 0x0FFF_FFFF, TimerMode::OneShot);
        let start_lapic = lapic.current_count();

        // wait exactly 10ms
        if let Some(hpet) = &hpet_opt {
            let target = hpet.frequency / 100;
            let start = hpet.read_counter();
            while hpet.read_counter() < start + target { core::hint::spin_loop(); }
        } else {
            let mut pit = timer::pit::PIT { frequency: 0 };
            pit.init_mode_0();
            while pit.read_counter() > 0 && pit.read_counter() <= 11932 { core::hint::spin_loop(); }
        }

        // stop the timers
        let end_lapic = lapic.current_count();
        let end_tsc = if use_tsc && tsc_fq == 0 {
            unsafe { core::arch::asm!("lfence"); }
            tsc.read_counter()
        } else { 
            0 
        };

        if lapic_fq == 0 {
            lapic_fq = (start_lapic.saturating_sub(end_lapic)) * 100;
        }
        if use_tsc && tsc_fq == 0 {
            tsc_fq = (end_tsc.saturating_sub(start_tsc)) * 100;
        }

        // if HPET isn't needed for main clock disable it bc it sucks
        if use_tsc {
            if let Some(mut hpet) = hpet_opt.take() {
                hpet.disable();
            }
        }
    }

    if lapic_fq == 0 {
        panic!("FATAL: Failed to obtain LAPIC frequency.");
    }
    LAPIC_FQ.store(lapic_fq, Ordering::Relaxed);

    // main system clock ( TSC or HPET )
    if use_tsc {
        if tsc_fq == 0 { panic!("FATAL: Failed to obtain TSC frequency."); }
        let tsc = timer::tsc::TSC { frequency: tsc_fq };
        TIME_SRC_FQ.store(tsc_fq, Ordering::Relaxed);
        *TIME_SOURCE.lock() = TimeSource::TSC(tsc);
    } else {
        let hpet = hpet_opt.expect("FATAL: Hardware requirements not met (Missing TSC and HPET)");
        TIME_SRC_FQ.store(hpet.frequency, Ordering::Relaxed);
        *TIME_SOURCE.lock() = TimeSource::HPET(hpet);
    }

    // oneshot timer ( TSC Deadline or LAPIC )
    let lapic = LOCAL_APIC.lock();
    if has_tsc_deadline() {
        USE_TSC_DEADLINE.store(true, Ordering::Relaxed);
        lapic.timer_setup(35, 0, TimerMode::TscDeadline);
    } else {
        USE_TSC_DEADLINE.store(false, Ordering::Relaxed);
        lapic.timer_setup(35, 0, TimerMode::OneShot);
    }
}
