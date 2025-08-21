use quanta::{Clock, IntoNanoseconds};
use super::TimeSource;

pub struct QuantaClock {
    clock: Clock,
}

impl QuantaClock {
    pub fn new() -> Self {
        Self { clock: Clock::new() }
    }
}

impl TimeSource for QuantaClock {
    fn now(&self) -> u64 {
        self.clock.raw().into_nanos()
    }
}