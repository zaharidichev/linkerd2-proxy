extern crate tokio_timer;

use std::rc::Rc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tokio_timer::clock;

/// This represents a "rotating" value which stores two T values, one which
/// should be read and one which should be written to.  Every period, the
/// read T is discarded and replaced by the write T.  The idea here is that
/// the read T should always contain a full period (the previous period) of
/// write operations.
pub struct Rotating<T> {
    read: Rc<Mutex<T>>,
    write: Rc<Mutex<T>>,
    new: fn() -> T,
    last_rotation: Instant,
    period: Duration,
}

impl<T> Rotating<T> {

    pub fn new(period: Duration, new: fn() -> T) -> Rotating<T> {
        Rotating {
            read: Rc::new(Mutex::new(new())),
            write: Rc::new(Mutex::new(new())),
            new,
            last_rotation: clock::now(),
            period,
        }
    }

    pub fn read(&mut self) -> Rc<Mutex<T>> {
        self.maybe_rotate();
        self.read.clone()
    }

    pub fn write(&mut self) -> Rc<Mutex<T>> {
        self.maybe_rotate();
        self.write.clone()
    }

    fn maybe_rotate(&mut self) {
        let delta = clock::now() - self.last_rotation;
        // TODO: replace with delta.duration_div when it becomes stable.
        let rotations = (Self::duration_as_nanos(&delta) / Self::duration_as_nanos(&self.period)) as u32;
        if rotations >= 2 {
            self.clear();
        } else if rotations == 1 {
            self.rotate();
        }
        self.last_rotation += self.period * rotations;
    }

    fn rotate(&mut self) {
        self.read = self.write.clone();
        self.write = Rc::new(Mutex::new((self.new)()));
    }

    fn clear(&mut self) {
        self.read = Rc::new(Mutex::new((self.new)()));
        self.write = Rc::new(Mutex::new((self.new)()));
    }

    fn duration_as_nanos(d: &Duration) -> u64 {
        d.as_secs() * 1_000_000_000 + (d.subsec_nanos() as u64)
    }

}
