use serde::Deserialize;

#[derive(Deserialize)]
struct Input {
    path: String,
    format: String,
    source: String,
}

#[derive(Deserialize)]
struct Golden {
    path: String,
    bytes: String,
}

#[test]
fn page_serializer_matches_master_ddf408c55_for_ten_tricky_pages() {
    let inputs: Vec<Input> = serde_json::from_str(include_str!(
        "../../../scripts/fixtures/og-differential-pages.json"
    ))
    .unwrap();
    let goldens: Vec<Golden> = serde_json::from_str(include_str!(
        "../../../scripts/fixtures/og-master-pages-golden.json"
    ))
    .unwrap();
    assert_eq!(inputs.len(), 10);
    assert_eq!(goldens.len(), inputs.len());
    let mut differences = Vec::new();
    for (input, golden) in inputs.iter().zip(goldens) {
        assert_eq!(input.path, golden.path);
        let actual = if input.format == "org" {
            tine_core::org::serialize_org_detect(
                &tine_core::org::parse_org(&input.source),
                Some(&input.source),
            )
        } else {
            tine_core::doc::serialize_with(
                &tine_core::doc::parse(&input.source),
                &tine_core::doc::SerializeOpts::detect(Some(&input.source)),
            )
        };
        if actual.as_bytes() != golden.bytes.as_bytes() {
            differences.push((input.path.clone(), actual, golden.bytes));
        }
    }
    // Baseline difference, documented in docs/og-differential.md. Keep the
    // exact bytes pinned so a future change cannot silently enlarge the gap.
    assert_eq!(
        differences.len(),
        1,
        "unexpected master byte differences: {differences:?}"
    );
    assert_eq!(differences[0].0, "pages/Whitespace.md");
    assert_eq!(differences[0].1.as_bytes(), b"- first\n\n- second  \n\n");
    assert_eq!(differences[0].2.as_bytes(), b"\n- first\n\n- second  \n\n");
}
