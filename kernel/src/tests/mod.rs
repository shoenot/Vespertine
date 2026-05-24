use crate::klogln;

pub mod memory_tests;
pub mod smp_tests;
pub mod object_tests;

pub const RUN_TESTS: bool = false;

pub fn run_pre_vfs_tests() {
    if !RUN_TESTS {
        return;
    }
    klogln!("========== RUNNING SYSTEM DIAGNOSITC UNIT TESTS (PHASE 1) ==========");
    memory_tests::run_pmm_tests();
    klogln!("================= ALL DIAGNOSTIC UNIT TESTS PASSED =================");
}

pub fn run_post_vfs_tests() {
    if !RUN_TESTS {
        return;
    }
    klogln!("========== RUNNING SYSTEM DIAGNOSITC UNIT TESTS (PHASE 2) ==========");
    object_tests::run_object_tests();
    klogln!("================= ALL DIAGNOSTIC UNIT TESTS PASSED =================");
}

#[macro_export]
macro_rules! vklog {
    ($verbose:expr, $($arg:tt)*) => {
        if $verbose {
            klog!($($arg)*);
        }
    };
}

#[macro_export]
macro_rules! vklogln {
    ($verbose:expr, $($arg:tt)*) => {
        if $verbose {
            klogln!($($arg)*);
        }
    };
}
