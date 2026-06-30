pub mod callout;
mod clock;
pub mod datetime;

pub use clock::*;

pub fn init() {
    hal::timer::init();
}
