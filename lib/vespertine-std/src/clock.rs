use vespertine_abi::tag::TAG_SYS_CLOCK;
use vespertine_rt::syscall::{sys_get_time, sys_sleep};

use crate::{Error, ErrorKind, env::find_tag};

pub struct Clock;

impl Clock {
    pub fn sleep_ms(ms: usize) -> Result<(), Error> {
        let handle = find_tag(TAG_SYS_CLOCK)
            .ok_or(Error { kind: ErrorKind::NotFound, message: "No clock handle in process".into()})?
            .id;
        sys_sleep(ms, handle).map_err(|e| Error::from(e))
    }

    pub fn now() -> usize {
        let handle = find_tag(TAG_SYS_CLOCK)
            .ok_or(Error { kind: ErrorKind::NotFound, message: "No clock handle in process".into()})
            .expect("No clock handle in process")
            .id;
        sys_get_time(handle)
    }
}
