//! Test-only external-editor barrier. Never built in production.
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

pub(crate) fn before_apply(path: &Path) {
    let Ok(target) = std::env::var("TINE_K1_RACE_TARGET") else {
        return;
    };
    if path != Path::new(&target) {
        return;
    }
    let barrier = std::env::var("TINE_K1_RACE_BARRIER").unwrap();
    fs::write(format!("{barrier}.ready"), b"preflight complete").unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !Path::new(&format!("{barrier}.resume")).exists() {
        assert!(
            Instant::now() < deadline,
            "external-editor race barrier timed out"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
