//! GH #809: one outstanding main-loop acknowledgement, no idle flight events.
//! Desktop only: mobile suspension is expected and is not a UI-thread hang.
//! The callback only acknowledges; recording and all waits happen off the UI.
#![cfg_attr(any(target_os = "android", target_os = "ios"), allow(dead_code))]

use std::time::{Duration, Instant};

const INTERVAL: Duration = Duration::from_secs(5);
const THRESHOLDS: [Duration; 3] = [
    Duration::from_secs(2),
    Duration::from_secs(10),
    Duration::from_secs(30),
];

/// Wait for one probe, reporting each threshold once even if it never replies.
/// One slot means a hung UI cannot accumulate callbacks or recorder traffic.
fn probe(
    post: impl FnOnce(Box<dyn FnOnce() + Send>) -> bool,
    thresholds: &[Duration],
    mut record: impl FnMut(u64, u64, bool),
) -> bool {
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let started = Instant::now();
    if !post(Box::new(move || { let _ = send.try_send(Instant::now()); })) {
        return false;
    }
    let mut last_threshold = None;
    for threshold in thresholds {
        let remaining = threshold.saturating_sub(started.elapsed());
        match receive.recv_timeout(remaining) {
            Ok(replied) => {
                let delay = replied.duration_since(started).as_millis() as u64;
                if let Some(last) = last_threshold {
                    record(last, delay, true);
                } else if replied.duration_since(started) >= thresholds[0] {
                    record(thresholds[0].as_millis() as u64, delay, true);
                }
                return true;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return false,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let threshold_ms = threshold.as_millis() as u64;
                record(threshold_ms, started.elapsed().as_millis() as u64, false);
                last_threshold = Some(threshold_ms);
            }
        }
    }
    // After 30 s, sleep until the one reply; no repeated events during a hang.
    match receive.recv() {
        Ok(replied) => {
            record(last_threshold.unwrap_or(0), replied.duration_since(started).as_millis() as u64, true);
            true
        }
        Err(_) => false,
    }
}

#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
pub(crate) fn start<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let app = app.clone();
    let _ = std::thread::Builder::new().name("main-loop-watch".into()).spawn(move || loop {
        std::thread::sleep(INTERVAL);
        if !probe(
            |acknowledge| app.run_on_main_thread(acknowledge).is_ok(),
            &THRESHOLDS,
            crate::flight::record_main_thread_stall,
        ) {
            break;
        }
    });
}

#[cfg(any(target_os = "android", target_os = "ios"))]
pub(crate) fn start<R: tauri::Runtime>(_app: &tauri::AppHandle<R>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_main_loop_is_quiet() {
        let mut events = Vec::new();
        for _ in 0..100 {
            assert!(probe(|ack| { ack(); true }, &THRESHOLDS,
                |threshold, delay, recovered| events.push((threshold, delay, recovered))));
        }
        assert!(events.is_empty());
        assert_eq!(INTERVAL, Duration::from_secs(5));
    }

    #[test]
    fn stalled_loop_records_thresholds_before_a_reply_then_recovers() {
        let (send, receive) = std::sync::mpsc::channel();
        let thresholds = [Duration::from_millis(1), Duration::from_millis(2), Duration::from_millis(3)];
        let worker = std::thread::spawn(move || probe(
            |ack| { send.send(Some(ack)).unwrap(); true }, &thresholds,
            |threshold, _, recovered| { send.send(None).unwrap(); assert!(threshold <= 3); if recovered { assert_eq!(threshold, 3); } },
        ));
        let ack = receive.recv().unwrap().unwrap();
        // Three threshold notifications arrive while the callback is withheld.
        for _ in 0..3 { assert!(receive.recv_timeout(Duration::from_secs(1)).unwrap().is_none()); }
        ack();
        assert!(receive.recv_timeout(Duration::from_secs(1)).unwrap().is_none());
        assert!(worker.join().unwrap());
    }

    #[test]
    fn platform_guard_names_all_five_shipped_targets() {
        let source = include_str!("main_thread_liveness.rs").split("#[cfg(test)]").next().unwrap();
        assert!(source.contains("#[cfg(any(target_os = \"linux\", target_os = \"windows\", target_os = \"macos\"))]"));
        assert!(source.contains("#[cfg(any(target_os = \"android\", target_os = \"ios\"))]"));
        assert!(source.contains("|acknowledge| app.run_on_main_thread(acknowledge).is_ok()"));
    }
}
