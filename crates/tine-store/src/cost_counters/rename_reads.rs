//! Uncommitted, opt-in investigation instrumentation; not a production policy.
#![allow(missing_docs)]
use std::sync::{atomic::{AtomicBool, Ordering}, Mutex};
use std::time::Instant;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static EVENTS: Mutex<Vec<(String, u128, String)>> = Mutex::new(Vec::new());

pub fn begin() {
    let _ = tine_core::render::RENAME_READS_HOOK.set(|name| event(name));
    EVENTS.lock().unwrap().clear();
    ACTIVE.store(true, Ordering::Relaxed);
}

pub fn event(name: impl Into<String>) {
    if ACTIVE.load(Ordering::Relaxed) {
        EVENTS.lock().unwrap().push((name.into(), 0, std::thread::current().name().unwrap_or("unnamed").into()));
    }
}

pub struct Span(Option<(String, Instant)>);
pub fn span(name: impl Into<String>) -> Span {
    Span(ACTIVE.load(Ordering::Relaxed).then(|| (name.into(), Instant::now())))
}
impl Drop for Span {
    fn drop(&mut self) {
        if let Some((name, start)) = self.0.take() {
            EVENTS.lock().unwrap().push((name, start.elapsed().as_nanos(), std::thread::current().name().unwrap_or("unnamed").into()));
        }
    }
}

pub fn finish(label: &str) {
    ACTIVE.store(false, Ordering::Relaxed);
    for (name, ns, thread) in EVENTS.lock().unwrap().drain(..) {
        println!("RR\t{label}\t{name}\t{ns}\t{thread}");
    }
}
