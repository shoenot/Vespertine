#[macro_export]
macro_rules! terminate_thread {
    () => {
        current_core_mut().scheduler.terminate(0)
    };
    ($code:expr) => {
        current_core_mut().scheduler.terminate($code)
    };
}
