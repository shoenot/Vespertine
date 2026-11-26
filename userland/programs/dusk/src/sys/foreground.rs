use alloc::{sync::Arc, vec::Vec};
use vespertine_abi::{Signal, app::termios::{ISIG, Termios}};
use vrt::{mutex::Mutex, println, syscall::sys_wait, thread};
use vstd::{Error, Process, notifications::Notifications, term::{enable_signals, get_terminfo, set_terminfo}};



#[derive(Clone)]
pub struct Foreground {
    inbox: Arc<Notifications>,
    targets: Arc<Mutex<Vec<Arc<Process>>>>,
}

impl Foreground {
    pub fn start() -> Result<Self, Error> {
        let foreground = Self {
            inbox: Arc::new(Notifications::claim()?),
            targets: Arc::new(Mutex::new(Vec::new())),
        };

        let worker = foreground.clone();
        thread::spawn(move || {
            if let Err(e) = worker.forward_interrupts() {
                println!("dusk: interrupt worker stopped: {e}");
            }
        })?;

        Ok(foreground)
    }

    pub fn forward_interrupts(&self) -> Result<(), Error> {
        loop {
            sys_wait(self.inbox.handle(), Signal::READABLE)?;

            let targets = {
                let targets = self.targets.lock();
                if self.inbox.try_take()?.is_none() { continue };
                targets.clone()
            };

            for process in targets {
                if let Err(e) = process.send_interrupt() {
                    println!("dusk: cannot interrupt foreground process: {e}");
                }
            }
        }
    }

    pub fn begin(&self, procs: Vec<Arc<Process>>) -> Result<(), Error> {
        self.finish()?;
        *self.targets.lock() = procs;

        if let Err(e) = enable_signals() {
            self.targets.lock().clear();
            return Err(e);
        }

        Ok(())
    }

    pub fn finish(&self) -> Result<(), Error> {
        let mut mode = Termios::default();
        mode.c_lflag &= !ISIG;

        let res = set_terminfo(mode).and_then(|_| get_terminfo());
        let mut targets = self.targets.lock();
        targets.clear();
        res?;
        self.inbox.try_take()?;

        Ok(())
    }
}
