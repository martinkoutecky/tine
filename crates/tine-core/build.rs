// The refused link schemes (`src-tauri/external-link-schemes.json`, its
// `refused` array), compiled into core, the one answerer of membership. The
// TypeScript policy imports the same file.
fn main() {
    let file = "../../src-tauri/external-link-schemes.json";
    println!("cargo:rerun-if-changed={file}");
    let list: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(file).expect(file)).expect(file);
    let refused: Vec<&str> = list["refused"]
        .as_array()
        .expect("refused")
        .iter()
        .map(|name| name.as_str().expect("refused name"))
        .collect();
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("build output"));
    std::fs::write(
        out.join("refused_schemes.rs"),
        format!("const REFUSED_SCHEMES: &[&str] = &{refused:?};\n"),
    )
    .expect("generated refused schemes");
}
