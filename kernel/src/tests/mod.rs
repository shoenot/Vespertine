mod concurrency_tests;
pub(crate) mod ext2_tests;
mod memory_tests;
mod object_tests;
pub(crate) mod shootdown_tests;
pub(crate) mod thread_lifetime_tests;

pub(crate) fn run_pre_vfs_tests() {
    crate::klogln!("[TEST] Pre-VFS diagnostics");
    concurrency_tests::run_concurrency_tests();
    crate::executor::run_diagnostic_tests();
    crate::object::ipc::socket::run_diagnostic_tests();
    crate::drivers::virtio::blk::run_diagnostic_tests();
    memory_tests::run();
    crate::klogln!("[TEST] Pre-VFS diagnostics passed");
}

pub(crate) async fn run_post_vfs_tests() {
    crate::klogln!("[TEST] Post-VFS diagnostics");
    object_tests::run().await;
    crate::klogln!("[TEST] Post-VFS diagnostics passed");
}
