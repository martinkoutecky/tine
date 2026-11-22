//! The one clock of the crates that also run in the browser (tine-core,
//! tine-outline, tine-search, tine-store, tine-graph-features): `std::time`'s
//! `Instant::now` and `SystemTime::now` panic on `wasm32-unknown-unknown`.
//! On every other target these are std's own clock. Production code in those
//! crates reads time only here (today's date and the backup stamp included) (enforced by tine-store's `clock_guard` test).

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

impl crate::date::JournalDate {
    /// Today's date in the device's local civil calendar.  Journal membership,
    /// relative queries, and their date-keyed caches must all agree on this
    /// boundary; UTC epoch days split those user-facing meanings near midnight.
    pub fn today() -> Self {
        use chrono::{Datelike, Local};
        let now = Local::now();
        Self {
            year: now.year(),
            month: now.month(),
            day: now.day(),
        }
    }
}

/// UTC backup directory stamp in `YYYY-MM-DD_HH-MM-SS` form.
pub fn utc_backup_stamp() -> String {
    use std::time::UNIX_EPOCH;
    let secs = system_now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let date = crate::date::JournalDate::from_days(days);
    format!(
        "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
        date.year,
        date.month,
        date.day,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}
