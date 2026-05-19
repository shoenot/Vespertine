pub mod memory_tests;
pub mod smp_tests;
pub mod file_tests;

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
