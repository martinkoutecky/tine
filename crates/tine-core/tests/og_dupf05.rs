use tine_core::{query::view::canonical_group_field, refs::page_key};

#[test]
fn native_unicode_name_and_group_policy() {
    assert_eq!(page_key("\u{85}/CAFÉ/\u{85}"), "café");
    assert_eq!(page_key("\u{feff}Foo\u{feff}"), "\u{feff}foo\u{feff}");
    assert_eq!(page_key(" ΟΣ "), "ος");
    assert_eq!(page_key("/Cafe\u{301}/"), "café");
    assert_eq!(page_key("//Foo//"), "/foo/");
    assert_eq!(
        canonical_group_field("\u{85}state\u{85}").unwrap().as_str(),
        "state"
    );
    assert!(canonical_group_field("\u{feff}state\u{feff}").is_none());
    for token in [
        "state",
        "priority",
        "scheduled",
        "deadline",
        "tags",
        "page",
        "prop:state",
        "formula:x",
    ] {
        assert_eq!(
            canonical_group_field(&format!("  {token}  "))
                .unwrap()
                .as_str(),
            token
        );
    }
    for token in [
        "",
        "prop:",
        "formula:",
        "other",
        "prop:x\ninside",
        "prop:x\0inside",
    ] {
        assert!(canonical_group_field(token).is_none());
    }
}
