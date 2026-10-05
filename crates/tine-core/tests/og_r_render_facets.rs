//! Checkpoint-5 packet R (I-12): the render facts shared with the app.
use tine_core::doc;
use tine_core::render_facets::{
    is_render_hidden_prop, normalize, task_checkbox_state, RENDER_HIDDEN,
};

#[test]
fn built_in_keys_tine_and_table_settings_are_hidden() {
    for key in [
        "id",
        "Collapsed",
        "Created_At",
        "hl color",
        "tine.view",
        "logseq.table.version",
        "Title",
    ] {
        assert!(is_render_hidden_prop(key, &[]), "{key}");
    }
    for key in ["logseq.custom", "status", "tags", "alias", "public"] {
        assert!(!is_render_hidden_prop(key, &[]), "{key}");
    }
}

#[test]
fn the_graphs_hidden_list_applies_after_normalization() {
    let user = vec!["My_Prop".to_owned()];
    assert!(is_render_hidden_prop("my-prop", &user));
    assert!(!is_render_hidden_prop("other", &user));
}

#[test]
fn normalize_is_the_documents_property_key_norm() {
    for key in ["Id", " Hl_Color ", "A B_c", "\u{c9}COLE_x", "", "tine.View"] {
        assert_eq!(normalize(key), doc::property_key_norm(key), "{key:?}");
    }
}

#[test]
fn every_built_in_key_is_already_normalized() {
    for key in RENDER_HIDDEN {
        assert_eq!(normalize(key), *key);
    }
}

/// The same table `src/markers.test.ts` asserts for `taskCheckboxState`.
#[test]
fn task_checkbox_state_covers_exactly_the_document_markers() {
    for marker in doc::MARKERS {
        let expected = match *marker {
            "DONE" => Some(true),
            "CANCELED" | "CANCELLED" => None,
            _ => Some(false),
        };
        assert_eq!(task_checkbox_state(marker), expected, "{marker}");
    }
    assert_eq!(task_checkbox_state(""), None);
    assert_eq!(task_checkbox_state("done"), None);
    assert_eq!(task_checkbox_state("NOT-A-MARKER"), None);
}
