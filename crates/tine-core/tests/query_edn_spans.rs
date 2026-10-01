use tine_core::query_edn::{self, Kind};

#[test]
fn spans_and_discards_do_not_shift_map_pairs() {
    let source = r##"{:x #_ :title [:title "keep"] #_ [:title "discard"] :title #_ "old" "café"}"##;
    let form = query_edn::read(source).unwrap();
    assert_eq!(form.kind, Kind::Map);
    assert_eq!(form.children.len(), 4);
    assert_eq!(&source[form.children[2].span.clone()], ":title");
    assert_eq!(
        query_edn::options(source).unwrap().title.as_deref(),
        Some("café")
    );
    assert_eq!(
        query_edn::edit_title(source, "new").unwrap(),
        source.replace("\"café\"", "\"new\"")
    );
    let nested_discard = "{:x #_ #_ :discard :also-discard 7 :title \"real\"}";
    assert_eq!(
        query_edn::options(nested_discard).unwrap().title.as_deref(),
        Some("real")
    );
}

#[test]
fn direct_title_splices_preserve_authored_bytes() {
    let source =
        "{ :x [:title \"keep\"] ; comment\n :title #_ \"discard\" \"old\" , :y #uuid \"abc\" }";
    assert_eq!(
        query_edn::edit_title(source, "é😀\n\t\\\"").unwrap(),
        source.replace("\"old\"", "\"é😀\\n\\t\\\\\\\"\"")
    );
    let removed = query_edn::edit_title(source, "").unwrap();
    assert_eq!(
        removed,
        "{ :x [:title \"keep\"] ; comment\n  #_ \"discard\"  , :y #uuid \"abc\" }"
    );
    assert_eq!(
        query_edn::edit_title("{:x [:title \"keep\"]}", "new").unwrap(),
        "{:title \"new\" :x [:title \"keep\"]}"
    );
    assert_eq!(
        query_edn::options("{:x [:title \"keep\" :collapsed? true :table-view? true]}")
            .unwrap()
            .title,
        None
    );
    assert!(
        !query_edn::options("{:x [:collapsed? true]}")
            .unwrap()
            .collapsed
    );
}

#[test]
fn malformed_ambiguous_or_deep_options_refuse() {
    for source in [
        "{:x}",
        "{:x [)}",
        "{:title \"\\q\"}",
        "{:x #_}",
        "{:title 1 :title 2}",
        "{:x 1} trailing",
        "{:x #{1 2}",
        "{:x #uuid}",
    ] {
        assert!(
            query_edn::edit_title(source, "new").is_none(),
            "accepted {source}"
        );
    }
    let deep = format!("{{:x {}1{}}}", "[".repeat(128), "]".repeat(128));
    assert!(query_edn::edit_title(&deep, "new").is_none());
    let huge = format!("{{:x \"{}\"}}", "a".repeat(1024 * 1024));
    assert!(query_edn::edit_title(&huge, "new").is_none());
    assert_eq!(query_edn::read("#{1 2 3}").unwrap().children.len(), 3);
}

#[test]
fn native_macro_printer_refuses_unreadable_options() {
    use tine_core::query::ir::DiagnosticKind;
    use tine_core::query::macro_text::{macro_safe, FormFamily};
    // TQL's lexical boundary locates the malformed trailing map; the same
    // shared reader then refuses it at native macro serialization.
    let error = macro_safe("@block {:title \"\\q\"}", FormFamily::Tql).unwrap_err();
    assert_eq!(error.kind, DiagnosticKind::Syntax);
    assert!(error.message.contains("unreadable EDN"));
}

#[test]
fn begin_query_inspector_preserves_live_payload_semantics() {
    use query_edn::BeginQueryMatch::{Supported, Unsupported};
    let query = "[:find (pull ?b [*]) :where (task ?b \"TODO\")]";
    let payload = format!(
        r#"{{#_ :discarded :title "Line\nTwo\t\u03bb" :query {query} :inputs [:current-page]}}"#
    );
    assert_eq!(
        query_edn::inspect_begin_query(&payload),
        Supported {
            query: format!("{query} :inputs [:current-page]"),
            title: Some("Line\nTwo\tλ".into()),
        }
    );
    for (entry, reason) in [
        (":inputs nope", "expected :inputs to be a vector"),
        (":inputs [] :inputs []", "expected :inputs to be a vector"),
        (":title 1", "expected :title to be a string"),
        (
            ":title \"one\" :title \"two\"",
            "expected :title to be a string",
        ),
        (":query []", "duplicate :query entry"),
    ] {
        assert_eq!(
            query_edn::inspect_begin_query(&format!("{{:query {query} {entry}}}")),
            Unsupported { reason }
        );
    }
    assert_eq!(
        query_edn::inspect_begin_query(&format!(
            r#"{{#_ [:title "hidden"] :nested {{:title "nested"}} :query {query}}}"#
        )),
        Supported {
            query: query.into(),
            title: None
        }
    );
}
