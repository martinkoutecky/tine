//! Process-local capture claims survive WebView reloads. Claim before the JS
//! insertion: an interrupted attempt remains uncertain, never permission to
//! insert again. No graph bytes, visible properties or persisted records.
use crate::state::{slot_for_context, GraphContext};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

#[derive(Default)]
pub(crate) struct CaptureReceipts(Mutex<ReceiptBook>);

#[derive(Default)]
struct ReceiptBook {
    entries: HashMap<(String, String), (u64, bool)>,
    completed: VecDeque<(String, String)>,
}

impl CaptureReceipts {
    fn apply(
        &self,
        window: &str,
        id: String,
        generation: u64,
        action: &str,
    ) -> Result<&'static str, String> {
        let mut book = self.0.lock().unwrap();
        let key = (window.to_owned(), id);
        match action {
            "status" => Ok(match book.entries.get(&key) {
                Some((_, true)) => "saved",
                Some(_) => "pending",
                None => "missing",
            }),
            "claim" => match book.entries.get(&key) {
                Some((_, true)) => Ok("saved"),
                Some(_) => Ok("pending"),
                None => {
                    book.entries.insert(key, (generation, false));
                    Ok("new")
                }
            },
            "saved" | "release" => {
                let Some((prior, saved)) = book.entries.get_mut(&key) else {
                    return Err("missing capture claim".into());
                };
                if *prior != generation {
                    return Err("stale capture claim".into());
                }
                if action == "saved" {
                    if !*saved {
                        *saved = true;
                        book.completed.push_back(key);
                        // As in the receiver, only proved completions have a
                        // cap. An unconfirmed sender cannot advance its id.
                        while book.completed.len() > 100 {
                            if let Some(old) = book.completed.pop_front() {
                                book.entries.remove(&old);
                            }
                        }
                    }
                } else if !*saved {
                    book.entries.remove(&key);
                }
                Ok("saved")
            }
            _ => Err("unknown capture receipt action".into()),
        }
    }
}

// Reviewed synchronous command: registry lookup and process-memory bookkeeping
// only, amortized O(1); no store writer, file read, fsync or graph-size work.
#[tauri::command]
pub(crate) fn capture_receipt(
    id: String,
    action: String,
    receipt_generation: u64,
    ctx: GraphContext<'_>,
    receipts: tauri::State<'_, CaptureReceipts>,
) -> Result<&'static str, String> {
    let slot = slot_for_context(&ctx)?;
    if action != "status" && slot.binding_generation != receipt_generation {
        return Err(crate::state::STALE_BINDING.into());
    }
    receipts.apply(ctx.window.label(), id, receipt_generation, &action)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reload_and_rebind_do_not_turn_an_existing_claim_into_a_new_insertion() {
        let receipts = CaptureReceipts::default();
        assert_eq!(
            receipts.apply("main", "id".into(), 12, "status").unwrap(),
            "missing"
        );
        assert_eq!(
            receipts.apply("main", "id".into(), 12, "status").unwrap(),
            "missing"
        );
        assert_eq!(
            receipts.apply("main", "id".into(), 11, "claim").unwrap(),
            "new"
        );
        assert_eq!(
            receipts.apply("main", "id".into(), 11, "claim").unwrap(),
            "pending"
        );
        assert_eq!(
            receipts.apply("main", "id".into(), 12, "claim").unwrap(),
            "pending"
        );
        assert_eq!(
            receipts.apply("main", "id".into(), 12, "status").unwrap(),
            "pending"
        );
        assert!(receipts.apply("main", "id".into(), 12, "release").is_err());
        receipts.apply("main", "id".into(), 11, "saved").unwrap();
        assert_eq!(
            receipts.apply("main", "id".into(), 12, "status").unwrap(),
            "saved"
        );
        assert_eq!(
            receipts.apply("main", "id".into(), 11, "claim").unwrap(),
            "saved"
        );
    }
}
