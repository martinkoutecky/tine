#[test]
fn asset_mime_matches_the_browser_fixture_for_every_extension() {
    let fixtures: Vec<(String, String)> =
        serde_json::from_str(include_str!("fixtures/media-mime.json")).unwrap();
    for (ext, mime) in fixtures {
        for name in [
            format!("nested/voice.{ext}"),
            format!("žluťoučký/VOICE.{}", ext.to_ascii_uppercase()),
        ] {
            assert_eq!(tine_core::media_mime::from_path(&name), mime, "{name}");
        }
    }
}
