use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Tracks heartbeats so we notice a "zombie" connection: one that looks open
/// but where Discord stopped answering.
pub struct Heartbeat {
    pub interval: Duration,
    awaiting_ack: bool,
    sent_at: Option<Instant>,
    /// Round-trip time of the last heartbeat. Nice to show in the UI later.
    pub latency: Option<Duration>,
}

impl Heartbeat {
    pub fn new(interval: Duration) -> Self {
        Heartbeat {
            interval,
            awaiting_ack: false,
            sent_at: None,
            latency: None,
        }
    }

    /// Discord asks that the *first* heartbeat waits a random fraction of the
    /// interval, so clients that reconnect together don't all beat at once.
    pub fn first_delay(&self) -> Duration {
        self.interval.mul_f64(jitter())
    }

    /// True if the previous heartbeat was never acknowledged.
    pub fn is_zombie(&self) -> bool {
        self.awaiting_ack
    }

    pub fn sent(&mut self) {
        self.awaiting_ack = true;
        self.sent_at = Some(Instant::now());
    }

    pub fn acked(&mut self) {
        self.awaiting_ack = false;
        self.latency = self.sent_at.map(|t| t.elapsed());
    }
}

/// A number in `0.0..1.0`. Not cryptographic, just enough to spread clients out.
pub fn jitter() -> f64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    f64::from(nanos) / 1_000_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_ack_means_zombie() {
        let mut hb = Heartbeat::new(Duration::from_secs(41));
        assert!(!hb.is_zombie());
        hb.sent();
        assert!(hb.is_zombie());
        hb.acked();
        assert!(!hb.is_zombie());
        assert!(hb.latency.is_some());
    }

    #[test]
    fn first_delay_is_within_the_interval() {
        let hb = Heartbeat::new(Duration::from_secs(41));
        assert!(hb.first_delay() < hb.interval);
    }
}
