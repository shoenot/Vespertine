#![no_std]
#![no_main]
pub mod app;
mod bitwise;
pub mod op;
pub mod protocol;
pub mod tag;
pub mod shell;

pub use op::*;

mod invocations;
pub use invocations::*;

mod process;
pub use process::*;

mod access_rights;
pub use access_rights::*;

mod capabilities;
pub use capabilities::*;

mod signals;
pub use signals::*;

mod user;
pub use user::*;

mod stat;
pub use stat::*;

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HandleID(pub usize);
