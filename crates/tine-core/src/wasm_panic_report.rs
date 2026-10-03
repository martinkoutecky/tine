//! The message the lsdoc-wasm panic hook keeps and logs (crates/lsdoc-wasm/src/lib.rs
//! includes this file by path). I-5: a formatted panic message can embed page text (str
//! slicing panics quote the string), and the message reaches the diagnostics log, so only
//! the location, which names the site, and a fixed literal message are kept. Lives here so
//! `cargo test --workspace` exercises it: the wasm crate is excluded from the workspace, and
//! lsdoc v0.5.8 left no reachable formatted panic to drive through the wasm door.

/// `place` is `file:line` ("" when unknown); `payload` is the panic payload.
pub fn panic_report(place: &str, payload: &(dyn std::any::Any + Send)) -> String {
    let literal = payload
        .downcast_ref::<&str>()
        .copied()
        .unwrap_or("<formatted message withheld>");
    format!("lsdoc-wasm panic at {place}: {literal}")
}

#[cfg(test)]
mod tests {
    use super::panic_report;

    #[test]
    fn keeps_a_literal_message() {
        let payload: Box<dyn std::any::Any + Send> = Box::new("quote nesting too deep");
        assert_eq!(
            panic_report("src/v2/mod.rs:26", &*payload),
            "lsdoc-wasm panic at src/v2/mod.rs:26: quote nesting too deep"
        );
    }

    #[test]
    fn withholds_a_formatted_message() {
        let secret = "private page text";
        let payload: Box<dyn std::any::Any + Send> = Box::new(format!("does not own {secret:?}"));
        let report = panic_report("src/v2/mod.rs:26", &*payload);
        assert_eq!(
            report,
            "lsdoc-wasm panic at src/v2/mod.rs:26: <formatted message withheld>"
        );
        assert!(!report.contains(secret));
    }

    #[test]
    fn the_real_hook_payload_of_a_formatted_panic_is_withheld() {
        let secret = "private page text";
        let payload = std::panic::catch_unwind(|| panic!("does not own {secret:?}")).unwrap_err();
        assert!(!panic_report("x:1", &*payload).contains(secret));
    }
}
