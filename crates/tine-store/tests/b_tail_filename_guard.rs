#[test]
fn filename_codec_is_shared_by_native_and_wasm() {
    let model = include_str!("../../tine-core/src/model.rs");
    let wasm = include_str!("../../lsdoc-wasm/src/lib.rs");
    let refs = include_str!("../../../src/render/pageRefs.ts");
    assert!(model.contains("mod page_filename;") && wasm.contains("../../tine-core/src/page_filename.rs") && refs.contains("decode_page_name("), "I-12: page_filename.rs owns the codec; model.rs and lsdoc-wasm must use that source, and pageRefs.ts must call its wasm export");
    assert!(
        !model.contains("fn push_percent_byte") && !refs.contains("replaceAll(\"___\""),
        "I-12: pageRefs.ts and model.rs must not carry a filename codec twin; use page_filename.rs"
    );
}
