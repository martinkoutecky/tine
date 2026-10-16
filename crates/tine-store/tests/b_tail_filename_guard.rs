#[test]
fn filename_codec_is_shared_by_native_and_wasm() {
    let _integration_guard = crate::integration_lock();
    let model = include_str!("../../tine-core/src/model.rs");
    let wasm = include_str!("../../lsdoc-wasm/src/lib.rs");
    let refs = include_str!("../../../src/render/pageRefs.ts");
    assert!(model.contains("use tine_outline::page_filename;") && wasm.contains("use tine_outline::page_filename;") && refs.contains("reference_target_name(") && include_str!("../../tine-outline/src/block_regions.rs").contains("reference_filename::decode_page_name(stem, false)"), "I-12: tine-outline's page_filename.rs owns the codec; model.rs and lsdoc-wasm must use that crate module, and pageRefs.ts must call the shared target policy via wasm");
    assert!(
        !model.contains("fn push_percent_byte") && !refs.contains("replaceAll(\"___\""),
        "I-12: pageRefs.ts and model.rs must not carry a filename codec twin; use page_filename.rs"
    );
}
