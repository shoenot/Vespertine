use crate::kernel::thread::schedule::SchedulerState;


struct CpuLocalData {
    lapic_id: usize,
    sched_state: SchedulerState,

}
