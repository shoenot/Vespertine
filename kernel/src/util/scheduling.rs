#[macro_export]
macro_rules! terminate_thread {
    () => {
        get_core_data().scheduler.terminate(0);
        unreachable!()
    };
    ($code:expr) => {
        get_core_data().scheduler.terminate($code);
        unreachable!()
    };
}
