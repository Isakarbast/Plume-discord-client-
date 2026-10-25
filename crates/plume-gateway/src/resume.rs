use std::time::Duration;

use crate::heartbeat::jitter;

/// What we need to resume a session after the connection drops.
#[derive(Debug, Clone)]
pub struct ResumeState {
    pub session_id: String,
    /// Discord gives each session its own URL to resume on.
    pub resume_url: String,
}

/// Exponential backoff between reconnect attempts: ~1s, 2s, 4s … up to 60s.
#[derive(Debug, Default)]
pub struct Backoff {
    attempt: u32,
}

impl Backoff {
    const MAX_SECONDS: u64 = 60;

    pub fn next_delay(&mut self) -> Duration {
        let seconds = (1u64 << self.attempt.min(6)).min(Self::MAX_SECONDS);
        self.attempt += 1;
        // Up to one extra second of jitter.
        Duration::from_secs(seconds) + Duration::from_secs_f64(jitter())
    }

    pub fn reset(&mut self) {
        self.attempt = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grows_then_caps() {
        let mut backoff = Backoff::default();
        let seconds: Vec<u64> = (0..9).map(|_| backoff.next_delay().as_secs()).collect();
        assert_eq!(seconds, [1, 2, 4, 8, 16, 32, 60, 60, 60]);
    }

    #[test]
    fn reset_starts_over() {
        let mut backoff = Backoff::default();
        backoff.next_delay();
        backoff.next_delay();
        backoff.reset();
        assert_eq!(backoff.next_delay().as_secs(), 1);
    }
}
