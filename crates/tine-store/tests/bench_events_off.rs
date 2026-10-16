//! Bench events off (`tine_store::bench_events`): with `TINE_BENCH_EVENTS`
//! unset, asking and emitting start no writer thread. A binary of its own
//! with one test, so no other thread starts in this process meanwhile.

/// This process's threads (Linux).
#[cfg(target_os = "linux")]
fn threads() -> usize {
    std::fs::read_dir("/proc/self/task").unwrap().count()
}

#[test]
fn unset_spawns_no_writer_thread() {
    let _integration_guard = crate::integration_lock();
    assert!(std::env::var_os("TINE_BENCH_EVENTS").is_none());
    #[cfg(target_os = "linux")]
    let before = threads();
    // The first question reads the variable; a writer would start here.
    assert!(!tine_store::bench_events::enabled());
    tine_store::bench_events::unfreeze(1.0);
    #[cfg(target_os = "linux")]
    assert_eq!(
        threads(),
        before,
        "a bench events writer thread runs with events off"
    );
}
