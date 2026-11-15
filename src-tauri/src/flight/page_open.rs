//! Closed, content-free facts about a refused physical page route (GH #863).
use super::*;

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Stage {
    Route,
    Probe,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Refusal {
    stage: Stage,
    read_source: Option<tine_store::PageReadSource>,
    path_mismatch: bool,
    name_mismatch: bool,
    kind_mismatch: bool,
}

pub(super) fn record(refusal: Option<Refusal>) {
    if let Some(refusal) = refusal {
        let source = refusal
            .read_source
            .map(|value| json!(value))
            .unwrap_or_else(|| json!("not-read"));
        let fields = Map::from_iter([
            ("stage".into(), json!(refusal.stage)),
            ("readSource".into(), source),
            ("pathMismatch".into(), json!(refusal.path_mismatch)),
            ("nameMismatch".into(), json!(refusal.name_mismatch)),
            ("kindMismatch".into(), json!(refusal.kind_mismatch)),
        ]);
        record_fixed_event("page.open-refused", fields);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusal_accepts_only_closed_sources_stages_and_boolean_operands() {
        let value = json!({ "stage": "route", "readSource": "held-absent",
            "pathMismatch": true, "nameMismatch": false, "kindMismatch": false });
        assert!(serde_json::from_value::<Refusal>(value.clone()).is_ok());
        for (field, invalid) in [
            ("readSource", json!("pages/private.md")),
            ("stage", json!("Private title")),
            ("pathMismatch", json!("private")),
            ("path", json!("pages/private.md")),
        ] {
            let mut bad = value.clone();
            bad[field] = invalid;
            assert!(serde_json::from_value::<Refusal>(bad).is_err());
        }
    }

    #[test]
    fn refusal_sources_and_operands_reach_the_existing_export() {
        for source in [
            Some("disk"),
            Some("held-bytes"),
            Some("held-absent"),
            Some("held-unindexed"),
            None,
        ] {
            for stage in ["route", "probe"] {
                let fields = json!({ "stage": stage, "readSource": source,
                    "pathMismatch": true, "nameMismatch": false, "kindMismatch": true });
                let refusal = serde_json::from_value(fields.clone()).unwrap();
                let mut expected = fields;
                expected["readSource"] = json!(source.unwrap_or("not-read"));
                diagnostic_frontend_event(
                    "page_open_refused".into(),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(refusal),
                    None,
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
                    .any(|event| event["event"] == "page.open-refused"
                        && expected
                            .as_object()
                            .unwrap()
                            .iter()
                            .all(|(key, value)| &event[key] == value)));
            }
        }
    }
}
