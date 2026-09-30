//! The read_highlights command worker boundary: keep native panics inside a
//! join result. O(asset entries + sidecar bytes); preserves feature errors and stringifies a
//! worker join error. The caller retains the graph binding for the whole read.

pub(super) async fn read(
    slot: std::sync::Arc<crate::state::GraphSlot>,
    pdf: String,
) -> Result<Vec<tine_core::pdf::Highlight>, String> {
    run(move || {
        tine_graph_features::pdf::read_highlights_checked(&slot.store, &pdf)
            .map_err(super::feature_pdf_error)
    })
    .await
}

pub(super) async fn run<F>(read: F) -> Result<Vec<tine_core::pdf::Highlight>, String>
where
    F: FnOnce() -> Result<Vec<tine_core::pdf::Highlight>, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(read)
        .await
        .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_highlights_worker_keeps_success_errors_and_panics_in_results() {
        tauri::async_runtime::block_on(async {
            assert!(run(|| Ok(Vec::new())).await.unwrap().is_empty());
            assert_eq!(
                run(|| Err("malformed sidecar".into())).await.unwrap_err(),
                "malformed sidecar"
            );
            let error = run(|| panic!("read-highlights-fixture-panic"))
                .await
                .unwrap_err();
            assert!(error.contains("read-highlights-fixture-panic"), "{error}");
        });
    }
}
