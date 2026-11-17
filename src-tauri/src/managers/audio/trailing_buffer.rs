use std::sync::{Condvar, Mutex};
use std::time::Duration;

/// The optional post-release tail belongs to a take only until the next press.
#[derive(Default)]
pub(crate) struct TrailingBuffer {
    interrupted: Mutex<bool>,
    changed: Condvar,
}

impl TrailingBuffer {
    pub fn reset(&self) {
        *self.interrupted.lock().unwrap() = false;
    }

    pub fn interrupt(&self) {
        *self.interrupted.lock().unwrap() = true;
        self.changed.notify_all();
    }

    pub fn wait(&self, millis: u64) {
        if millis > 0 {
            let guard = self.interrupted.lock().unwrap();
            drop(
                self.changed
                    .wait_timeout_while(guard, Duration::from_millis(millis), |cut| !*cut)
                    .unwrap(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn press_before_stop_worker_starts_already_ends_its_tail() {
        let tail = TrailingBuffer::default();
        tail.reset();
        tail.interrupt();
        let start = std::time::Instant::now();
        tail.wait(1500);
        assert!(start.elapsed() < Duration::from_millis(100));
    }
}
