//! Closed, content-free facts when a requested block editor does not hold
//! focus one frame after the request (GH #939).
use super::*;

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Entry {
    EnterSplit,
    Other,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Guard {
    ExitGuard,
    EntryGuard,
    None,
}

/// `activeElement`'s tag; variants are named after the element so no source
/// scan mistakes one for a same-named type elsewhere.
#[derive(serde::Deserialize, serde::Serialize)]
pub(crate) enum Tag {
    #[serde(rename = "textarea")]
    TextareaElement,
    #[serde(rename = "input")]
    InputElement,
    #[serde(rename = "button")]
    ButtonElement,
    #[serde(rename = "select")]
    SelectElement,
    #[serde(rename = "iframe")]
    IframeElement,
    #[serde(rename = "a")]
    AnchorElement,
    #[serde(rename = "div")]
    DivElement,
    #[serde(rename = "span")]
    SpanElement,
    #[serde(rename = "body")]
    BodyElement,
    #[serde(rename = "none")]
    NoElement,
    #[serde(rename = "other")]
    OtherElement,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Missed {
    entry: Entry,
    document_has_focus: bool,
    guard: Guard,
    target_mounted: bool,
    active_tag: Tag,
    active_is_block_editor: bool,
}

pub(super) fn record(missed: Option<Missed>) {
    if let Some(missed) = missed {
        let fields = Map::from_iter([
            ("entry".into(), json!(missed.entry)),
            ("documentHasFocus".into(), json!(missed.document_has_focus)),
            ("guard".into(), json!(missed.guard)),
            ("targetMounted".into(), json!(missed.target_mounted)),
            ("activeTag".into(), json!(missed.active_tag)),
            (
                "activeIsBlockEditor".into(),
                json!(missed.active_is_block_editor),
            ),
        ]);
        record_fixed_event("editor.focus-missed", fields);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> Value {
        json!({ "entry": "enter-split", "documentHasFocus": false, "guard": "exit-guard",
            "targetMounted": true, "activeTag": "textarea", "activeIsBlockEditor": true })
    }

    #[test]
    fn missed_accepts_only_closed_tokens_and_booleans() {
        assert!(serde_json::from_value::<Missed>(valid()).is_ok());
        for (field, invalid) in [
            ("entry", json!("Private title")),
            ("guard", json!("pages/private.md")),
            ("activeTag", json!("my-private-element")),
            ("targetMounted", json!("block-uuid")),
            ("blockId", json!("6650a1b2-0000-0000-0000-000000000000")),
        ] {
            let mut bad = valid();
            bad[field] = invalid;
            assert!(serde_json::from_value::<Missed>(bad).is_err(), "{field}");
        }
    }

    #[test]
    fn missed_reaches_the_existing_export() {
        let missed = serde_json::from_value(valid()).unwrap();
        diagnostic_frontend_event(
            "editor_focus_missed".into(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(missed),
        );
        let report = build_diagnostic_report(
            None,
            Vec::new(),
            String::new(),
            String::new(),
            crate::defender::Realtime::NotApplicable,
        );
        let report: Value = serde_json::from_str(&report.text).unwrap();
        let events = report["sessions"]["current"].as_array().unwrap();
        assert!(events
            .iter()
            .any(|event| event["event"] == "editor.focus-missed"
                && valid()
                    .as_object()
                    .unwrap()
                    .iter()
                    .all(|(key, value)| &event[key] == value)));
    }
}
