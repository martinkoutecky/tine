//! The one bounded latency record of the diagnostics: store lock waits
//! (tine-store `launch_diag`) and per-command latency in the app's flight
//! recorder share this type, so the two shapes cannot drift (I-12). Numbers
//! only (I-5); no I/O.

use serde_json::{json, Value};
use std::collections::VecDeque;
use std::time::Duration;

/// A duration in whole microseconds, saturating at `u64::MAX`.
pub fn micros(duration: Duration) -> u64 {
    duration.as_micros().min(u128::from(u64::MAX)) as u64
}

/// Microseconds as milliseconds with one decimal (sub-millisecond phases stay visible).
pub fn ms(us: u64) -> f64 {
    (us as f64 / 100.0).round() / 10.0
}

/// Upper bounds (ms) of the latency buckets; the last bucket is open-ended.
pub const LATENCY_BOUNDS_MS: [u64; 5] = [1, 10, 100, 500, 2000];
/// Most recent durations a histogram keeps verbatim.
pub const LATENCY_RECENT: usize = 8;

/// A bounded latency record: count, bucket counts, maximum and the last few
/// durations. Fixed size whatever the traffic; numbers only (I-5). It is the
/// one latency shape in the diagnostics (store lock waits and per-command
/// latency in the app's flight recorder), so the two cannot drift (I-12).
#[derive(Clone, Debug, Default)]
pub struct LatencyHist {
    count: u64,
    buckets: [u64; LATENCY_BOUNDS_MS.len() + 1],
    max_us: u64,
    recent: VecDeque<u64>,
}

impl LatencyHist {
    /// An empty histogram.
    pub fn new() -> Self {
        Self::default()
    }

    /// Count one duration (O(1), no allocation once the recent ring is full).
    pub fn record(&mut self, elapsed: Duration) {
        let us = micros(elapsed);
        self.count += 1;
        let bucket = LATENCY_BOUNDS_MS
            .iter()
            .position(|bound| us <= bound * 1000)
            .unwrap_or(LATENCY_BOUNDS_MS.len());
        self.buckets[bucket] += 1;
        self.max_us = self.max_us.max(us);
        if self.recent.len() == LATENCY_RECENT {
            self.recent.pop_front();
        }
        self.recent.push_back(us);
    }

    /// Count, buckets with their upper bounds, maximum and last durations, in ms.
    pub fn to_json(&self) -> Value {
        json!({
            "count": self.count,
            "maxMs": ms(self.max_us),
            "bucketUpperBoundsMs": LATENCY_BOUNDS_MS,
            "buckets": self.buckets,
            "lastMs": self.recent.iter().map(|us| ms(*us)).collect::<Vec<_>>(),
        })
    }
}
