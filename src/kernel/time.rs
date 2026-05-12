use core::{
    arch::asm, 
    mem::transmute, 
    ptr::null_mut,
    sync::atomic::{
        AtomicBool, 
        AtomicPtr, 
        AtomicUsize, 
        Ordering
    }
};

use crate::{
    arch::LOCAL_APIC,
    arch::{self, x86_64::{
        apic::lapic::*, 
        cpuid::*, 
        interrupts::{
            disable_interrupts, 
            enable_interrupts
        }, 
        timer::{
            self, hpet::read_hpet_direct, tsc::read_tsc_direct, *
        },
    }},
    kernel::{
        acpi::hpet::get_hpet_base_addr,
        sync::TicketLock, 
        thread::{
            ThreadState,
            schedule::SCHEDULER,
        },
    },
    memory::PAGER,
};

pub static TIME_SRC_FQ: AtomicUsize = AtomicUsize::new(0);
pub static LAPIC_FQ: AtomicUsize = AtomicUsize::new(0);
pub static USE_TSC_DEADLINE: AtomicBool = AtomicBool::new(false);
pub static TIME_SOURCE: TicketLock<TimeSource> = TicketLock::new(TimeSource::None);
pub static HPET_BASE_ADDR: AtomicPtr<usize> = AtomicPtr::new(null_mut());


const IA32_TSC_DEADLINE: u32 = 0x6E0;

#[derive(Debug)]
pub enum TimeSource {
    None,
    HPET(timer::hpet::HPET),
    TSC(timer::tsc::TSC),
}

impl ClockSource for TimeSource {
    fn name(&self) -> &'static str {
        match self {
            Self::None => "None",
            Self::HPET(x) => x.name(),
            Self::TSC(x) => x.name(),
        }
    }

    fn read_counter(&self) -> usize {
        match self {
            Self::None => 0,
            Self::HPET(x) => x.read_counter(),
            Self::TSC(x) => x.read_counter(),
        }
    }

    fn frequency(&self) -> usize { TIME_SRC_FQ.load(Ordering::Relaxed) }
}

#[allow(dead_code)]
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

pub fn ns_to_ticks(ns: usize) -> usize {
    let freq = TIME_SRC_FQ.load(Ordering::Relaxed);
    ((ns as u128 * freq as u128) / 1_000_000_000) as usize
}

pub fn get_time() -> usize {
    let ptr = GET_TIME_FN.load(Ordering::Relaxed);
    let time_func: TimeFn = unsafe { transmute(ptr) };
    time_func()
}

pub fn sleep(ns: usize) {
    let target_time = get_time() + ns_to_ticks(ns);
    disable_interrupts();

    let mut sched = SCHEDULER.lock();
    let current_thread = sched.get_current_thread();
    unsafe {
        (*current_thread).state = ThreadState::Blocked;
        (*current_thread).wake_time = target_time;
    }
    sched.push_sleep(current_thread);
    sched.schedule();

    drop(sched);
    enable_interrupts();
}

pub fn init() {
    arch::init_timers();
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
            HPET_BASE_ADDR.store(addr as *mut usize, Ordering::Relaxed);
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
            unsafe {
                core::arch::asm!("lfence");
            }
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
            while hpet.read_counter() < start + target {
                core::hint::spin_loop();
            }
        }

        // stop the timers
        let end_lapic = lapic.current_count();
        let end_tsc = if use_tsc && tsc_fq == 0 {
            unsafe {
                core::arch::asm!("lfence");
            }
            tsc.read_counter()
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
        if tsc_fq == 0 {
            panic!("FATAL: Failed to obtain TSC frequency.");
        }
        let tsc = timer::tsc::TSC { frequency: tsc_fq };
        TIME_SRC_FQ.store(tsc_fq, Ordering::Relaxed);
        *TIME_SOURCE.lock() = TimeSource::TSC(tsc);
        GET_TIME_FN.store(read_tsc_direct as *mut (), Ordering::Relaxed);
    } else {
        let hpet = hpet_opt.expect("FATAL: Hardware requirements not met (Missing TSC and HPET)");
        TIME_SRC_FQ.store(hpet.frequency, Ordering::Relaxed);
        *TIME_SOURCE.lock() = TimeSource::HPET(hpet);
        GET_TIME_FN.store(read_hpet_direct as *mut (), Ordering::Relaxed);
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
