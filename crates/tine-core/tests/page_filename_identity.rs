use std::collections::HashSet;
use tine_core::config::FileNameFormat;
use tine_core::model::{decode_page_name, encode_page_name};

#[test]
fn new_page_filenames_are_reversible_injective_and_windows_safe() {
    let titles = [
        "a/b",
        "a___b",
        "a_/b",
        "%",
        "%2F",
        "a:b",
        "CON",
        "PRN.txt",
        "LPT³",
        "trailing dot.",
        "trailing space ",
        ".hidden",
        "Release 1.0",
    ];
    for format in [FileNameFormat::Legacy, FileNameFormat::TripleLowbar] {
        let mut seen = HashSet::new();
        for title in titles {
            let stem = encode_page_name(title, format);
            assert_eq!(decode_page_name(&stem, format), title, "{title:?}");
            assert!(seen.insert(stem.clone()), "noninjective {title:?}");
            assert!(!stem.ends_with([' ', '.']), "unsafe suffix: {stem}");
            assert!(!stem
                .chars()
                .any(|ch| matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')));
            let body = stem.split('.').next().unwrap().to_uppercase();
            assert!(!matches!(
                body.as_str(),
                "CON" | "PRN" | "AUX" | "NUL" | "LPT³"
            ));
        }
    }
}
