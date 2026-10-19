//! The `sample` clause's randomness: the seed (pinnable by a test) and the uniform
//! random subset (a child module of `exec`).

thread_local! {
    /// A test's pinned `sample` seed (see [`pin_sample_seed`]); `None` draws a
    /// fresh seed per execution, as OG's `shuffle` does.
    static SAMPLE_SEED: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

/// Pin the seed every `sample` on THIS thread draws from, until the guard
/// drops. A seed is the only randomness in query execution, so a test that
/// pins it gets a reproducible subset.
#[cfg(test)]
pub(crate) fn pin_sample_seed(seed: u64) -> SampleSeedGuard {
    SAMPLE_SEED.with(|cell| cell.set(Some(seed)));
    SampleSeedGuard
}

#[cfg(test)]
pub(crate) struct SampleSeedGuard;

#[cfg(test)]
impl Drop for SampleSeedGuard {
    fn drop(&mut self) {
        SAMPLE_SEED.with(|cell| cell.set(None));
    }
}

fn fresh_sample_seed() -> u64 {
    if let Some(seed) = SAMPLE_SEED.with(|cell| cell.get()) {
        return seed;
    }
    static DRAWS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos() as u64);
    nanos
        ^ DRAWS
            .fetch_add(0x9E37_79B9_7F4A_7C15, std::sync::atomic::Ordering::Relaxed)
            .rotate_left(17)
}

/// Keep a uniformly random subset of `items` of at most `n` elements, in their
/// current relative order (a partial Fisher-Yates over the indices, splitmix64
/// as the generator). Cost O(len).
pub(super) fn take_random_subset<T>(items: &mut Vec<T>, n: usize) {
    if items.len() <= n {
        return;
    }
    let mut state = fresh_sample_seed();
    let mut next = move || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let mut order: Vec<usize> = (0..items.len()).collect();
    for at in 0..n {
        let pick = at + (next() % (order.len() - at) as u64) as usize;
        order.swap(at, pick);
    }
    let mut keep = vec![false; items.len()];
    for &index in &order[..n] {
        keep[index] = true;
    }
    let mut index = 0;
    items.retain(|_| {
        index += 1;
        keep[index - 1]
    });
}
