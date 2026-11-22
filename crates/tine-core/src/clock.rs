//! The one clock of the crates that also run in the browser (tine-core,
//! tine-outline, tine-search, tine-store, tine-graph-features): `std::time`'s
//! `Instant::now` and `SystemTime::now` panic on `wasm32-unknown-unknown`.
//! On every other target these are std's own clock. Production code in those
//! crates reads time only here (enforced by tine-store's `clock_guard` test).

/// A monotonic instant: `std::time::Instant` natively, the browser's
/// `performance.now()` on wasm32.
pub use web_time::Instant;

/// The current wall-clock time as a `std::time::SystemTime`, so it compares
/// with file metadata times on every target (on wasm32 from `Date.now()`).
pub fn system_now() -> std::time::SystemTime {
    #[cfg(target_arch = "wasm32")]
    {
        let since = web_time::SystemTime::now()
            .duration_since(web_time::UNIX_EPOCH)
            .unwrap_or_default();
        std::time::UNIX_EPOCH + since
    }
    #[cfg(not(target_arch = "wasm32"))]
    std::time::SystemTime::now()
}
