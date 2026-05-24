#[macro_export]
macro_rules! terminate_thread {
    () => {
        get_core_data().scheduler.terminate();
        unreachable!()
    };
}
