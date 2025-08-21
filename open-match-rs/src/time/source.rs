pub trait TimeSource {
    fn now(&self) -> u64;
}