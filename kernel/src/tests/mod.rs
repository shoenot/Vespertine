mod concurrency_tests;
pub(crate) mod ext2_tests;
mod futex_tests;
mod memory_tests;
mod object_tests;
pub(crate) mod shootdown_tests;
pub(crate) mod thread_lifetime_tests;
mod file_cache_tests;

pub(crate) fn run_pre_vfs_tests() {
    crate::klogln!(Info, "Tests", "Starting pre-VFS diagnostics");
    concurrency_tests::run_concurrency_tests();
    crate::executor::run_diagnostic_tests();
    crate::object::ipc::socket::run_diagnostic_tests();
    crate::object::ipc::portal::run_diagnostic_tests();
    futex_tests::run();
    crate::drivers::virtio::blk::run_diagnostic_tests();
    memory_tests::run();
    crate::klogln!(Info, "Tests", "Pre-VFS diagnostics passed");
}

pub(crate) async fn run_post_vfs_tests() {
    crate::klogln!(Info, "Tests", "Starting post-VFS diagnostics");
    object_tests::run().await;
    file_cache_tests::run().await;
    crate::klogln!(Info, "Tests", "Post-VFS diagnostics passed");
}
