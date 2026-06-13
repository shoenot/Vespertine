use vespertine_abi::{AccessRights, HandleID, tag::CAP_CLOCK};
use vespertine_rt::syscall::{sys_close, sys_get_time, sys_sleep};
use vespertine_rt::once::OnceCell;

use crate::{Error, broker::Broker, env, fs::walk_path};

// Low level clock capability API
pub struct Clock {
    handle: HandleID,
}

impl Clock {
    pub fn request(rights: AccessRights) -> Result<Self, Error> {
        let broker_handle = walk_path("/System/Services/Clock", AccessRights::READ).map_err(Error::from)?;
        let broker = Broker::from_handle(broker_handle);
        let handle = broker.request(CAP_CLOCK, rights)?;
        Ok(Self { handle })
    }

    pub fn sleep_ms(&self, ms: usize) -> Result<(), Error> {
        sys_sleep(ms, self.handle).map_err(|e| Error::from(e))
    }

    pub fn now(&self) -> (usize, usize) {
        sys_get_time(self.handle)
    }
}

impl Drop for Clock {
    fn drop(&mut self) {
        let _ = sys_close(self.handle);
    }
}

// High level clock functions abstraction 
pub struct Time;

static READ_CLOCK: OnceCell<Clock> = OnceCell::new();
static WRITE_CLOCK: OnceCell<Clock> = OnceCell::new();

impl Time {
    pub fn now() -> (usize, usize) {
        let clock = READ_CLOCK.get_or_init(|| {
            Clock::request(AccessRights::READ)
                .expect("Program does not have the appropriate Clock capability")
        });
        clock.now()
    }

    pub fn sleep_ms(ms: usize) -> Result<(), Error> {
        let clock = WRITE_CLOCK.get_or_init(|| {
            Clock::request(AccessRights::WRITE)
                .expect("Program does not have the appropriate Clock capability")
        });
        clock.sleep_ms(ms)
    }
}
