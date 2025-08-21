use super::TimeSource;

#[derive(Clone)]
pub struct MockClock {
    pub fixed: u64,
}

impl TimeSource for MockClock {
    fn now(&self) -> u64 {
        self.fixed
    }
}