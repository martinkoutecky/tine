// Lists compiled into core from the checked-in JSON they live in, so the wasm
// modules carry neither the JSON text nor a reader for it:
// - the refused link schemes (`src-tauri/external-link-schemes.json`, its
//   `refused` array); core is the one answerer of membership and the
//   TypeScript policy imports the same file;
// - the colour rule's style properties (`fixtures/html-sanitize-policy.json`,
//   `browser.styleProperties`).
fn main() {
    let refused = strings("../../src-tauri/external-link-schemes.json", &["refused"]);
    let style = strings(
        "../../fixtures/html-sanitize-policy.json",
        &["browser", "styleProperties"],
    );
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("build output"));
    for (file, name, list) in [
        ("refused_schemes.rs", "REFUSED_SCHEMES", refused),
        ("style_properties.rs", "STYLE_PROPERTIES", style),
    ] {
        std::fs::write(
            out.join(file),
            format!("const {name}: &[&str] = &{list:?};\n"),
        )
        .expect(file);
    }
}

/// The string array at `path` in the JSON file `file`.
fn strings(file: &str, path: &[&str]) -> Vec<String> {
    println!("cargo:rerun-if-changed={file}");
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(file).expect(file)).expect(file);
    let list = path.iter().fold(&json, |value, key| &value[key]);
    list.as_array()
        .unwrap_or_else(|| panic!("{file}: {path:?} is not an array"))
        .iter()
        .map(|name| name.as_str().expect("a string").to_owned())
        .collect()
}
