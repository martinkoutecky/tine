//! Direct page-commit regression tests live inside tine-store. Feature crates
//! can only construct PagePlan and hand it to the host.
use crate::Store;
fn integration_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

// The alias cost probe uses process-global counters. Its old integration
// harness serialized every case; isolate it from unrelated library tests.
fn isolated_cost_case(name: &str) -> bool {
    const CASE: &str = "TINE_PRIVATE_TRANSACTION_COST_CASE";
    if std::env::var(CASE).as_deref() == Ok(name) {
        return false;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--test-threads=1", "--nocapture"])
        .env(CASE, name)
        .output()
        .expect("start isolated transaction cost probe");
    assert!(
        output.status.success(),
        "{name}: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}
#[path = "../tests/answer_changes.rs"]
mod answer_changes;
#[path = "../tests/c3s_content_loss.rs"]
mod c3s_content_loss;
#[path = "../tests/change_summary.rs"]
mod change_summary;
#[path = "../tests/conformance_fixture.rs"]
mod conformance_f3;
#[path = "../tests/f8_vcs_markers.rs"]
mod f8_vcs_markers;
#[path = "../tests/lifecycle.rs"]
mod lifecycle;
#[path = "../tests/linkable_page_property_refs.rs"]
mod linkable_page_property_refs;
#[path = "../tests/loading_own_save.rs"]
mod loading_own_save;
#[path = "../tests/og_create_unreadable_owner.rs"]
mod og_create_unreadable_owner;
#[path = "../tests/store_read.rs"]
mod store_read;
#[path = "../tests/store_save.rs"]
mod store_save;
#[path = "../tests/transaction.rs"]
mod transaction;
#[path = "../tests/transaction_plan.rs"]
mod transaction_plan;
#[path = "../tests/watch.rs"]
mod watch;

#[path = "../tests/internal_crash_transactions.rs"]
mod i02_crash_gaps;
#[path = "../tests/internal_case_transactions.rs"]
mod og_gh609_case_rename;

#[path = "../tests/alias_edit_cost.rs"]
mod alias_edit_cost;
