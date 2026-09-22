use super::*;
use std::time::{Duration, Instant};

// DUP mapping semantics (2026-08-25 duplication audit): both tree walkers
// delegate every block to the one shared field mapping, so identical DTO
// input must produce identical parseable trees (identity, format flag,
// fresh lazy projection included).

// DUP-7 (2026-08-25 duplication audit): the block-level property recognizer
// (`doc::parse_property_line`, an lsdoc transcription) and this page-HEADER
// recognizer are deliberately different grammars — the header must never
// promote a property-looking prose line into page metadata, so its keys sit
// at column zero and its values stay verbatim; the block rule follows lsdoc
// (leading parser spaces skipped, `::` must be followed by a space or end,
// value trimmed). Do not unify them; pin the distinction.
#[test]
fn page_header_rule_stays_deliberately_distinct_from_block_rule() {
    // Column zero: the block rule (like lsdoc) tolerates indent; the header
    // does not — an indented property-looking line is content, not metadata.
    assert_eq!(
        crate::doc::parse_property_line(" key:: v"),
        Some(("key", "v"))
    );
    assert_eq!(page_header_property_line(" key:: v"), None);
    // Separator spacing: lsdoc requires a space after `::` (or line end);
    // the header rule keeps verbatim values so `title::x` still rewrites.
    assert_eq!(crate::doc::parse_property_line("key::value"), None);
    assert_eq!(
        page_header_property_line("key::value"),
        Some(("key", "value"))
    );
    // Verbatim vs trimmed value.
    assert_eq!(
        page_header_property_line("title:: Name"),
        Some(("title", " Name"))
    );
    // Both take Unicode and dotted keys; neither takes a space in the key.
    for line in ["kéy:: v", "logseq.order-list-type:: number"] {
        assert!(crate::doc::parse_property_line(line).is_some(), "{line}");
        assert!(page_header_property_line(line).is_some(), "{line}");
    }
    assert_eq!(crate::doc::parse_property_line("a b:: v"), None);
    assert_eq!(page_header_property_line("a b:: v"), None);
}

// I-12: every production `DocBlock` literal is a reviewed constructor boundary.
// This is syntax-aware so a renamed `uuid: dto.id.clone()` mapping cannot evade
// the old substring check. Wire DTO conversions use
// `dto_block_to_doc_block`; cheap raw-only leaves use `DocBlock::new`.
#[test]
fn production_docblock_struct_literals_are_reviewed() {
    use syn::visit::{self, Visit};

    #[derive(Default)]
    struct Literals {
        owner: Option<String>,
        owners: std::collections::BTreeSet<String>,
    }
    fn test_only(attributes: &[syn::Attribute]) -> bool {
        attributes.iter().any(|attribute| {
            attribute.path().is_ident("test")
                || (attribute.path().is_ident("cfg")
                    && matches!(&attribute.meta, syn::Meta::List(list) if list.tokens.to_string().contains("test")))
        })
    }
    impl<'ast> Visit<'ast> for Literals {
        fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
            if test_only(&item.attrs) {
                return;
            }
            let previous = self.owner.replace(item.sig.ident.to_string());
            visit::visit_item_fn(self, item);
            self.owner = previous;
        }

        fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
            if test_only(&item.attrs) {
                return;
            }
            let previous = self.owner.replace(item.sig.ident.to_string());
            visit::visit_impl_item_fn(self, item);
            self.owner = previous;
        }

        fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
            if !test_only(&item.attrs) {
                visit::visit_item_mod(self, item);
            }
        }

        fn visit_expr_struct(&mut self, expression: &'ast syn::ExprStruct) {
            if expression
                .path
                .segments
                .last()
                .is_some_and(|part| part.ident == "DocBlock")
            {
                self.owners.insert(
                    self.owner
                        .clone()
                        .expect("a production DocBlock literal has an enclosing item"),
                );
            }
            visit::visit_expr_struct(self, expression);
        }
    }

    let allowed = [
        (
            "crates/tine-core/src/doc.rs",
            "clone",
            "Clone resets the lazy projection memo",
        ),
        (
            "crates/tine-core/src/doc.rs",
            "new",
            "DocBlock::new is the raw-only canonical constructor",
        ),
        (
            "crates/tine-core/src/vocab/block_dto.rs",
            "dto_block_to_doc_block",
            "the one BlockDto field mapping",
        ),
        (
            "crates/tine-core/src/pdf.rs",
            "highlight_block",
            "PDF import constructs parser-native highlight blocks",
        ),
        (
            "crates/tine-core/src/query.rs",
            "property_projection",
            "page pre-block projection has no BlockDto source",
        ),
    ];
    let reasons = allowed
        .iter()
        .map(|(file, owner, reason)| ((*file, *owner), *reason))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut actual = Vec::new();
    for file in crate::projection_producer_census::production_rust() {
        let syntax = syn::parse_file(&file.raw)
            .unwrap_or_else(|error| panic!("{} is valid Rust: {error}", file.relative));
        let mut literals = Literals::default();
        literals.visit_file(&syntax);
        for owner in literals.owners {
            let reason = reasons
                .get(&(file.relative.as_str(), owner.as_str()))
                .copied()
                .unwrap_or("UNCLASSIFIED: use dto_block_to_doc_block or DocBlock::new");
            actual.push((file.relative.as_str(), owner, reason));
        }
    }
    actual.sort();
    let mut expected = allowed
        .into_iter()
        .map(|(file, owner, reason)| (file, owner.to_owned(), reason))
        .collect::<Vec<_>>();
    expected.sort();
    assert_eq!(
        actual, expected,
        "I-12: every production DocBlock literal is reviewed; wire DTO mappings must use dto_block_to_doc_block and raw-only leaves must use DocBlock::new"
    );
}

// Direct Files performance audit 2026-08-09, finding F7. The digest exists to
// let the frontend skip transporting several thousand names it already has,
// so it has exactly two jobs: never report a change that did not happen (the
// memo's order comes from a HashMap, so a sequence-dependent hash would
// report one on every rebuild and the gate would save nothing), and never
// miss one (which would strand a stale inventory in autocomplete).
#[test]
fn the_reference_digest_ignores_order_and_notices_every_real_change() {
    let names = |values: &[&str]| -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    };
    let base = names(&["Alpha", "Beta", "Gamma"]);
    let shuffled = names(&["Gamma", "Alpha", "Beta"]);
    assert_eq!(
        referenced_names_digest(&base),
        referenced_names_digest(&shuffled),
        "a reordered set is the same set"
    );

    for changed in [
        names(&["Alpha", "Beta"]),                   // removed
        names(&["Alpha", "Beta", "Gamma", "Delta"]), // added
        names(&["Alpha", "Beta", "gamma"]),          // recased
        names(&["Alpha", "Beta", "Gamma", "Gamma"]), // duplicated
        Vec::new(),                                  // emptied
    ] {
        assert_ne!(
            referenced_names_digest(&base),
            referenced_names_digest(&changed),
            "{changed:?} must not be mistaken for {base:?}"
        );
    }
}

#[test]
fn a_caller_holding_the_current_reference_set_is_not_sent_it_again() {
    let names = vec!["Alpha".to_owned(), "Beta".to_owned()];
    let digest = referenced_names_digest(&names);

    let first = ReferencedPageNames::answer(digest, &names, None);
    assert_eq!(first.digest, digest);
    assert_eq!(first.names.as_deref(), Some(&names[..]));

    let repeat = ReferencedPageNames::answer(digest, &names, Some(digest));
    assert_eq!(repeat.names, None, "the caller already has this set");

    let stale = ReferencedPageNames::answer(digest, &names, Some(digest ^ 1));
    assert_eq!(
        stale.names.as_deref(),
        Some(&names[..]),
        "a caller naming any other set must be sent the current one"
    );
}

#[test]
fn page_name_encoding_round_trips_both_formats() {
    // Legacy: `/` ↔ `%2F`; a literal `___` is NOT a separator (stays put).
    let leg = FileNameFormat::Legacy;
    assert_eq!(encode_page_name("a/b/c", leg), "a%2Fb%2Fc");
    assert_eq!(decode_page_name("a%2Fb%2Fc", leg), "a/b/c");
    assert_eq!(decode_page_name("Foo.Bar", leg), "Foo/Bar");
    assert_eq!(encode_page_name("a___b", leg), "a___b");
    assert_eq!(decode_page_name("a___b", leg), "a___b");

    // Triple-lowbar: `/` ↔ `___`; a literal `___` is disambiguated via `%5F`
    // so it survives the round-trip (and isn't read back as a separator).
    let tlb = FileNameFormat::TripleLowbar;
    assert_eq!(encode_page_name("a/b/c", tlb), "a___b___c");
    assert_eq!(decode_page_name("a___b___c", tlb), "a/b/c");
    assert_eq!(encode_page_name("a___b", tlb), "a%5F%5F%5Fb");
    assert_eq!(decode_page_name("a%5F%5F%5Fb", tlb), "a___b");
    // `_` adjacent to the separator round-trips too.
    assert_eq!(
        decode_page_name(&encode_page_name("a_/b", tlb), tlb),
        "a_/b"
    );
    assert_eq!(
        decode_page_name(&encode_page_name("x/_y", tlb), tlb),
        "x/_y"
    );

    // The cross-format hazard the fix addresses: a legacy `%2F` file is read
    // as a namespace ONLY under legacy; a triple-lowbar `___` file ONLY under
    // triple-lowbar — each matching its OG counterpart.
    assert_eq!(decode_page_name("math%2Falgebra", leg), "math/algebra");
    assert_eq!(decode_page_name("math.algebra", leg), "math/algebra");
    assert_eq!(decode_page_name("math___algebra", tlb), "math/algebra");
    assert_eq!(decode_page_name("math.algebra", tlb), "math.algebra");
    // A unicode percent-escape decodes (UTF-8 aware), like OG.
    assert_eq!(decode_page_name("caf%C3%A9", leg), "café");
    // Reserved-character spelling follows OG's percent syntax in both modes;
    // literal escapes are pre-escaped so decoding is single-pass and exact.
    assert_eq!(
        encode_page_name("2026-07-23_18:01:20", leg),
        "2026-07-23_18%3A01%3A20"
    );
    assert_eq!(
        encode_page_name("2026-07-23_18:01:20", tlb),
        "2026-07-23_18%3A01%3A20"
    );
    assert_eq!(encode_page_name("%2F", leg), "%252F");
    assert_eq!(encode_page_name("%2F", tlb), "%252F");
}

fn assert_windows_safe_page_stem(stem: &str) {
    assert!(!stem.is_empty(), "page filename stem is empty");
    assert!(
        !stem.chars().any(|character| character <= '\u{1f}'
            || matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            )),
        "page filename stem is not Windows-safe: {stem:?}"
    );
    assert!(
        !stem.ends_with([' ', '.']),
        "page filename stem has a Windows-illegal suffix: {stem:?}"
    );
    let device_body = stem.split('.').next().unwrap_or(stem).to_ascii_uppercase();
    assert!(
        !matches!(
            device_body.as_str(),
            "CON"
                | "PRN"
                | "AUX"
                | "NUL"
                | "COM1"
                | "COM2"
                | "COM3"
                | "COM4"
                | "COM5"
                | "COM6"
                | "COM7"
                | "COM8"
                | "COM9"
                | "LPT1"
                | "LPT2"
                | "LPT3"
                | "LPT4"
                | "LPT5"
                | "LPT6"
                | "LPT7"
                | "LPT8"
                | "LPT9"
                | "COM¹"
                | "COM²"
                | "COM³"
                | "LPT¹"
                | "LPT²"
                | "LPT³"
        ),
        "page filename stem is a Windows DOS device name: {stem:?}"
    );
}

#[test]
fn page_name_encoding_is_injective_reversible_and_windows_safe() {
    let titles = [
        "2026-07-23_18:01:20",
        "reserved < > : \\ | ? * \" #",
        "trailing dot.",
        "trailing space ",
        ".hidden",
        ".",
        "..",
        "CON",
        "con",
        "PRN.txt",
        "Lpt9.report",
        "COM¹.txt",
        "LPT³",
        "%",
        "%2F",
        "%3a",
        "%25",
        "%ZZ",
        "a/b",
        "a___b",
        "a_/b",
        "x/_y",
        "Release 1.0",
        "café",
        "cafe\u{301}",
        "日本語 📝",
        "control\u{1f}character",
    ];
    for format in [FileNameFormat::Legacy, FileNameFormat::TripleLowbar] {
        let mut stems = std::collections::HashMap::new();
        for title in titles {
            let stem = encode_page_name(title, format);
            assert_windows_safe_page_stem(&stem);
            assert_eq!(
                decode_page_name(&stem, format),
                title,
                "filename codec did not round-trip {title:?} under {format:?}"
            );
            assert_eq!(
                stems.insert(stem.clone(), title),
                None,
                "distinct titles collided at {stem:?} under {format:?}"
            );
        }
    }
}

#[test]
fn unsafe_page_titles_save_and_reopen_through_one_safe_identity() {
    for (label, config, format) in [
        ("legacy", "", FileNameFormat::Legacy),
        (
            "triple-lowbar",
            "{:file/name-format :triple-lowbar}\n",
            FileNameFormat::TripleLowbar,
        ),
    ] {
        let dir = scratch(&format!("unsafe-page-title-{label}"));
        if !config.is_empty() {
            fs::create_dir_all(dir.join("logseq")).unwrap();
            fs::write(dir.join("logseq/config.edn"), config).unwrap();
        }
        let title = "2026-07-23_18:01:20";
        let expected_stem = encode_page_name(title, format);
        assert_windows_safe_page_stem(&expected_stem);
        let expected_rel = format!("pages/{expected_stem}.md");

        let graph = Graph::open(&dir);
        graph.warm_cache();
        let page = markdown_page_dto(title, title, "- created\n").unwrap();
        graph.save_page(&page, None).unwrap();
        assert_eq!(fs::read(dir.join(&expected_rel)).unwrap(), b"- created\n");
        drop(graph);

        let graph = Graph::open(&dir);
        let mut reopened = graph
            .load_named(title, PageKind::Page)
            .unwrap()
            .expect("safe filename page reopens by its exact logical title");
        assert_eq!(reopened.name, title);
        assert_eq!(reopened.path, expected_rel);
        reopened.blocks[0].raw = "edited and durable".into();
        let base = reopened.rev.clone().unwrap();
        graph.save_page(&reopened, Some(&base)).unwrap();
        drop(graph);

        let graph = Graph::open(&dir);
        let final_page = graph
            .load_named(title, PageKind::Page)
            .unwrap()
            .expect("edited safe filename page reopens");
        assert_eq!(final_page.name, title);
        assert_eq!(final_page.path, expected_rel);
        assert_eq!(final_page.blocks[0].raw, "edited and durable");
        let _ = fs::remove_dir_all(dir);
    }
}

#[test]
fn unsafe_page_titles_rename_and_rescue_use_the_same_safe_identity() {
    let dir = scratch("unsafe-page-title-rename-rescue");
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq/config.edn"),
        "{:file/name-format :triple-lowbar}\n",
    )
    .unwrap();
    fs::write(dir.join("pages/Old.md"), "- old body\n").unwrap();
    fs::write(dir.join("journals/Loose.md"), "- rescued body\n").unwrap();
    let graph = Graph::open(&dir);

    let renamed = "2026-07-23_18:01:20";
    graph.rename_page("Old", renamed).unwrap();
    let renamed_stem = encode_page_name(renamed, FileNameFormat::TripleLowbar);
    assert_windows_safe_page_stem(&renamed_stem);
    assert_eq!(
        fs::read(dir.join(format!("pages/{renamed_stem}.md"))).unwrap(),
        b"- old body\n"
    );

    let rescued = "CON";
    graph
        .rename_file_to_page("journals/Loose.md", rescued)
        .unwrap();
    let rescued_stem = encode_page_name(rescued, FileNameFormat::TripleLowbar);
    assert_windows_safe_page_stem(&rescued_stem);
    assert_eq!(
        fs::read(dir.join(format!("pages/{rescued_stem}.md"))).unwrap(),
        b"- rescued body\n"
    );
    drop(graph);

    let reopened = Graph::open(&dir);
    assert_eq!(
        reopened
            .load_named(renamed, PageKind::Page)
            .unwrap()
            .unwrap()
            .path,
        format!("pages/{renamed_stem}.md")
    );
    assert_eq!(
        reopened
            .load_named(rescued, PageKind::Page)
            .unwrap()
            .unwrap()
            .path,
        format!("pages/{rescued_stem}.md")
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn safe_filename_collisions_refuse_without_overwriting_existing_bytes() {
    let dir = scratch("safe-filename-collision");
    let occupied = dir.join("pages/A%3AB.md");
    let original = b"title:: Occupant\n\n- keep these exact bytes\n";
    fs::write(&occupied, original).unwrap();
    let graph = Graph::open(&dir);
    let page = markdown_page_dto("A:B", "A:B", "- must not land\n").unwrap();

    let error = graph.save_page(&page, None).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(&occupied).unwrap(), original);
    assert!(!dir.join("pages/A:B.md").exists());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn generated_legacy_dot_title_round_trips_without_becoming_a_namespace() {
    let dir = scratch("legacy-dot-generated-title");
    let graph = Graph::open(&dir);
    let page = markdown_page_dto("Release 1.0", "Release 1.0", "- body\n").unwrap();

    graph.save_page(&page, None).unwrap();

    let path = dir.join("pages/Release 1%2E0.md");
    let bytes = fs::read_to_string(&path).unwrap();
    assert_eq!(bytes, "- body\n");
    let reopened = graph
        .load_named("Release 1.0", PageKind::Page)
        .unwrap()
        .expect("generated dot title remains addressable by its literal title");
    assert_eq!(reopened.name, "Release 1.0");
    assert_eq!(reopened.path, "pages/Release 1%2E0.md");
    assert!(graph.find_entry("Release 1/0", PageKind::Page).is_none());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn existing_legacy_dot_title_keeps_its_exact_path_and_bytes() {
    let dir = scratch("legacy-dot-existing-title");
    let path = dir.join("pages/Release 1.0.md");
    let original = "title:: Release 1.0\n\n- body\n";
    fs::write(&path, original).unwrap();
    let graph = Graph::open(&dir);
    let page = graph
        .load_named("Release 1.0", PageKind::Page)
        .unwrap()
        .expect("existing legacy dot title remains addressable");
    assert_eq!(page.path, "pages/Release 1.0.md");
    graph.save_page(&page, page.rev.as_deref()).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    assert!(!dir.join("pages/Release 1%2E0.md").exists());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn runtime_ids_are_owner_structural_and_separate_from_explicit_ids() {
    // Equal-text siblings, including duplicate persisted ids, are distinct
    // runtime nodes. Persisted ids remain content used by external resolution.
    let mut roots = vec![
        DocBlock::new("first\nid:: dup-1234"),
        DocBlock::new("first\nid:: dup-1234"),
    ];
    assign_doc_runtime_ids(&mut roots, "pages/client-a/Foo.md");
    assert_ne!(roots[0].uuid, roots[1].uuid);
    assert_ne!(roots[0].uuid, "dup-1234");
    assert_ne!(roots[1].uuid, "dup-1234");

    let first_ids = roots.iter().map(|b| b.uuid.clone()).collect::<Vec<_>>();
    let mut same = vec![
        DocBlock::new("first\nid:: dup-1234"),
        DocBlock::new("first\nid:: dup-1234"),
    ];
    assign_doc_runtime_ids(&mut same, "pages/client-a/Foo.md");
    assert_eq!(
        first_ids,
        same.iter().map(|b| b.uuid.clone()).collect::<Vec<_>>()
    );

    let mut other_owner = vec![DocBlock::new("first\nid:: dup-1234")];
    assign_doc_runtime_ids(&mut other_owner, "pages/client-b/Foo.md");
    assert_ne!(roots[0].uuid, other_owner[0].uuid);

    // A nested duplicate derives from its structural child path.
    let mut parent = DocBlock::new("p\nid:: x");
    parent.children.push(DocBlock::new("c\nid:: x"));
    assign_doc_runtime_ids(std::slice::from_mut(&mut parent), "pages/tree.md");
    assert_ne!(parent.uuid, parent.children[0].uuid);
    assert_ne!(parent.uuid, "x");
    assert_ne!(parent.children[0].uuid, "x");
}

#[test]
fn projected_order_reproduces_reopened_runtime_ids_after_live_structural_edits() {
    for owner in ["pages/Case.md", "pages/é/日本.org", "pages\\é\\日本.org"] {
        let mut roots = vec![DocBlock::new("same\nid:: external"), DocBlock::new("same")];
        roots[0].children.push(DocBlock::new("child"));
        assign_doc_runtime_ids(&mut roots, owner);
        assert_eq!(
            doc_runtime_id_for_order(owner, "00000000")
                .unwrap()
                .to_string(),
            roots[0].uuid
        );
        assert_eq!(
            doc_runtime_id_for_order(owner, "00000000/00000000")
                .unwrap()
                .to_string(),
            roots[0].children[0].uuid
        );
        let old_id = roots[0].uuid.clone();
        roots.insert(0, DocBlock::new("inserted"));
        roots[1].children.push(DocBlock::new("new child"));
        assign_doc_runtime_ids(&mut roots, owner);
        assert_eq!(roots[1].uuid, old_id, "live identity survives insertion");
        let mut reopened = roots.clone();
        for root in &mut reopened {
            root.uuid.clear();
            for child in &mut root.children {
                child.uuid.clear();
            }
        }
        assign_doc_runtime_ids(&mut reopened, owner);
        assert_ne!(reopened[1].uuid, old_id);
        assert_eq!(
            doc_runtime_id_for_order(owner, "00000001")
                .unwrap()
                .to_string(),
            reopened[1].uuid
        );
        assert_eq!(
            doc_runtime_id_for_order(owner, "00000001/00000001")
                .unwrap()
                .to_string(),
            reopened[1].children[1].uuid
        );
        assert_eq!(
            roots[1].children[1].uuid, reopened[1].children[1].uuid,
            "new children use structural namespace even when parent keeps live ID"
        );
    }
    assert_eq!(
        doc_runtime_id_for_order("pages/é/日本.org", "00000000").unwrap(),
        doc_runtime_id_for_order("pages\\é\\日本.org", "00000000").unwrap()
    );
    assert_ne!(
        doc_runtime_id_for_order("pages/Case.md", "00000000").unwrap(),
        doc_runtime_id_for_order("pages/case.md", "00000000").unwrap()
    );
}

#[test]
fn projected_order_rejects_invalid_identity_metadata() {
    for order in [
        "",
        "0",
        "0000000g",
        "0000000A",
        "00000000/",
        "/00000000",
        "000000000",
    ] {
        assert!(
            doc_runtime_id_for_order("pages/a.md", order).is_err(),
            "{order:?}"
        );
    }
    let deepest = vec!["00000000"; MAX_BLOCK_DEPTH].join("/");
    assert!(doc_runtime_id_for_order("pages/a.md", &deepest).is_ok());
    assert!(doc_runtime_id_for_order("pages/a.md", &format!("{deepest}/00000000")).is_err());
    assert!(doc_runtime_id_for_order("../a.md", "00000000").is_err());
    assert!(doc_runtime_id_for_order("", "00000000").is_err());
}

#[test]
fn reserve_asset_avoids_overwrite() {
    let dir = std::env::temp_dir().join(format!("tine-asset-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    // Each reserve CREATES the file (exclusively), so the next reserve of the
    // same name is forced onto a fresh suffix — no manual writes needed, and a
    // racing writer can never be handed an already-taken name.
    assert_eq!(reserve_asset(&dir, "paper.pdf").unwrap().0, "paper.pdf");
    assert_eq!(reserve_asset(&dir, "paper.pdf").unwrap().0, "paper_1.pdf");
    assert_eq!(reserve_asset(&dir, "paper.pdf").unwrap().0, "paper_2.pdf");
    // Extensionless names work too.
    assert_eq!(reserve_asset(&dir, "NOTES").unwrap().0, "NOTES");
    assert_eq!(reserve_asset(&dir, "NOTES").unwrap().0, "NOTES_1");
    // Compound extensions (drawio/excalidraw editable assets) survive de-dup:
    // the counter goes BEFORE the whole `.drawio.svg` suffix so the collided
    // name still matches the editor affordance (GH #38). A naive last-dot
    // split would have produced `flow.drawio_1.svg`.
    assert_eq!(
        reserve_asset(&dir, "flow.drawio.svg").unwrap().0,
        "flow.drawio.svg"
    );
    assert_eq!(
        reserve_asset(&dir, "flow.drawio.svg").unwrap().0,
        "flow_1.drawio.svg"
    );
    assert_eq!(
        reserve_asset(&dir, "flow.drawio.svg").unwrap().0,
        "flow_2.drawio.svg"
    );
    // Case-insensitive suffix match, and .excalidraw.png too.
    assert_eq!(
        reserve_asset(&dir, "S.DRAWIO.SVG").unwrap().0,
        "S.DRAWIO.SVG"
    );
    assert_eq!(
        reserve_asset(&dir, "S.DRAWIO.SVG").unwrap().0,
        "S_1.DRAWIO.SVG"
    );
    assert_eq!(
        reserve_asset(&dir, "art.excalidraw.png").unwrap().0,
        "art.excalidraw.png"
    );
    assert_eq!(
        reserve_asset(&dir, "art.excalidraw.png").unwrap().0,
        "art_1.excalidraw.png"
    );
    // An ordinary double-dotted name (not a known compound) still splits on
    // the last dot — `my.file.txt` → `my.file_1.txt`.
    assert_eq!(reserve_asset(&dir, "my.file.txt").unwrap().0, "my.file.txt");
    assert_eq!(
        reserve_asset(&dir, "my.file.txt").unwrap().0,
        "my.file_1.txt"
    );
    // Every reserved name is a real, distinct file on disk.
    for n in [
        "paper.pdf",
        "paper_1.pdf",
        "paper_2.pdf",
        "NOTES",
        "NOTES_1",
        "flow.drawio.svg",
        "flow_1.drawio.svg",
        "flow_2.drawio.svg",
        "art.excalidraw.png",
        "art_1.excalidraw.png",
        "my.file.txt",
        "my.file_1.txt",
    ] {
        assert!(dir.join(n).exists(), "{n} reserved");
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn reserve_asset_rejects_path_traversal() {
    // F5: a frontend-supplied asset name with a separator or `..`/`.` component
    // must not reach outside assets/. (read_asset shares the same guard.)
    let dir = std::env::temp_dir().join(format!("tine-asset-trav-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    for bad in ["../evil.md", "..", ".", "a/b.png", "a\\b.png", ""] {
        assert!(reserve_asset(&dir, bad).is_err(), "must reject {bad:?}");
    }
    // A plain top-level name still works.
    assert_eq!(reserve_asset(&dir, "ok.png").unwrap().0, "ok.png");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn native_capture_import_streams_with_limit_and_collision_rewind() {
    let dir = scratch("native-capture-import");
    let graph = Graph::open(&dir);
    let source_path = dir.join("tine_memo_source.m4a");
    fs::write(&source_path, b"bounded voice memo").unwrap();
    let mut source = fs::File::open(&source_path).unwrap();

    fs::create_dir_all(dir.join("assets")).unwrap();
    fs::write(dir.join("assets/voice.m4a"), b"existing memo").unwrap();
    let stored = graph
        .import_asset_file(&mut source, "voice.m4a", 32 * 1024 * 1024)
        .unwrap();
    assert_eq!(stored, "voice_1.m4a");
    assert_eq!(
        fs::read(dir.join("assets/voice_1.m4a")).unwrap(),
        b"bounded voice memo"
    );
    assert_eq!(
        fs::read(dir.join("assets/voice.m4a")).unwrap(),
        b"existing memo",
        "collision retry must not overwrite an existing graph asset"
    );

    source.seek(io::SeekFrom::Start(0)).unwrap();
    assert!(graph
        .import_asset_file(&mut source, "too-large.m4a", 4)
        .is_err());
    assert!(
        !dir.join("assets/too-large.m4a").exists(),
        "an over-limit stream must not leave a visible partial asset"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn merge_pages_preserves_src_page_properties() {
    // F2: reconciling a duplicate page must not silently drop src's page
    // properties (alias/tags/icon). dst wins on a key clash (no duplicate line).
    let dir = std::env::temp_dir().join(format!("tine-merge-props-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("journals")).unwrap();
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::write(
        dir.join("pages").join("dst.md"),
        "tags:: Keep\n- dst body\n",
    )
    .unwrap();
    fs::write(
        dir.join("pages").join("src.md"),
        "alias:: Foo\ntags:: Other\n- src body\n",
    )
    .unwrap();
    let g = Graph::open(&dir);
    g.warm_cache();
    g.merge_pages("pages/src.md", "pages/dst.md").unwrap();
    let merged = fs::read_to_string(dir.join("pages").join("dst.md")).unwrap();
    assert!(
        merged.contains("alias:: Foo"),
        "src alias:: preserved: {merged:?}"
    );
    assert!(
        merged.contains("tags:: Keep"),
        "dst tags:: kept: {merged:?}"
    );
    assert!(
        !merged.contains("tags:: Other"),
        "src tags:: must not duplicate dst's key: {merged:?}"
    );
    assert!(
        merged.contains("dst body") && merged.contains("src body"),
        "both bodies merged: {merged:?}"
    );
    assert!(
        !dir.join("pages").join("src.md").exists(),
        "src moved to trash"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn gh62_alias_from_first_bullet_merges_backlinks() {
    // GH #62: a user types `alias:: book` as the FIRST bullet on the "books"
    // page (the natural outliner action). OG treats a properties-only first
    // block as page properties, so `#book` references must resolve to "books"
    // and appear in its backlinks. Before the fix this only worked when the
    // alias lived in the page pre-block (dedicated properties panel / Logseq
    // file convention); the bulleted form silently did nothing.
    let build = |books_body: &str| {
        let dir = std::env::temp_dir().join(format!(
            "tine-gh62-{}-{}",
            books_body.len(),
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::write(dir.join("pages").join("books.md"), books_body).unwrap();
        fs::write(
            dir.join("pages").join("note.md"),
            "- I read a #book today\n",
        )
        .unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();
        let aliases = g.page_aliases();
        let n: usize = g
            .backlinks("books")
            .iter()
            .map(|grp| grp.blocks.len())
            .sum();
        let _ = fs::remove_dir_all(&dir);
        (aliases, n)
    };

    // Alias as the first bullet — now recognized.
    let (a, n) = build("- alias:: book\n- I like reading\n");
    assert_eq!(
        a,
        vec![("book".to_string(), "books".to_string())],
        "first-bullet alias registered"
    );
    assert_eq!(n, 1, "#book backlink merges onto the books page");

    // Pre-block alias keeps working (Logseq file convention / properties panel).
    let (a, n) = build("alias:: book\n\n- I like reading\n");
    assert_eq!(
        a,
        vec![("book".to_string(), "books".to_string())],
        "pre-block alias still registered"
    );
    assert_eq!(n, 1, "pre-block alias backlink still merges");

    // Both Logseq spellings and both common comma glyphs are accepted.
    let (a, n) = build("- aliases:: book，volume\n- I like reading\n");
    assert_eq!(
        a,
        vec![
            ("book".to_string(), "books".to_string()),
            ("volume".to_string(), "books".to_string()),
        ],
        "plural aliases and full-width comma registered"
    );
    assert_eq!(n, 1, "plural alias backlink merges");

    // A whole quoted value is literal text, not a list of page aliases.
    let (a, n) = build("- alias:: \"book\"\n- I like reading\n");
    assert!(a.is_empty(), "quoted alias stays literal: {a:?}");
    assert_eq!(n, 0, "quoted alias does not merge backlinks");

    // A NON-first bullet with `alias::` is a block property, NOT a page alias
    // (OG parity — only the first properties block counts).
    let (a, n) = build("- I like reading\n- alias:: book\n");
    assert!(
        a.is_empty(),
        "alias in a non-first block is not a page alias: {a:?}"
    );
    assert_eq!(n, 0, "no backlink merge for a mid-page block alias");

    // A first block that mixes content with the property is a regular block,
    // not a page-properties block.
    let (a, _) = build("- reading list\nalias:: book\n");
    assert!(
        a.is_empty(),
        "content+property first block is not page properties: {a:?}"
    );
}

#[test]
fn gh62_alias_typed_into_first_block_survives_save_and_reload() {
    let dir = scratch("gh62-save-reload");
    fs::write(
        dir.join("pages").join("books.md"),
        "- placeholder\n- I like reading\n",
    )
    .unwrap();
    fs::write(
        dir.join("pages").join("note.md"),
        "- I read a #book today\n",
    )
    .unwrap();
    let g = Graph::open(&dir);
    g.warm_cache();
    let mut books = g.load_named("books", PageKind::Page).unwrap().unwrap();
    books.blocks[0].raw = "alias:: book".into();
    g.save_page(&books, books.rev.as_deref()).unwrap();

    let disk = fs::read_to_string(dir.join("pages").join("books.md")).unwrap();
    assert_eq!(disk, "alias:: book\n\n- I like reading\n");
    assert_eq!(
        g.load_named("book", PageKind::Page).unwrap().unwrap().name,
        "books"
    );
    assert_eq!(
        g.backlinks("books")
            .iter()
            .map(|group| group.blocks.len())
            .sum::<usize>(),
        1
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn quick_switch_includes_referenced_pages() {
    // A page referenced by `#tag` / `[[link]]` but with no file of its own
    // still "exists" (OG semantics) and must show up in quick-switch — that's
    // what lets `#`/`[[ ]]` autocomplete say "#thistag" rather than a
    // misleading "Create #thistag" when the tag is already used elsewhere.
    let dir = std::env::temp_dir().join(format!("tine-refpages-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("journals")).unwrap();
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::write(
        dir.join("pages").join("notes.md"),
        "- uses #thistag and [[Some Page]]\n",
    )
    .unwrap();
    // A page whose page-properties carry tags::/alias:: (OG autolinks these as
    // page references, bare or bracketed).
    fs::write(
            dir.join("pages").join("paper.md"),
            "tags:: ProjectX， [[Linear IP]]\naliases:: LP Survey，Paper Notes\nstatus:: \"Private, Draft\"\n- body\n",
        )
        .unwrap();
    let g = Graph::open(&dir);
    g.warm_cache(); // referenced names come from the whole-graph cache

    let has = |q: &str, name: &str| {
        g.quick_switch(q, 8)
            .iter()
            .any(|e| crate::refs::same_page(&e.name, name))
    };
    assert!(
        has("thistag", "thistag"),
        "referenced #thistag should appear"
    );
    assert!(
        has("some page", "Some Page"),
        "referenced [[Some Page]] should appear"
    );
    // tags:: values (bare and bracketed) and alias:: values count too.
    assert!(
        has("projectx", "ProjectX"),
        "bare tags:: value should appear"
    );
    assert!(
        has("linear ip", "Linear IP"),
        "bracketed tags:: value should appear"
    );
    for alias in ["LP Survey", "Paper Notes"] {
        let hit = g
            .quick_switch(alias, 8)
            .into_iter()
            .find(|entry| crate::refs::same_page(&entry.name, alias));
        assert_eq!(
            hit.as_ref().map(|entry| entry.rel_path.as_str()),
            Some("pages/paper.md"),
            "an authored alias should be inserted while retaining its owning page identity"
        );
    }
    assert!(
        !has("private", "Private"),
        "quoted custom value stays literal"
    );
    // Neither filed nor referenced → not offered (so autocomplete still says
    // "Create" for a genuinely new name).
    assert!(!has("nonexistent", "nonexistent"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn quick_switch_offers_each_matching_authored_alias_before_and_after_readiness() {
    let dir = std::env::temp_dir().join(format!(
        "tine-gh482-alias-suggestions-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("journals")).unwrap();
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::write(
        dir.join("pages/Welcome to Tine.md"),
        "alias:: Welcome-To-Tine, Tine greet\n\n- home\n",
    )
    .unwrap();
    fs::write(
        dir.join("pages/mentions.md"),
        "- [[Welcome-To-Tine]] and [[Tine greet]]\n",
    )
    .unwrap();

    let graph = Graph::open(&dir);
    graph.warm_cache();
    let assert_suggestions = |phase: &str| {
        let w = graph.quick_switch("W", 20);
        for spelling in ["Welcome to Tine", "Welcome-To-Tine"] {
            assert!(
                w.iter().any(|entry| {
                    entry.name == spelling && entry.rel_path == "pages/Welcome to Tine.md"
                }),
                "{phase}: W must offer {spelling:?} on its real owner: {w:?}"
            );
        }
        let t = graph.quick_switch("T", 20);
        assert!(
            t.iter().any(|entry| {
                entry.name == "Tine greet" && entry.rel_path == "pages/Welcome to Tine.md"
            }),
            "{phase}: T must offer the authored alias: {t:?}"
        );
        assert!(
            w.iter().chain(&t).all(|entry| {
                !matches!(entry.name.as_str(), "Welcome-To-Tine" | "Tine greet")
                    || !entry.rel_path.is_empty()
            }),
            "{phase}: no alias suggestion may be a pathless phantom"
        );
    };

    assert_suggestions("pre-ready fallback");
    graph
        .attach_direct_projection(dir.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_for_direct_query_projection(&graph);
    assert_suggestions("ready projection");

    let _ = fs::remove_dir_all(&dir);
}

/// Open a fixture graph the way the app opens a Direct graph: with its
/// disposable SQLite projection attached and initialized.
///
/// RET2 retired the parsed-graph walk from every public Direct query, so a
/// fixture that only called `Graph::open` would now ask a question the
/// projection cannot answer and get a typed `Unavailable` rather than a walked
/// answer. Fixtures that deliberately test the walk call the oracle free
/// functions in `crate::query` directly instead.
fn ready_graph(dir: &Path) -> Graph {
    let graph = Graph::open(dir);
    // The parsed cache FIRST, then the projection. `warm_cache` prefers the
    // projection when one is attached and then retains no parsed graph at all
    // (R6), and a session with no parsed cache has no per-page derived
    // retention to speak of — so a fixture about the memo scope has to be the
    // ordinary open-parse-project session, not the warm-reopen one.
    graph.warm_cache();
    graph
        .attach_direct_projection(dir.join("private/projection.sqlite"))
        .expect("the disposable projection attaches");
    graph.warm_cache();
    wait_for_direct_query_projection(&graph);
    graph
}

/// Run `attempt` until it answers, retrying ONLY typed readiness.
///
/// `Unavailable` and `Cancelled` fail the fixture immediately: a fixture that
/// slept through them would hide exactly the regression RET2's typed
/// vocabulary exists to expose.
/// Wait until the projection has moved PAST `generation` and is ready there.
///
/// `when_ready` returns the first `Ok`, and readiness is not freshness: right
/// after a save the projection can still be ready AT THE PRE-SAVE GENERATION,
/// so the answer comes back well-formed, prompt, and stale. Waiting for the
/// generation to transition is the only signal that the edit is in the answer.
///
/// Without this, `advanced_query_reexecutes_and_observes_every_edit` failed
/// about one run in four on this machine while passing in CI, and the failure
/// reads as a product defect ("the DONE edit was not observed") rather than as
/// the fixture sampling too early.
fn when_current(graph: &Graph, generation: u64) {
    let started = Instant::now();
    while graph.cache_generation() <= generation || !graph.direct_projection_ready_test() {
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "the projection never advanced past generation {generation} (now {}, ready={})",
            graph.cache_generation(),
            graph.direct_projection_ready_test()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn when_ready<T>(mut attempt: impl FnMut() -> Result<T, crate::query::QueryExecutionError>) -> T {
    let started = Instant::now();
    loop {
        match attempt() {
            Ok(answer) => return answer,
            Err(crate::query::QueryExecutionError::NotReady(reason)) => {
                assert!(
                    started.elapsed() < Duration::from_secs(15),
                    "the query index never became ready ({})",
                    reason.as_str()
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(other) => panic!("the public query route refused: {other}"),
        }
    }
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tine-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("journals")).unwrap();
    fs::create_dir_all(dir.join("pages")).unwrap();
    dir
}

fn arm_present_conflict_for_force(graph: &Graph, page: &PageDto, path: &Path) -> ConflictOverride {
    let bytes = fs::read_to_string(path).unwrap();
    let resource_identity =
        canonical_projection_file_resource_id(&fs::File::open(path).unwrap()).unwrap();
    let observation_epoch = graph.mint_conflict_authority(
        path,
        &ConflictEditorEpisode {
            // The conflict is minted FOR this editor, so it must name it —
            // otherwise the episode equality that increment 3 strengthened
            // would refuse the very editor the banner belongs to.
            activation: page.activation.map(EditorActivation::from_u64),
            loaded_revision: page.rev.clone(),
        },
        ConflictSnapshot::Present {
            revision: content_rev(&bytes),
            resource_identity,
        },
        Some(bytes),
    );
    ConflictOverride { observation_epoch }
}

#[test]
fn external_document_admission_reuses_the_retained_parse() {
    let dir = scratch("external-admission-parse-count");
    let graph = Graph::open(&dir);
    for (relative, content, expected_attempts) in [
        // Markdown's round-trip oracle produces identical canonical
        // source here, so the exact-source cache reuses its retained
        // original parse instead of invoking the outline parser again.
        ("pages/reused.md", "- parent\n  - child\n", 1),
        ("pages/reused.org", "* parent\n** child\n", 1),
    ] {
        let entry = PageEntry {
            name: "reused".into(),
            kind: PageKind::Page,
            date_key: None,
            rel_path: relative.into(),
            path: dir.join(relative),
        };
        crate::outline::reset_parse_attempts();
        let _parsed = parse_external_document(&graph, entry, content).unwrap();
        assert_eq!(
            crate::outline::parse_attempts(),
            expected_attempts,
            "{relative}"
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

fn regular_file_tree(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    fn collect(
        root: &Path,
        current: &Path,
        out: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>,
    ) {
        if !current.exists() {
            return;
        }
        for entry in fs::read_dir(current).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                collect(root, &path, out);
            } else if kind.is_file() {
                out.insert(
                    path.strip_prefix(root).unwrap().to_owned(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }

    let mut out = std::collections::BTreeMap::new();
    collect(root, root, &mut out);
    out
}

fn set_graph_text_content_budget_limit(limit: u64) {
    GRAPH_TEXT_INVENTORY_LIMITS_OVERRIDE.with(|override_limits| {
        *override_limits.borrow_mut() = Some(GraphTextInventoryLimits {
            retained_content_bytes: limit,
            ..GRAPH_TEXT_INVENTORY_LIMITS
        });
    });
}

fn clear_graph_text_content_budget_limit() {
    GRAPH_TEXT_INVENTORY_LIMITS_OVERRIDE.with(|override_limits| {
        *override_limits.borrow_mut() = None;
    });
}

fn last_graph_text_content_budget_peak() -> u64 {
    GRAPH_TEXT_BUDGET_LAST_PEAK.with(Cell::get)
}

#[test]
fn publisher_p1_graph_text_classifier_uses_longest_component_root_and_preserves_exact_path() {
    let dir = scratch("graph-text-classifier-longest-root");
    let mut graph = Graph::open(&dir);
    graph.config.pages_dir = "graph/text".to_owned();
    graph.config.journals_dir = "graph/text/daily".to_owned();

    let nested = GraphTextPath::parse("graph/text/daily/2026/07/naïve.md").unwrap();
    assert_eq!(
        graph.classify_graph_text_path(&nested),
        Ok(GraphTextKind::Journal)
    );
    assert_eq!(nested.as_str(), "graph/text/daily/2026/07/naïve.md");
    assert_eq!(
        graph.classify_graph_text_path(
            &GraphTextPath::parse("graph/text/projects/2026/roadmap.md").unwrap()
        ),
        Ok(GraphTextKind::Page)
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn publisher_p1_graph_text_classifier_rejects_boundary_misses_outside_paths_and_equal_roots() {
    let dir = scratch("graph-text-classifier-rejections");
    let mut graph = Graph::open(&dir);
    graph.config.pages_dir = "pages".to_owned();
    graph.config.journals_dir = "pages-journal".to_owned();
    for path in ["pages-old/file.md", "outside/file.md"] {
        assert!(
            graph
                .classify_graph_text_path(&GraphTextPath::parse(path).unwrap())
                .is_err(),
            "accepted {path}"
        );
    }
    assert_eq!(
        graph.classify_graph_text_path(&GraphTextPath::parse("pages-journal/a.md").unwrap()),
        Ok(GraphTextKind::Journal)
    );

    graph.config.journals_dir = "pages".to_owned();
    assert!(graph
        .classify_graph_text_path(&GraphTextPath::parse("pages/a.md").unwrap())
        .is_err());

    for malformed_pages_root in ["bad*", "COM¹"] {
        graph.config.pages_dir = malformed_pages_root.to_owned();
        graph.config.journals_dir = "journals".to_owned();
        assert!(graph
            .classify_graph_text_path(&GraphTextPath::parse("journals/2026/07/24.md").unwrap())
            .is_err());
    }
    let _ = fs::remove_dir_all(&dir);
}

fn candidate_paths(candidates: &ReferenceCandidatePages) -> Vec<String> {
    let mut paths = candidates
        .pages
        .iter()
        .map(|(entry, _)| entry.rel_path.clone())
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

#[test]
fn search_cache_isolates_one_page_projection_panic() {
    let dir = scratch("search-page-panic-isolation");
    for i in 0..64 {
        fs::write(
            dir.join("pages").join(format!("Page {i:02}.md")),
            format!("- ordinary page {i}\n"),
        )
        .unwrap();
    }

    let g = Graph::open(&dir);
    let entries = g.list_pages();
    let workers = page_cache_worker_count();
    assert!(workers > 1, "test must exercise the parallel cache build");
    assert!(
        entries.len() >= 64,
        "test must cross the parallel threshold"
    );
    let per = (entries.len() + workers - 1) / workers;
    assert!(per >= 2, "a worker shard must contain a sibling page");

    // Pick adjacent entries after observing the actual directory-walk order,
    // guaranteeing both are in the first worker shard on every filesystem.
    let bad = &entries[0];
    let sibling = &entries[1];
    fs::write(&bad.path, format!("- {TEST_PAGE_PARSE_PANIC_SENTINEL}\n")).unwrap();
    let needle = "uniquesameshardsibling";
    fs::write(&sibling.path, format!("- {needle}\n")).unwrap();
    let sibling_path = sibling.rel_path.clone();
    let bad_path = bad.rel_path.clone();
    // Both files were rewritten behind the graph's back, which in the product
    // is a watcher event. `list_pages` above now joins the shared page-build
    // flight (GH #550), so without that event the query below would be served
    // the parse from before these writes and never reach the panic isolation
    // this test is about. The build is still cold afterwards.
    g.invalidate_cache();

    let execution = crate::query_plan::QueryPlan::friendly(needle, 0, 8).execute_with_explain(
        &g,
        || false,
        false,
    );
    assert!(
        execution.hits.iter().any(|hit| matches!(
            hit,
            crate::query_plan::QueryHit::Block { path, .. } if path == &sibling_path
        )),
        "a normal same-shard sibling must remain searchable"
    );
    assert_eq!(g.page_index_failures(), vec![bad_path]);

    // Invalidation clears the old diagnostic, and the paced warm-cache path
    // applies the same page-sized isolation when it rebuilds.
    g.invalidate_cache();
    assert!(g.page_index_failures().is_empty());
    g.warm_cache();
    assert!(crate::query_plan::QueryPlan::friendly(needle, 0, 8)
        .execute_with_explain(&g, || false, false)
        .hits
        .iter()
        .any(|hit| matches!(
            hit,
            crate::query_plan::QueryHit::Block { path, .. } if path == &sibling_path
        )));
    assert_eq!(g.page_index_failures(), vec![bad.rel_path.clone()]);
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(any(unix, windows))]
fn assert_editor_save_identity_race(force: bool) {
    let mode = if force { "force" } else { "normal" };
    let dir = scratch(&format!("graph-text-{mode}-identity-race"));
    fs::create_dir_all(dir.join("external")).unwrap();
    let path = dir.join("external/Exact.md");
    fs::write(&path, "- loaded baseline\n").unwrap();
    let graph = Graph::open(&dir);
    let mut page = graph.load_by_path("external/Exact.md").unwrap().unwrap();
    as_editor(&graph, &mut page);
    page.blocks[0].raw = format!("{mode} editor bytes");

    let replacement = dir.join("external/.foreign-replacement");
    let foreign_bytes = format!("- foreign {mode} winner\n").into_bytes();
    fs::write(&replacement, &foreign_bytes).unwrap();
    let foreign_identity =
        canonical_projection_file_resource_id(&fs::File::open(&replacement).unwrap()).unwrap();
    let shown = if force {
        fs::write(&path, "- shown force conflict\n").unwrap();
        let conflict = graph.save_page(&page, page.rev.as_deref()).unwrap_err();
        assert_eq!(
            direct_save_failure_code(&conflict),
            "conflict.save_baseline_present"
        );
        Some(gh254_shown(&conflict))
    } else {
        None
    };
    GRAPH_TEXT_WRITE_BEFORE_MUTATION.with(|hook| {
        let path = path.clone();
        let replacement = replacement.clone();
        *hook.borrow_mut() = Some(Box::new(move || {
            #[cfg(unix)]
            fs::rename(&replacement, &path)?;
            #[cfg(windows)]
            {
                fs::remove_file(&path)?;
                fs::rename(&replacement, &path)?;
            }
            Ok(())
        }));
    });

    let error = if force {
        graph.force_save_page_at_revision(
            &page,
            page.rev.as_deref(),
            shown.expect("the forced arm captured its shown observation"),
        )
    } else {
        graph.save_page(&page, page.rev.as_deref())
    }
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{error}");
    assert_eq!(fs::read(&path).unwrap(), foreign_bytes);
    assert_eq!(
        canonical_projection_file_resource_id(&fs::File::open(&path).unwrap()).unwrap(),
        foreign_identity,
        "{mode} save must restore the exact foreign file identity"
    );
    assert!(
        fs::read_dir(dir.join("external"))
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("editor-recovery")),
        "{mode} save restored the foreign target but leaked a recovery name"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(any(unix, windows))]
#[test]
fn normal_save_identity_race_restores_foreign_target_without_overwrite() {
    assert_editor_save_identity_race(false);
}

#[cfg(any(unix, windows))]
#[test]
fn force_save_identity_race_restores_foreign_target_without_overwrite() {
    assert_editor_save_identity_race(true);
}

#[cfg(any(unix, windows))]
fn assert_post_retirement_foreign_destination(restoration_branch: bool) {
    let branch = if restoration_branch {
        "restoration"
    } else {
        "publication"
    };
    let dir = scratch(&format!("graph-text-post-retire-{branch}-race"));
    let parent = dir.join("external");
    fs::create_dir_all(&parent).unwrap();
    let path = parent.join("Exact.md");
    let original_bytes = b"- loaded baseline\n".to_vec();
    fs::write(&path, &original_bytes).unwrap();
    let original_identity =
        canonical_projection_file_resource_id(&fs::File::open(&path).unwrap()).unwrap();

    let graph = Graph::open(&dir);
    graph.warm_cache();
    let mut page = graph.load_by_path("external/Exact.md").unwrap().unwrap();
    let baseline = graph
        .loaded_file_identities
        .read()
        .unwrap()
        .get(&path)
        .cloned()
        .unwrap();
    let mut cached_revisions = graph.disk_revs.read().unwrap().clone();
    let cache_generation = graph.cache_gen.load(std::sync::atomic::Ordering::Acquire);
    page.blocks[0].raw = format!("user staged {branch} bytes");
    let staged_bytes = format!("- user staged {branch} bytes\n").into_bytes();

    let replacement = parent.join(".foreign-replacement");
    let foreign_bytes = format!("- foreign {branch} winner\n").into_bytes();
    fs::write(&replacement, &foreign_bytes).unwrap();
    let foreign_identity =
        canonical_projection_file_resource_id(&fs::File::open(&replacement).unwrap()).unwrap();

    if restoration_branch {
        GRAPH_TEXT_WRITE_AFTER_RETIRE.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(|| {
                Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "injected post-retirement validation failure",
                ))
            }));
        });
        GRAPH_TEXT_WRITE_BEFORE_RESTORE.with(|hook| {
            let path = path.clone();
            let replacement = replacement.clone();
            *hook.borrow_mut() = Some(Box::new(move || fs::rename(replacement, path)));
        });
    } else {
        GRAPH_TEXT_WRITE_AFTER_RETIRE.with(|hook| {
            let path = path.clone();
            let replacement = replacement.clone();
            *hook.borrow_mut() = Some(Box::new(move || fs::rename(replacement, path)));
        });
    }

    let error = graph.save_page(&page, page.rev.as_deref()).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{error}");
    if restoration_branch {
        assert!(
            error.to_string().contains("displaced target retained as")
                && error
                    .to_string()
                    .contains("staged editor bytes retained as"),
            "{error}"
        );
    } else {
        assert_eq!(
            direct_save_failure_code(&error),
            "conflict.replace_publication_collision"
        );
    }
    assert_eq!(fs::read(&path).unwrap(), foreign_bytes);
    assert_eq!(
        canonical_projection_file_resource_id(&fs::File::open(&path).unwrap()).unwrap(),
        foreign_identity,
        "{branch} collision must preserve the exact foreign destination identity"
    );

    let mut retired = None;
    let mut staged = None;
    for entry in fs::read_dir(&parent).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.contains("editor-recovery") {
            retired = Some(entry.path());
        } else if name.contains("editor-staged-recovery") {
            staged = Some(entry.path());
        }
    }
    let retired = retired.expect("retired original must remain in its hidden recovery name");
    assert_eq!(fs::read(&retired).unwrap(), original_bytes);
    assert_eq!(
        canonical_projection_file_resource_id(&fs::File::open(&retired).unwrap()).unwrap(),
        original_identity,
        "{branch} collision must retain the exact retired original identity"
    );
    let staged = staged.expect("staged editor bytes must remain in their hidden recovery name");
    assert_eq!(fs::read(staged).unwrap(), staged_bytes);

    assert_eq!(
        graph
            .loaded_file_identities
            .read()
            .unwrap()
            .get(&path)
            .cloned(),
        Some(baseline),
        "{branch} failure must not advance the loaded identity baseline"
    );
    cached_revisions.insert(
        path.clone(),
        content_rev(std::str::from_utf8(&foreign_bytes).unwrap()),
    );
    assert_eq!(
        *graph.disk_revs.read().unwrap(),
        cached_revisions,
        "{branch} failure must publish the surviving owner's disk revision"
    );
    assert!(
        graph.cache_gen.load(std::sync::atomic::Ordering::Acquire) > cache_generation,
        "{branch} failure must publish the surviving owner"
    );
    graph.with_pages(|pages| {
        let (_, document) = pages
            .iter()
            .find(|(entry, _)| entry.rel_path == "external/Exact.md")
            .unwrap();
        assert_eq!(document.roots[0].raw, format!("foreign {branch} winner"));
    });
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(any(unix, windows))]
#[test]
fn foreign_destination_after_retirement_blocks_staged_publication_without_overwrite() {
    assert_post_retirement_foreign_destination(false);
}

#[cfg(any(unix, windows))]
#[test]
fn foreign_destination_before_restore_keeps_retired_original_recoverable() {
    assert_post_retirement_foreign_destination(true);
}

// Restored after `a5cf7c11 refactor: remove legacy managed sync model path`
// deleted it wholesale. Its second half called `migrate_sync_identities`,
// which that refactor legitimately removed — but the link-count parity above
// it is not legacy and is still load-bearing. Since GH #571 the raw count no
// longer refuses an ordinary save; it remains the interim refusal on the two
// paths that have no complete identity index (the rename transaction's move
// source and editor recovery), and on Windows that count comes from
// `GetFileInformationByHandle` on a handle opened BEFORE the hard link
// existed. If a held handle reported a stale 1, those refusals would be
// defeated on Windows only. The v1 half is dropped; the platform assertion
// is not.
#[cfg(windows)]
#[test]
fn projection_windows_held_handle_link_count_tracks_one_and_two_links() {
    let dir = scratch("projection-windows-held-handle-link-count");
    let target = dir.join("pages/Target.md");
    let alias = dir.join("pages/Alias.md");
    fs::write(&target, b"- retained\n").unwrap();
    let file = fs::File::open(&target).unwrap();

    assert_eq!(projection_file_link_count(&file).unwrap(), 1);
    fs::hard_link(&target, &alias).unwrap();
    assert_eq!(
        projection_file_link_count(&file).unwrap(),
        2,
        "a held handle must observe the new link, or the rename and recovery \
             move paths cannot refuse a hard-linked graph text file on Windows"
    );
    assert_eq!(fs::read(&target).unwrap(), b"- retained\n");
    assert_eq!(fs::read(&alias).unwrap(), b"- retained\n");

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(windows)]
#[test]
fn windows_handle_relative_noreplace_renames_the_exact_source() {
    let path = scratch("windows-handle-relative-noreplace-success");
    let source = path.join("pages/source");
    let destination = path.join("pages/destination");
    fs::write(&source, b"exact source bytes").unwrap();
    let source_identity =
        canonical_projection_file_resource_id(&fs::File::open(&source).unwrap()).unwrap();
    let dir = Dir::open_ambient_dir(path.join("pages"), ambient_authority()).unwrap();

    rename_projection_noreplace(&dir, "source", "destination").unwrap();

    assert!(!source.exists());
    assert_eq!(fs::read(&destination).unwrap(), b"exact source bytes");
    assert_eq!(
        canonical_projection_file_resource_id(&fs::File::open(&destination).unwrap()).unwrap(),
        source_identity
    );
    assert_eq!(
        regular_file_tree(&path),
        std::collections::BTreeMap::from([(
            PathBuf::from("pages/destination"),
            b"exact source bytes".to_vec(),
        )])
    );
    let _ = fs::remove_dir_all(path);
}

#[cfg(windows)]
#[test]
fn windows_handle_relative_noreplace_moves_between_nonstandard_retained_directories_with_unicode() {
    let path = scratch("windows-cross-directory-unicode-noreplace");
    let source_path = path.join("source tree").join("nested.dir");
    let destination_path = path.join("destination-tree").join("nested space");
    fs::create_dir_all(&source_path).unwrap();
    fs::create_dir_all(&destination_path).unwrap();
    let source = source_path.join("exact-source");
    let destination_name = "résumé-東京.md";
    let destination = destination_path.join(destination_name);
    let bytes = b"cross-directory exact source bytes";
    fs::write(&source, bytes).unwrap();
    let source_identity =
        canonical_projection_file_resource_id(&fs::File::open(&source).unwrap()).unwrap();
    let source_dir = Dir::open_ambient_dir(&source_path, ambient_authority()).unwrap();
    let destination_dir = Dir::open_ambient_dir(&destination_path, ambient_authority()).unwrap();

    rename_projection_between_noreplace(
        &source_dir,
        "exact-source",
        &destination_dir,
        destination_name,
    )
    .unwrap();

    assert!(!source.exists());
    assert_eq!(fs::read(&destination).unwrap(), bytes);
    assert_eq!(
        canonical_projection_file_resource_id(&fs::File::open(&destination).unwrap()).unwrap(),
        source_identity
    );
    assert_eq!(
        regular_file_tree(&path),
        std::collections::BTreeMap::from([(
            PathBuf::from("destination-tree")
                .join("nested space")
                .join(destination_name),
            bytes.to_vec(),
        )])
    );
    let _ = fs::remove_dir_all(path);
}

#[cfg(windows)]
#[test]
fn windows_handle_relative_noreplace_preserves_occupied_destination() {
    let path = scratch("windows-handle-relative-noreplace");
    fs::write(path.join("source"), b"source").unwrap();
    fs::write(path.join("destination"), b"destination").unwrap();
    let source_identity =
        canonical_projection_file_resource_id(&fs::File::open(path.join("source")).unwrap())
            .unwrap();
    let destination_identity =
        canonical_projection_file_resource_id(&fs::File::open(path.join("destination")).unwrap())
            .unwrap();
    let dir = Dir::open_ambient_dir(&path, ambient_authority()).unwrap();

    let error = rename_projection_noreplace(&dir, "source", "destination").unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{error}");
    assert_eq!(fs::read(path.join("source")).unwrap(), b"source");
    assert_eq!(fs::read(path.join("destination")).unwrap(), b"destination");
    assert_eq!(
        canonical_projection_file_resource_id(&fs::File::open(path.join("source")).unwrap())
            .unwrap(),
        source_identity
    );
    assert_eq!(
        canonical_projection_file_resource_id(&fs::File::open(path.join("destination")).unwrap())
            .unwrap(),
        destination_identity
    );
    let _ = fs::remove_dir_all(path);
}

#[test]
fn graph_wide_exact_load_parser_failure_never_returns_a_writable_dto() {
    let dir = scratch("graph-text-exact-parser-failure");
    fs::create_dir_all(dir.join("external")).unwrap();
    let path = dir.join("external/Parser.md");
    let bytes = format!("- {TEST_PAGE_PARSE_PANIC_SENTINEL}\n");
    fs::write(&path, &bytes).unwrap();
    let graph = Graph::open(&dir);

    assert_eq!(
        graph.load_by_path("external/Parser.md").unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), bytes);
    assert!(graph
        .loaded_file_identities
        .read()
        .unwrap()
        .get(&path)
        .is_none());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn graph_wide_discovery_preserves_direct_files_without_id_stamping() {
    let dir = scratch("graph-text-direct-files");
    fs::create_dir_all(dir.join("external")).unwrap();
    let path = dir.join("external/Outside.md");
    fs::write(&path, "- outside\n").unwrap();
    let graph = Graph::open(&dir);

    let mut page = graph.load_by_path("external/Outside.md").unwrap().unwrap();
    page.blocks[0].raw = "edited outside".into();
    graph.save_page(&page, page.rev.as_deref()).unwrap();
    let saved = fs::read_to_string(&path).unwrap();
    assert_eq!(saved, "- edited outside\n");
    assert!(!saved.contains("id::"));
    fs::write(&path, "- watcher outside\n").unwrap();
    graph.sync_file_checked(&path).unwrap();
    fs::remove_file(&path).unwrap();
    graph.sync_deleted_file(&path).unwrap();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn graph_wide_markdown_discovery_preserves_direct_files_without_id_stamping() {
    let dir = scratch("graph-text-markdown-direct-files");
    fs::create_dir_all(dir.join("external")).unwrap();
    let path = dir.join("external/Outside.markdown");
    fs::write(&path, "- outside\n").unwrap();
    let graph = Graph::open(&dir);

    let mut page = graph
        .load_by_path("external/Outside.markdown")
        .unwrap()
        .unwrap();
    assert_eq!(page.path, "external/Outside.markdown");
    page.blocks[0].raw = "edited outside".into();
    graph.save_page(&page, page.rev.as_deref()).unwrap();
    let saved = fs::read_to_string(&path).unwrap();
    assert_eq!(saved, "- edited outside\n");
    assert!(!saved.contains("id::"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn graph_wide_inventory_is_bounded_and_visits_entries_linearly() {
    let dir = scratch("graph-text-linear-inventory");
    for index in 0..64 {
        let directory = dir.join("external").join(format!("d{index:02}"));
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join(format!("P{index:02}.md")), "- page\n").unwrap();
    }
    let graph = Graph::open(&dir);
    let permit = graph.admit_retained_graph_text_writer().unwrap();
    GRAPH_TEXT_INVENTORY_ENTRY_VISITS.with(|visits| visits.set(0));
    let entries = graph.graph_text_entries(&permit).unwrap();
    let visits = GRAPH_TEXT_INVENTORY_ENTRY_VISITS.with(Cell::get);
    assert_eq!(entries.len(), 64);
    assert!(
        visits <= 2 * entries.len() + 4,
        "one retained walk must stay linear: visits={visits}, entries={}",
        entries.len()
    );

    for limits in [
        GraphTextInventoryLimits {
            graph_text_files: 1,
            ..GRAPH_TEXT_INVENTORY_LIMITS
        },
        GraphTextInventoryLimits {
            directory_depth: 1,
            ..GRAPH_TEXT_INVENTORY_LIMITS
        },
        GraphTextInventoryLimits {
            all_entries: 1,
            ..GRAPH_TEXT_INVENTORY_LIMITS
        },
        GraphTextInventoryLimits {
            directories: 1,
            ..GRAPH_TEXT_INVENTORY_LIMITS
        },
        GraphTextInventoryLimits {
            pending_directories: 1,
            ..GRAPH_TEXT_INVENTORY_LIMITS
        },
        GraphTextInventoryLimits {
            path_bytes: 1,
            ..GRAPH_TEXT_INVENTORY_LIMITS
        },
    ] {
        assert!(graph
            .text_entries_with_limits_and_budget(&permit, false, limits, None, vec![("", 0)], true,)
            .is_err());
    }
    drop(permit);
    GRAPH_TEXT_INVENTORY_LIMITS_OVERRIDE.with(|override_limits| {
        *override_limits.borrow_mut() = Some(GraphTextInventoryLimits {
            retained_content_bytes: 1,
            ..GRAPH_TEXT_INVENTORY_LIMITS
        });
    });
    assert_eq!(
        graph
            .load_by_path("external/d00/P00.md")
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    GRAPH_TEXT_INVENTORY_LIMITS_OVERRIDE.with(|override_limits| {
        *override_limits.borrow_mut() = None;
    });
    let _ = fs::remove_dir_all(&dir);
}

fn reset_cache_linear_scan_steps() {
    CACHE_LINEAR_SCAN_STEPS.with(|steps| steps.set(0));
}

fn cache_linear_scan_steps() -> usize {
    CACHE_LINEAR_SCAN_STEPS.with(|steps| steps.get())
}

#[test]
fn find_entry_cache_avoids_per_lookup_graph_inventory_fanout() {
    let dir = scratch("find-entry-cache-fanout");
    for i in 0..16 {
        fs::write(dir.join("pages").join(format!("Page {i}.md")), "- body\n").unwrap();
    }
    let g = Graph::open(&dir);
    GRAPH_TEXT_INVENTORY_ENTRY_VISITS.with(|visits| visits.set(0));
    g.warm_cache();
    let warm_visits = GRAPH_TEXT_INVENTORY_ENTRY_VISITS.with(Cell::get);
    assert!(warm_visits >= 16);

    for i in 0..16 {
        let entry = g
            .find_entry(&format!("Page {i}"), PageKind::Page)
            .expect("page exists");
        assert_eq!(entry.name, format!("Page {i}"));
    }
    assert_eq!(
        GRAPH_TEXT_INVENTORY_ENTRY_VISITS.with(Cell::get),
        warm_visits,
        "all page lookups in one generation should share the warm graph inventory"
    );

    for i in 0..16 {
        assert!(g.find_entry(&format!("Page {i}"), PageKind::Page).is_some());
    }
    assert_eq!(
        GRAPH_TEXT_INVENTORY_ENTRY_VISITS.with(Cell::get),
        warm_visits,
        "warm find_entry index should serve repeated lookups without inventory rescans"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn find_entry_cache_uses_semantic_identity_and_prefers_canonical_journal_days() {
    let dir = scratch("find-entry-cache-equivalence");
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq").join("config.edn"),
        "{:preferred-format \"Org\"\n :journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
    )
    .unwrap();
    fs::write(dir.join("pages").join("Foo.md"), "- normal\n").unwrap();
    fs::create_dir_all(dir.join("pages").join("sub")).unwrap();
    fs::write(
        dir.join("pages").join("sub").join("Nested.md"),
        "- nested\n",
    )
    .unwrap();
    fs::write(dir.join("journals").join("2026_06_26.org"), "* canonical\n").unwrap();
    fs::write(
        dir.join("journals").join("Friday, 26-06-2026.org"),
        "* stray\n",
    )
    .unwrap();
    let g = Graph::open(&dir);

    assert_eq!(
        g.find_entry("foo", PageKind::Page).unwrap().rel_path,
        "pages/Foo.md"
    );
    assert_eq!(
        g.find_entry("Nested", PageKind::Page).unwrap().rel_path,
        "pages/sub/Nested.md"
    );
    assert_eq!(
        g.find_entry("Friday, 26-06-2026", PageKind::Journal)
            .expect("duplicate journal day keeps its canonical logical winner")
            .rel_path,
        "journals/2026_06_26.org"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn parsed_doc_cache_index_avoids_warm_open_linear_scans() {
    let dir = scratch("doc-cache-index-fanout");
    for i in 0..24 {
        fs::write(dir.join("pages").join(format!("Page {i}.md")), "- body\n").unwrap();
    }
    let g = Graph::open(&dir);
    g.warm_cache();
    assert!(
        g.cache_index.read().unwrap().is_some(),
        "warm cache should install the by-name parsed-doc index"
    );

    reset_cache_linear_scan_steps();
    for i in 0..24 {
        let page = g
            .load_named(&format!("Page {i}"), PageKind::Page)
            .unwrap()
            .expect("page exists");
        assert_eq!(page.name, format!("Page {i}"));
    }
    assert_eq!(
        cache_linear_scan_steps(),
        0,
        "warm page opens must not fall back to Vec scans"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn parsed_doc_cache_index_does_not_serve_deleted_page() {
    let dir = scratch("doc-cache-index-delete");
    fs::write(dir.join("pages").join("Gone.md"), "- old\n").unwrap();
    let g = Graph::open(&dir);
    g.warm_cache();
    let entry = g.find_entry("Gone", PageKind::Page).unwrap();
    assert!(g.load_page(&entry).is_ok());

    g.delete_page("Gone", PageKind::Page).unwrap();
    assert!(
        g.load_page(&entry).is_err(),
        "stale cache/index must not serve the deleted entry"
    );
    assert!(g.load_named("Gone", PageKind::Page).unwrap().is_none());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn parsed_doc_cache_index_rebuilds_after_rename() {
    let dir = scratch("doc-cache-index-rename");
    fs::write(
        dir.join("pages").join("Old.md"),
        "- links [[Old]] and #Old\n",
    )
    .unwrap();
    let g = Graph::open(&dir);
    g.warm_cache();
    let old_entry = g.find_entry("Old", PageKind::Page).unwrap();
    assert!(g.load_page(&old_entry).is_ok());

    g.rename_page("Old", "New").unwrap();
    assert!(
        g.load_page(&old_entry).is_err(),
        "old entry must not be served after rename"
    );
    assert!(g.load_named("Old", PageKind::Page).unwrap().is_none());
    let new_page = g
        .load_named("New", PageKind::Page)
        .unwrap()
        .expect("new name resolves");
    assert_eq!(new_page.name, "New");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn find_entry_cache_rebuilds_after_file_rescue_generation_bump() {
    let dir = scratch("find-entry-cache-rescue");
    fs::write(dir.join("journals").join("Loose.md"), "- loose\n").unwrap();
    let g = Graph::open(&dir);

    assert!(g.find_entry("Rescued", PageKind::Page).is_none());
    assert!(g.find_entry("Loose", PageKind::Page).is_some());

    g.rename_file_to_page("journals/Loose.md", "Rescued")
        .unwrap();
    assert!(g.find_entry("Loose", PageKind::Page).is_none());
    assert!(g.find_entry("Rescued", PageKind::Page).is_some());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn find_entry_cache_invalidated_by_cold_sync_file() {
    // Regression: the gen-keyed find_entry index must not go stale on
    // sync_file with the parsed-doc cache absent. The ordinary page upsert now
    // advances the generation and enqueues the projection delta in this case;
    // find_entry must not keep serving the pre-create index.
    let dir = scratch("find-entry-cache-cold-sync");
    fs::write(dir.join("pages").join("Existing.md"), "- body\n").unwrap();
    let g = Graph::open(&dir);

    // Do NOT warm the doc cache: find_entry builds only its own index, so
    // self.cache stays cold throughout reconciliation.
    assert!(g.find_entry("New", PageKind::Page).is_none());

    // A brand-new external file appears (as Logseq/Syncthing would create it),
    // reconciled while the doc cache is still cold.
    fs::write(dir.join("pages").join("New.md"), "- new body\n").unwrap();
    g.sync_file(&dir.join("pages").join("New.md"));

    assert!(
        g.find_entry("New", PageKind::Page).is_some(),
        "find_entry index must reflect a file added via the cold sync_file branch"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn with_pages_snapshot_does_not_block_cache_upsert() {
    let dir = scratch("with-pages-snapshot-nonblocking");
    fs::write(dir.join("pages").join("A.md"), "- old\n").unwrap();
    let g = Arc::new(Graph::open(&dir));
    g.warm_cache();

    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let scan_graph = Arc::clone(&g);
    let scan = std::thread::spawn(move || {
        scan_graph.with_pages(|pages| {
            assert!(!pages.is_empty());
            entered_tx.send(()).unwrap();
            release_rx
                .recv_timeout(std::time::Duration::from_secs(2))
                .expect("test should release the blocked snapshot scan");
        });
    });

    entered_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("snapshot scan should enter its closure");

    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let write_graph = Arc::clone(&g);
    let path = dir.join("pages").join("B.md");
    fs::write(&path, "- new\n").unwrap();
    let writer = std::thread::spawn(move || {
        let content = "- new\n";
        let entry = PageEntry {
            name: "B".to_string(),
            kind: PageKind::Page,
            date_key: None,
            rel_path: "pages/B.md".to_string(),
            path: path.clone(),
        };
        write_graph.cache_upsert(entry, parse_doc(&path, content), content_rev(content));
        done_tx.send(()).unwrap();
    });

    let writer_finished_while_scan_blocked = done_rx
        .recv_timeout(std::time::Duration::from_millis(300))
        .is_ok();
    release_tx.send(()).unwrap();
    scan.join().unwrap();
    writer.join().unwrap();

    assert!(
        writer_finished_while_scan_blocked,
        "cache_upsert must not wait for a with_pages closure to finish"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn with_pages_snapshot_survives_concurrent_upsert() {
    let dir = scratch("with-pages-snapshot-consistent");
    let path = dir.join("pages").join("A.md");
    fs::write(&path, "- old body\n").unwrap();
    let g = Arc::new(Graph::open(&dir));
    g.warm_cache();

    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let (observed_tx, observed_rx) = std::sync::mpsc::channel();
    let scan_graph = Arc::clone(&g);
    let scan = std::thread::spawn(move || {
        scan_graph.with_pages(|pages| {
            let (_, doc) = pages
                .iter()
                .find(|(entry, _)| entry.kind == PageKind::Page && entry.name == "A")
                .expect("cached page exists");
            let before = doc.roots[0].raw.clone();
            entered_tx.send(()).unwrap();
            release_rx
                .recv_timeout(std::time::Duration::from_secs(2))
                .expect("test should release the blocked snapshot scan");
            let after = doc.roots[0].raw.clone();
            observed_tx.send((before, after)).unwrap();
        });
    });

    entered_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("snapshot scan should enter its closure");

    let new_content = "- new body\n";
    fs::write(&path, new_content).unwrap();
    let entry = PageEntry {
        name: "A".to_string(),
        kind: PageKind::Page,
        date_key: None,
        rel_path: "pages/A.md".to_string(),
        path: path.clone(),
    };
    g.cache_upsert(
        entry,
        parse_doc(&path, new_content),
        content_rev(new_content),
    );

    release_tx.send(()).unwrap();
    let (before, after) = observed_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("snapshot scan should report observed values");
    scan.join().unwrap();

    assert_eq!(before, "old body");
    assert_eq!(
        after, "old body",
        "a with_pages scan must keep iterating its original snapshot"
    );
    let loaded = g
        .load_named("A", PageKind::Page)
        .unwrap()
        .expect("page remains loadable");
    assert_eq!(loaded.blocks[0].raw, "new body");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn migrate_recovers_title_named_org_journals() {
    // Regression: changing :journal/page-title-format while a stale in-memory
    // format was still active saved new journals under their title
    // ("Thursday, 25-06-2026.org") instead of the date stem, so they dropped
    // out of the feed. A reopen + migrate (now .org-aware) must recover them.
    let dir = scratch("journal-migrate-org");
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq").join("config.edn"),
        "{:preferred-format \"Org\"\n :journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
    )
    .unwrap();
    fs::write(
        dir.join("journals").join("Thursday, 25-06-2026.org"),
        "* bla\n",
    )
    .unwrap();
    // A canonical file for another day must be left untouched.
    fs::write(dir.join("journals").join("2026_06_24.org"), "* prior\n").unwrap();

    let g = Graph::open(&dir);
    assert_eq!(
        g.migrate_journal_filenames(),
        1,
        "exactly the title-named file renamed"
    );
    assert!(
        dir.join("journals").join("2026_06_25.org").exists(),
        "renamed to date stem"
    );
    assert!(
        !dir.join("journals")
            .join("Thursday, 25-06-2026.org")
            .exists(),
        "old name gone"
    );
    assert!(
        dir.join("journals").join("2026_06_24.org").exists(),
        "canonical file untouched"
    );

    // It's now recognized in the feed listing (name via the title format).
    let names: Vec<String> = Graph::open(&dir)
        .journals_desc()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert!(
        names.iter().any(|n| n == "Thursday, 25-06-2026"),
        "listed: {names:?}"
    );
    let _ = fs::remove_dir_all(&dir);
}

fn duplicate_day_graph(name: &str) -> PathBuf {
    let dir = scratch(name);
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq").join("config.edn"),
        "{:journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
    )
    .unwrap();
    fs::write(
        dir.join("journals").join("2026_06_26.md"),
        "- shared line\n- only in canonical\n",
    )
    .unwrap();
    fs::write(
        dir.join("journals").join("Friday, 26-06-2026.md"),
        "- shared line\n- only in stray\n",
    )
    .unwrap();
    dir
}

#[test]
fn conflict_queue_offers_duplicate_journal_days_as_resolvable_objects() {
    // The whole point of item 5: a duplicate day used to reach the user only
    // through a startup toast, because it was not a queue object at all. As
    // an object it inherits the badge, the count, the dock and the walk.
    let dir = duplicate_day_graph("queue-duplicate-journal");
    let graph = Graph::open(&dir);
    graph.warm_cache();

    let queue = graph.conflict_queue();
    let day = queue
        .iter()
        .find(|object| object.source == crate::concord_queue::ConflictSource::DuplicateJournal)
        .expect("the duplicate day is a queue object");

    assert_eq!(day.page_name, "Friday, 26-06-2026");
    assert_eq!(day.page_path, "journals/2026_06_26.md");
    assert_eq!(day.kind, PageKind::Journal);
    assert_eq!(day.sides.len(), 2, "canonical + one stray");
    assert_eq!(day.sides[0].label, "2026_06_26.md");
    assert_eq!(day.sides[1].label, "Friday, 26-06-2026.md");
    assert!(day.markers.is_empty());
    // Merge is implicit: real rows, so the panel can offer keep-mine /
    // keep-theirs / keep-both rather than a bare file list.
    assert!(
        day.block_conflicts.is_some_and(|rows| rows > 0),
        "expected decidable rows, got {:?}",
        day.block_conflicts
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn duplicate_journal_ids_are_stable_across_a_restart() {
    let dir = duplicate_day_graph("queue-duplicate-journal-stable");
    let first = Graph::open(&dir).conflict_queue();
    let second = Graph::open(&dir).conflict_queue();
    let ids = |queue: &[crate::concord_queue::ConflictObject]| {
        queue.iter().map(|o| o.id.clone()).collect::<Vec<_>>()
    };
    assert_eq!(ids(&first), ids(&second));
    assert!(first
        .iter()
        .any(|o| o.id == "journal:journals/2026_06_26.md"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn resolving_a_duplicate_day_folds_the_stray_in_and_leaves_the_queue() {
    let dir = duplicate_day_graph("queue-duplicate-journal-resolve");
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let diff = graph
        .duplicate_journal_diff("journals/2026_06_26.md", "journals/Friday, 26-06-2026.md")
        .unwrap()
        .expect("a same-format pair diffs");
    // Keep both sides of every decidable row - the case that must reproduce
    // what Settings' Merge does by concatenation.
    fn keep_both(
        rows: &[crate::sync_diff::DiffRow],
        out: &mut std::collections::HashMap<String, String>,
    ) {
        for row in rows {
            if row.kind != crate::sync_diff::RowKind::Unchanged {
                out.insert(row.id.clone(), "both".to_string());
            }
            keep_both(&row.children, out);
        }
    }
    let mut decisions = std::collections::HashMap::new();
    keep_both(&diff.rows, &mut decisions);

    graph
        .resolve_duplicate_journal_day(
            "journals/2026_06_26.md",
            "journals/Friday, 26-06-2026.md",
            &decisions,
            &diff.base_rev,
            &diff.conflict_rev,
            "union",
        )
        .unwrap();

    let kept = fs::read_to_string(dir.join("journals").join("2026_06_26.md")).unwrap();
    assert!(
        kept.contains("only in canonical"),
        "canonical kept: {kept:?}"
    );
    assert!(kept.contains("only in stray"), "stray folded in: {kept:?}");
    assert!(
        !dir.join("journals").join("Friday, 26-06-2026.md").exists(),
        "the stray is gone from the graph"
    );
    // Recoverable, never deleted (ADR 0007).
    let trash = typed_trash_dir(&dir, TrashEntryKind::Conflict);
    assert!(
        fs::read_dir(&trash)
            .map(|entries| entries.flatten().count() > 0)
            .unwrap_or(false),
        "the stray must be recoverable in typed trash"
    );
    let after = Graph::open(&dir).conflict_queue();
    assert!(
        !after
            .iter()
            .any(|o| o.source == crate::concord_queue::ConflictSource::DuplicateJournal),
        "the resolved day leaves the queue: {after:?}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn resolving_a_duplicate_day_refuses_files_from_different_days() {
    // The guard that keeps this from becoming a merge-any-two-pages command.
    let dir = duplicate_day_graph("queue-duplicate-journal-guard");
    fs::write(dir.join("journals").join("2026_06_24.md"), "- other day\n").unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let error = graph
        .resolve_duplicate_journal_day(
            "journals/2026_06_26.md",
            "journals/2026_06_24.md",
            &std::collections::HashMap::new(),
            "whatever",
            "whatever",
            "union",
        )
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(
        fs::read_to_string(dir.join("journals").join("2026_06_24.md"))
            .unwrap()
            .contains("other day"),
        "the unrelated day is untouched"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_cross_format_duplicate_day_offers_no_rows_but_still_lists_its_files() {
    // `merge_pages` and the sync-copy resolve both refuse a .md/.org pair, so
    // offering row choices we could never apply would be a dead end.
    let dir = scratch("queue-duplicate-journal-cross-format");
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq").join("config.edn"),
        "{:journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
    )
    .unwrap();
    fs::write(dir.join("journals").join("2026_06_26.md"), "- markdown\n").unwrap();
    fs::write(
        dir.join("journals").join("Friday, 26-06-2026.org"),
        "* org\n",
    )
    .unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();

    let queue = graph.conflict_queue();
    let day = queue
        .iter()
        .find(|o| o.source == crate::concord_queue::ConflictSource::DuplicateJournal)
        .expect("still a queue object");
    assert_eq!(day.sides.len(), 2, "both files are still listed");
    assert!(
        day.block_conflicts.is_none(),
        "no row-by-row choice on a pair that cannot be merged"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn journal_conflicts_reports_duplicate_days() {
    let dir = scratch("journal-conflicts");
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq").join("config.edn"),
        "{:preferred-format \"Org\"\n :journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
    )
    .unwrap();
    // Same day, two files (canonical stem + title-named) — a conflict.
    fs::write(
        dir.join("journals").join("2026_06_26.org"),
        "* canonical content\n",
    )
    .unwrap();
    fs::write(
        dir.join("journals").join("Friday, 26-06-2026.org"),
        "* stray content\n",
    )
    .unwrap();
    // A clean day with one file — not a conflict.
    fs::write(dir.join("journals").join("2026_06_24.org"), "* fine\n").unwrap();

    let conflicts = Graph::open(&dir).journal_conflicts();
    assert_eq!(
        conflicts.len(),
        1,
        "exactly one conflicted day: {conflicts:?}"
    );
    let c = &conflicts[0];
    assert_eq!(c.title, "Friday, 26-06-2026");
    assert_eq!(c.files.len(), 2);
    // Canonical (date-stem) file sorts first and is flagged; preview is the body line.
    assert_eq!(c.files[0].name, "2026_06_26.org");
    assert!(c.files[0].canonical);
    assert_eq!(c.files[0].preview, "canonical content");
    assert!(!c.files[1].canonical);
    assert_eq!(c.files[1].name, "Friday, 26-06-2026.org");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn journal_conflicts_reports_nested_duplicate_days() {
    let dir = scratch("journal-conflicts-nested");
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq").join("config.edn"),
        "{:preferred-format \"Org\"\n :journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
    )
    .unwrap();
    fs::create_dir_all(dir.join("journals").join("archive")).unwrap();
    fs::write(
        dir.join("journals").join("archive").join("2026_06_26.org"),
        "* canonical nested\n",
    )
    .unwrap();
    fs::write(
        dir.join("journals")
            .join("archive")
            .join("Friday, 26-06-2026.org"),
        "* stray nested\n",
    )
    .unwrap();

    let conflicts = Graph::open(&dir).journal_conflicts();
    assert_eq!(
        conflicts.len(),
        1,
        "nested duplicate day is surfaced: {conflicts:?}"
    );
    let files = &conflicts[0].files;
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].path, "journals/archive/2026_06_26.org");
    assert_eq!(files[1].path, "journals/archive/Friday, 26-06-2026.org");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn list_sync_conflicts_reports_nested_conflict_copy() {
    let dir = scratch("sync-conflicts-nested");
    fs::create_dir_all(dir.join("pages").join("client-a")).unwrap();
    fs::write(
        dir.join("pages").join("client-a").join("Foo.md"),
        "- base\n",
    )
    .unwrap();
    fs::write(
        dir.join("pages")
            .join("client-a")
            .join("Foo.sync-conflict-20260705-141233-A2B2C3D.md"),
        "- conflict copy\n",
    )
    .unwrap();

    let conflicts = Graph::open(&dir).list_sync_conflicts();
    assert_eq!(
        conflicts.len(),
        1,
        "nested sync-conflict copy is surfaced: {conflicts:?}"
    );
    let c = &conflicts[0];
    assert_eq!(
        c.path,
        "pages/client-a/Foo.sync-conflict-20260705-141233-A2B2C3D.md"
    );
    assert_eq!(c.base_path.as_deref(), Some("pages/client-a/Foo.md"));
    assert_eq!(c.base_name, "Foo");
    assert_eq!(c.kind, PageKind::Page);
    assert_eq!(c.preview, "conflict copy");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn real_page_with_sync_conflict_like_name_stays_indexed() {
    // The recognizer must match Syncthing's GENERATED shape
    // (`.sync-conflict-YYYYMMDD-HHMMSS-DEVICEID`), not a bare
    // `.sync-conflict-` substring — a real page whose name merely contains
    // the substring was silently deindexed as a false positive.
    let dir = scratch("conflict-lookalike-page");
    fs::write(
        dir.join("pages").join("Foo.sync-conflict-notes.md"),
        "- real content\n",
    )
    .unwrap();
    let graph = Graph::open(&dir);
    let pages = graph.list_pages();
    assert!(
        pages
            .iter()
            .any(|p| p.rel_path == "pages/Foo.sync-conflict-notes.md"),
        "a page whose name merely CONTAINS `.sync-conflict-` is a real page: {pages:?}"
    );
    assert!(
        graph.list_sync_conflicts().is_empty(),
        "a name without the generated timestamp shape is not a conflict copy"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn seafile_conflict_copy_is_surfaced_not_indexed() {
    // Seafile names conflict copies `<stem> (SFConflict <modifier>
    // <YYYY-MM-DD-HH-MM-SS>).<ext>` (seafile/common/vc-common.c,
    // `gen_conflict_path`). Left unrecognized, the copy is indexed as a
    // duplicate page — with `title::` it duplicates page identity.
    let dir = scratch("seafile-conflict");
    fs::write(dir.join("pages").join("Note.md"), "- winner\n").unwrap();
    fs::write(
        dir.join("pages")
            .join("Note (SFConflict me@example.com 2026-08-01-10-00-00).md"),
        "- conflict copy\n",
    )
    .unwrap();
    let graph = Graph::open(&dir);
    assert!(
        graph
            .list_pages()
            .iter()
            .all(|p| !p.rel_path.contains("SFConflict")),
        "a Seafile conflict copy must not be indexed as a page"
    );
    let conflicts = graph.list_sync_conflicts();
    assert_eq!(
        conflicts.len(),
        1,
        "the Seafile copy is surfaced in the conflicts workflow: {conflicts:?}"
    );
    assert_eq!(conflicts[0].base_name, "Note");
    assert_eq!(conflicts[0].base_path.as_deref(), Some("pages/Note.md"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn sync_conflict_base_matches_real_provider_formats_only() {
    for (stem, base) in [
        // Syncthing: `<stem>.sync-conflict-YYYYMMDD-HHMMSS-<short device id>`
        // (syncthing lib/model/folder_sendrecv.go `conflictName`; the device
        // id is up to 7 base32 chars [A-Z2-7], empty when the modifying
        // device is unknown; pre-1.1.0 versions omitted `-<device>`).
        ("Foo.sync-conflict-20260705-141233-A2B3C4D", Some("Foo")),
        ("Foo.sync-conflict-20260705-141233-", Some("Foo")),
        ("Foo.sync-conflict-20190201-124559", Some("Foo")),
        (
            "Foo.bar.sync-conflict-20260705-141233-ABCDEFG",
            Some("Foo.bar"),
        ),
        // Nested copy: the deepest tag wins, the base keeps the outer tag.
        (
            "Foo.sync-conflict-20260101-010101-AAAAAAA.sync-conflict-20260202-020202-BBBBBBB",
            Some("Foo.sync-conflict-20260101-010101-AAAAAAA"),
        ),
        // False positives the loose substring match used to deindex:
        ("Foo.sync-conflict-notes", None),
        ("Foo.sync-conflict-", None),
        ("Foo.sync-conflict-2026-08-01", None),
        ("Foo.sync-conflict-20260705", None),
        ("Foo.sync-conflict-20260705-141233x", None),
        ("Foo.sync-conflict-20260705-141233-abcdefg", None),
        ("Foo.sync-conflict-20260705-141233-ABCDEFGH", None),
        // Seafile: `<stem> (SFConflict [modifier ]YYYY-MM-DD-HH-MM-SS)`
        // (seafile/common/vc-common.c `gen_conflict_path`).
        (
            "Note (SFConflict me@example.com 2026-08-01-10-00-00)",
            Some("Note"),
        ),
        ("Note (SFConflict 2026-08-01-10-00-00)", Some("Note")),
        ("Note (SFConflict discussion)", None),
        ("Note (SFConflict 2026-08-01)", None),
        (
            "Note (SFConflict me@example.com 2026-08-01-10-00-00) extra",
            None,
        ),
        // Dropbox (behavior unchanged):
        ("Report (conflicted copy 2026-08-01)", Some("Report")),
        (
            "Report (Alice's conflicted copy 2026-08-01)",
            Some("Report"),
        ),
    ] {
        assert_eq!(sync_conflict_base(stem), base, "stem: {stem:?}");
    }
}

#[test]
fn marker_bearing_page_is_never_rewritten_by_save() {
    // A file holding git/Fossil merge conflict markers must be quarantined:
    // re-serializing it re-indents the column-0 markers as continuation
    // lines, which breaks git's own conflict detection. Saves are refused
    // with a typed refusal naming the markers; the bytes stay untouched.
    let dir = scratch("vcs-marker-quarantine");
    let original =
        "<<<<<<< HEAD\n- mine\n||||||| base\n- old\n=======\n- theirs\n>>>>>>> feature\n";
    fs::write(dir.join("pages").join("Merge.md"), original).unwrap();
    let graph = Graph::open(&dir);
    let mut page = graph
        .load_named("Merge", PageKind::Page)
        .unwrap()
        .expect("a marker-bearing page stays readable");
    assert!(!page.blocks.is_empty());
    page.blocks[0].raw = "mine edited".into();
    let base = page.rev.clone().unwrap();
    let result = graph.save_page(&page, Some(&base));
    let after = fs::read_to_string(dir.join("pages").join("Merge.md")).unwrap();
    assert_eq!(
        after, original,
        "a marker-bearing file must never be rewritten by Tine"
    );
    let error = result.expect_err("saves to a marker-bearing page are refused");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    let message = error.to_string();
    assert!(
        message.contains("<<<<<<<") && message.contains(">>>>>>>"),
        "the refusal names the markers it found: {message}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn vcs_marker_detection_matches_real_markers_only() {
    // git (merge and diff3 styles).
    assert_eq!(
            doc::vcs_conflict_markers(
                "<<<<<<< HEAD\n- mine\n||||||| merged common ancestors\n- old\n=======\n- theirs\n>>>>>>> feature\n"
            ),
            vec!["<<<<<<<", "|||||||", "=======", ">>>>>>>"]
        );
    // Fossil's verbose variants (mergeMarker table in fossil src/merge3.c).
    assert_eq!(
        doc::vcs_conflict_markers(concat!(
            "<<<<<<< BEGIN MERGE CONFLICT: local copy shown first <<<<<<<<<<<<\n",
            "- mine\n",
            "####### SUGGESTED CONFLICT RESOLUTION follows ###################\n",
            "- suggestion\n",
            "||||||| COMMON ANCESTOR content follows |||||||||||||||||||||||||\n",
            "- old\n",
            "======= MERGED IN content follows ===============================\n",
            "- theirs\n",
            ">>>>>>> END MERGE CONFLICT >>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>> (line 3)\n"
        )),
        vec!["<<<<<<<", "#######", "|||||||", "=======", ">>>>>>>"]
    );
    // Markers quoted inside a column-0 fenced code block (someone
    // DOCUMENTING git) must not flag the page.
    assert!(doc::vcs_conflict_markers(
        "```\n<<<<<<< HEAD\n=======\n>>>>>>> feature\n```\n- notes about git\n"
    )
    .is_empty());
    assert!(doc::vcs_conflict_markers("~~~text\n<<<<<<< HEAD\n>>>>>>> feature\n~~~\n").is_empty());
    // Markers quoted in an indented fence inside a bullet are not at
    // column 0 at all.
    assert!(doc::vcs_conflict_markers(
        "- how git conflicts look:\n  ```\n  <<<<<<< HEAD\n  =======\n  >>>>>>> theirs\n  ```\n"
    )
    .is_empty());
    // A lone `=======` (setext-style divider) never quarantines a page —
    // an anchor marker must be present.
    assert!(doc::vcs_conflict_markers("Heading\n=======\n- content\n").is_empty());
    // Markers must start at column 0 with their trailing space/shape.
    assert!(doc::vcs_conflict_markers("- <<<<<<< HEAD\n- >>>>>>> x\n").is_empty());
    // A real conflict below a closed fence is still detected.
    assert_eq!(
        doc::vcs_conflict_markers(
            "```\nexample\n```\n<<<<<<< HEAD\n- mine\n=======\n- theirs\n>>>>>>> feature\n"
        ),
        vec!["<<<<<<<", "=======", ">>>>>>>"]
    );
}

#[test]
fn list_vcs_marker_conflicts_reports_only_marker_pages() {
    let dir = scratch("vcs-marker-listing");
    fs::write(
        dir.join("pages").join("Merge.md"),
        "<<<<<<< HEAD\n- mine\n=======\n- theirs\n>>>>>>> feature\n",
    )
    .unwrap();
    fs::write(dir.join("pages").join("Clean.md"), "- ordinary page\n").unwrap();
    fs::write(
        dir.join("pages").join("Docs about git.md"),
        "```\n<<<<<<< HEAD\n=======\n>>>>>>> feature\n```\n",
    )
    .unwrap();
    // A sync-tool conflict copy containing markers belongs to the
    // conflict-copy listing, not this one.
    fs::write(
        dir.join("pages")
            .join("Merge.sync-conflict-20260817-101010-ABCDEFG.md"),
        "<<<<<<< HEAD\n- mine\n=======\n- theirs\n>>>>>>> feature\n",
    )
    .unwrap();
    let graph = Graph::open(&dir);
    let conflicts = graph.list_vcs_marker_conflicts();
    assert_eq!(
        conflicts.len(),
        1,
        "only the real marker-bearing page is listed: {conflicts:?}"
    );
    assert_eq!(conflicts[0].path, "pages/Merge.md");
    assert_eq!(conflicts[0].name, "Merge");
    assert_eq!(conflicts[0].kind, PageKind::Page);
    assert_eq!(conflicts[0].markers, vec!["<<<<<<<", "=======", ">>>>>>>"]);
    let _ = fs::remove_dir_all(&dir);
}

// --- Concord P4: the derived conflict queue + in-page marker resolution ---

/// Markers exactly as `git merge` writes them in `diff3` style.
const P4_DIFF3_MARKERS: &str = concat!(
    "- shared top\n",
    "<<<<<<< HEAD\n- mine wins\n",
    "||||||| merged common ancestors\n- original\n",
    "=======\n- theirs wins\n",
    ">>>>>>> feature\n",
);

#[test]
fn conflict_queue_derives_both_artifact_sources_and_survives_a_restart() {
    let dir = scratch("concord-queue-sources");
    fs::write(dir.join("pages").join("Notes.md"), "- winner text\n").unwrap();
    fs::write(
        dir.join("pages")
            .join("Notes.sync-conflict-20260817-101010-ABCDEFG.md"),
        "- copy text\n",
    )
    .unwrap();
    fs::write(dir.join("pages").join("Merged.md"), P4_DIFF3_MARKERS).unwrap();
    fs::write(dir.join("pages").join("Calm.md"), "- nothing wrong here\n").unwrap();

    let queue = Graph::open(&dir).conflict_queue();
    assert_eq!(
        queue.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        vec![
            "markers:pages/Merged.md",
            "copy:pages/Notes.sync-conflict-20260817-101010-ABCDEFG.md",
        ],
        "one object per artifact, ordered by page name: {queue:?}"
    );

    let markers = &queue[0];
    assert_eq!(
        markers.source,
        crate::concord_queue::ConflictSource::VcsMarkers
    );
    assert_eq!(markers.page_path, "pages/Merged.md");
    // Three sides: the diff3 marker block carries its own common ancestor.
    assert_eq!(
        markers.sides.iter().map(|s| s.role).collect::<Vec<_>>(),
        vec![
            crate::concord_queue::SideRole::Mine,
            crate::concord_queue::SideRole::Theirs,
            crate::concord_queue::SideRole::Base,
        ]
    );
    assert_eq!(markers.sides[0].label, "HEAD");
    assert_eq!(markers.sides[1].label, "feature");
    assert!(markers.block_conflicts.is_some_and(|n| n > 0));

    let copy = &queue[1];
    assert_eq!(copy.source, crate::concord_queue::ConflictSource::SyncCopy);
    assert_eq!(copy.page_name, "Notes");
    assert_eq!(copy.page_path, "pages/Notes.md");
    assert_eq!(
        copy.sides
            .iter()
            .filter_map(|s| s.path.clone())
            .collect::<Vec<_>>(),
        vec![
            "pages/Notes.md".to_string(),
            "pages/Notes.sync-conflict-20260817-101010-ABCDEFG.md".to_string(),
        ]
    );
    assert!(copy.block_conflicts.is_some_and(|n| n > 0));

    // The queue is DERIVED: a second, independent Graph over the same disk
    // state — what a restart is — reproduces it identically, with no stored
    // state of any kind (invariant 1).
    let after_restart = Graph::open(&dir).conflict_queue();
    assert_eq!(
        after_restart
            .iter()
            .map(|c| (c.id.clone(), c.block_conflicts))
            .collect::<Vec<_>>(),
        queue
            .iter()
            .map(|c| (c.id.clone(), c.block_conflicts))
            .collect::<Vec<_>>()
    );
    // And nothing was written into the graph to make that work.
    assert_eq!(
        fs::read_to_string(dir.join("pages").join("Merged.md")).unwrap(),
        P4_DIFF3_MARKERS
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn marker_conflict_diff_reads_the_pages_own_sides_without_writing() {
    let dir = scratch("concord-marker-diff");
    fs::write(dir.join("pages").join("Merged.md"), P4_DIFF3_MARKERS).unwrap();
    let graph = Graph::open(&dir);
    let parsed = graph
        .vcs_marker_conflict_diff("pages/Merged.md")
        .unwrap()
        .expect("a conflicted page");
    assert_eq!(parsed.mine_label, "HEAD");
    assert_eq!(parsed.theirs_label, "feature");
    assert_eq!(parsed.regions, 1);
    let diff = parsed.diff;
    assert!(diff.three_way, "the ||||||| section is a real ancestor");
    // Both staleness tokens address the ONE file the resolution will write.
    let rev = content_rev(&fs::read_to_string(dir.join("pages").join("Merged.md")).unwrap());
    assert_eq!(diff.base_rev, rev);
    assert_eq!(diff.conflict_rev, rev);
    // A page with no markers has no marker diff.
    fs::write(dir.join("pages").join("Calm.md"), "- fine\n").unwrap();
    assert!(Graph::open(&dir)
        .vcs_marker_conflict_diff("pages/Calm.md")
        .unwrap()
        .is_none());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn resolving_markers_keep_both_writes_sibling_blocks_and_clears_the_quarantine() {
    let dir = scratch("concord-marker-resolve");
    let rel = "pages/Merged.md";
    let file = dir.join("pages").join("Merged.md");
    fs::write(&file, P4_DIFF3_MARKERS).unwrap();
    let graph = Graph::open(&dir);

    // Before: the page is quarantined — an ordinary save is refused.
    let entry = graph.find_entry("Merged", PageKind::Page).unwrap();
    let page = graph.load_page(&entry).unwrap();
    assert!(
        graph.save_page(&page, page.rev.as_deref()).is_err(),
        "a marker-bearing page must refuse ordinary saves"
    );

    let diff = graph
        .vcs_marker_conflict_diff(rel)
        .unwrap()
        .expect("conflicted")
        .diff;
    // Keep-both on every decidable row — the no-loss default.
    let decisions: std::collections::HashMap<String, String> = collect_decidable_ids(&diff.rows)
        .into_iter()
        .map(|id| (id, "both".to_string()))
        .collect();
    graph
        .resolve_vcs_marker_conflict(rel, &decisions, &diff.base_rev, "union")
        .expect("resolution writes the merged result");

    let after = fs::read_to_string(&file).unwrap();
    assert!(
        doc::vcs_conflict_markers(&after).is_empty(),
        "no markers survive a resolution: {after:?}"
    );
    // Both sides are present, as adjacent sibling blocks of valid markdown.
    assert!(after.contains("- mine wins"), "{after:?}");
    assert!(after.contains("- theirs wins"), "{after:?}");
    assert!(after.contains("- shared top"), "{after:?}");
    let reparsed = doc::parse(&after);
    assert_eq!(
        reparsed
            .roots
            .iter()
            .map(|b| b.raw.trim().to_string())
            .collect::<Vec<_>>(),
        vec!["shared top", "mine wins", "theirs wins"]
    );
    // The quarantine lifts by itself: the file simply has no markers now.
    assert!(Graph::open(&dir).list_vcs_marker_conflicts().is_empty());
    assert!(Graph::open(&dir).conflict_queue().is_empty());
    let _ = fs::remove_dir_all(&dir);
}

/// A marker resolution rewrites the conflicted file IN PLACE, so the sides
/// the user did not choose survive nowhere else — the resolve must stage a
/// byte-exact copy of the pre-resolution file in the recoverable trash
/// (ADR 0007), like the sync-copy resolve trashes its conflict copy.
#[test]
fn resolving_markers_stages_the_preresolution_file_in_recoverable_trash() {
    let dir = scratch("concord-marker-resolve-trash");
    let rel = "pages/Merged.md";
    let file = dir.join("pages").join("Merged.md");
    fs::write(&file, P4_DIFF3_MARKERS).unwrap();
    let graph = Graph::open(&dir);
    let diff = graph
        .vcs_marker_conflict_diff(rel)
        .unwrap()
        .expect("conflicted")
        .diff;
    // Keep-mine everywhere — the LOSSY choice: "theirs wins" survives only
    // in the staged recovery copy.
    let decisions: std::collections::HashMap<String, String> = collect_decidable_ids(&diff.rows)
        .into_iter()
        .map(|id| (id, "mine".to_string()))
        .collect();
    graph
        .resolve_vcs_marker_conflict(rel, &decisions, &diff.base_rev, "union")
        .expect("resolution writes the merged result");
    assert!(
        !fs::read_to_string(&file).unwrap().contains("theirs wins"),
        "keep-mine drops the other side from the page itself"
    );
    let trash = dir.join("logseq").join(".tine-trash").join("conflicts");
    let staged: Vec<_> = fs::read_dir(&trash)
        .expect("the resolve staged a recovery copy")
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .ends_with("__markers__Merged.md")
        })
        .collect();
    assert_eq!(staged.len(), 1, "exactly one recovery copy");
    assert_eq!(
        fs::read_to_string(staged[0].path()).unwrap(),
        P4_DIFF3_MARKERS,
        "the recovery copy is the byte-exact pre-resolution file"
    );
    // A stale resolve refuses BEFORE staging anything.
    let dir2 = scratch("concord-marker-resolve-trash-stale");
    fs::write(dir2.join("pages").join("Merged.md"), P4_DIFF3_MARKERS).unwrap();
    let graph2 = Graph::open(&dir2);
    let diff2 = graph2
        .vcs_marker_conflict_diff(rel)
        .unwrap()
        .expect("conflicted")
        .diff;
    let decisions2: std::collections::HashMap<String, String> = collect_decidable_ids(&diff2.rows)
        .into_iter()
        .map(|id| (id, "mine".to_string()))
        .collect();
    graph2
        .resolve_vcs_marker_conflict(rel, &decisions2, "not-the-current-rev", "union")
        .expect_err("stale rev refuses");
    assert!(
        !dir2.join("logseq").join(".tine-trash").exists(),
        "a refused resolve must not materialize a trash directory"
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&dir2);
}

/// The marker resolver's ancestor is the SAME reconstructed base side the
/// marker diff used, so a `"merged"` row re-derives the body it offered.
#[test]
fn resolving_markers_can_apply_a_confirmed_merged_body() {
    const DISJOINT: &str = concat!(
        "- shared top\n",
        "<<<<<<< HEAD\n- the shared desktop machine label\n",
        "||||||| merged common ancestors\n- the shared desktop machine label 5\n",
        "=======\n- the shared desktop machine label 5 kk\n",
        ">>>>>>> feature\n",
    );
    let dir = scratch("concord-marker-merged");
    let rel = "pages/Merged.md";
    let file = dir.join("pages").join("Merged.md");
    fs::write(&file, DISJOINT).unwrap();
    let graph = Graph::open(&dir);
    let diff = graph.vcs_marker_conflict_diff(rel).unwrap().unwrap().diff;
    assert!(diff.three_way);
    let row = diff
        .rows
        .iter()
        .find(|row| row.merged.is_some())
        .expect("a merged proposal");
    assert_eq!(row.suggestion.as_deref(), Some("merged"));
    assert_eq!(
        row.merged.as_ref().unwrap().text,
        "the shared desktop machine label kk"
    );

    let decisions = std::collections::HashMap::from([(row.id.clone(), "merged".to_string())]);
    graph
        .resolve_vcs_marker_conflict(rel, &decisions, &diff.base_rev, "union")
        .expect("the confirmed merged body applies");
    let after = fs::read_to_string(&file).unwrap();
    assert!(doc::vcs_conflict_markers(&after).is_empty(), "{after:?}");
    assert_eq!(
        doc::parse(&after)
            .roots
            .iter()
            .map(|b| b.raw.trim().to_string())
            .collect::<Vec<_>>(),
        vec!["shared top", "the shared desktop machine label kk"]
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Fossil's own `####### SUGGESTED CONFLICT RESOLUTION` is the second
/// source for the fourth outcome: the two edits here OVERLAP, so the
/// disjoint-edit merge declines and the artifact is what fills the row.
/// Resolving writes exactly the suggested body — re-derived from the
/// guarded file bytes, never echoed back from the client.
#[test]
fn resolving_fossil_markers_can_apply_the_suggested_resolution() {
    const FOSSIL: &str = concat!(
        "- shared top\n",
        "<<<<<<< BEGIN MERGE CONFLICT: local copy shown first <<<<<<<<<<<<<<<\n",
        "- the quick brown fox jumped over it\n",
        "####### SUGGESTED CONFLICT RESOLUTION follows ##################\n",
        "- the quick brown fox leapt over it\n",
        "||||||| COMMON ANCESTOR content follows |||||||||||||||||||||||||\n",
        "- the quick brown fox jumps over it\n",
        "======= MERGED IN content follows ==============================\n",
        "- the quick brown fox leaped over it\n",
        ">>>>>>> END MERGE CONFLICT >>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>\n",
    );
    let dir = scratch("concord-marker-artifact");
    let rel = "pages/Merged.md";
    let file = dir.join("pages").join("Merged.md");
    fs::write(&file, FOSSIL).unwrap();
    let graph = Graph::open(&dir);
    let diff = graph.vcs_marker_conflict_diff(rel).unwrap().unwrap().diff;
    assert!(diff.three_way);
    let row = diff
        .rows
        .iter()
        .find(|row| row.merged.is_some())
        .expect("an artifact proposal");
    let proposal = row.merged.as_ref().unwrap();
    assert_eq!(
        proposal.source,
        crate::sync_diff::MergedSource::Artifact,
        "the edits overlap, so nothing was computed"
    );
    assert_eq!(proposal.text, "the quick brown fox leapt over it");
    assert_eq!(row.suggestion.as_deref(), Some("merged"));

    let decisions = std::collections::HashMap::from([(row.id.clone(), "merged".to_string())]);
    graph
        .resolve_vcs_marker_conflict(rel, &decisions, &diff.base_rev, "union")
        .expect("the confirmed artifact applies");
    let after = fs::read_to_string(&file).unwrap();
    assert!(doc::vcs_conflict_markers(&after).is_empty(), "{after:?}");
    assert_eq!(
        doc::parse(&after)
            .roots
            .iter()
            .map(|b| b.raw.trim().to_string())
            .collect::<Vec<_>>(),
        vec!["shared top", "the quick brown fox leapt over it"]
    );
    // The suggestion text itself never leaked into a side.
    assert!(!after.contains("jumped"), "{after:?}");
    assert!(!after.contains("leaped"), "{after:?}");
    // And the stale-rev guard still fires against the ORIGINAL rev.
    fs::write(&file, FOSSIL).unwrap();
    let graph = Graph::open(&dir);
    let err = graph
        .resolve_vcs_marker_conflict(rel, &decisions, "not-the-current-rev", "union")
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read_to_string(&file).unwrap(), FOSSIL);
    let _ = fs::remove_dir_all(&dir);
}

/// A suggestion region that merely repeats one side is not a fourth
/// outcome, so nothing is offered — and a forged `"merged"` refuses the
/// whole resolve, leaving the marker file byte-identical.
#[test]
fn a_fossil_suggestion_equal_to_a_side_offers_nothing_and_writes_nothing() {
    const FOSSIL: &str = concat!(
        "- shared top\n",
        "<<<<<<< BEGIN MERGE CONFLICT: local copy shown first <<<<<<<<<<<<<<<\n",
        "- the quick brown fox jumped over it\n",
        "####### SUGGESTED CONFLICT RESOLUTION follows ##################\n",
        "- the quick brown fox leaped over it\n",
        "||||||| COMMON ANCESTOR content follows |||||||||||||||||||||||||\n",
        "- the quick brown fox jumps over it\n",
        "======= MERGED IN content follows ==============================\n",
        "- the quick brown fox leaped over it\n",
        ">>>>>>> END MERGE CONFLICT >>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>\n",
    );
    let dir = scratch("concord-marker-artifact-refused");
    let rel = "pages/Merged.md";
    let file = dir.join("pages").join("Merged.md");
    fs::write(&file, FOSSIL).unwrap();
    let graph = Graph::open(&dir);
    let diff = graph.vcs_marker_conflict_diff(rel).unwrap().unwrap().diff;
    assert!(diff.three_way);
    assert!(
        diff.rows.iter().all(|row| row.merged.is_none()),
        "a proposal equal to a side duplicates an existing choice"
    );
    let decisions: std::collections::HashMap<String, String> = collect_decidable_ids(&diff.rows)
        .into_iter()
        .map(|id| (id, "merged".to_string()))
        .collect();
    let error = graph
        .resolve_vcs_marker_conflict(rel, &decisions, &diff.base_rev, "union")
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{error}");
    assert_eq!(fs::read_to_string(&file).unwrap(), FOSSIL);
    let _ = fs::remove_dir_all(&dir);
}

/// Without a reconstructed ancestor (a 2-marker conflict) the diff offers
/// nothing to merge, and a forged `"merged"` decision refuses the resolve.
#[test]
fn markers_without_an_ancestor_refuse_a_forged_merged_decision() {
    const NO_BASE: &str = concat!(
        "- shared top\n",
        "<<<<<<< HEAD\n- the shared desktop machine label\n",
        "=======\n- the shared desktop machine label 5 kk\n",
        ">>>>>>> feature\n",
    );
    let dir = scratch("concord-marker-nobase");
    let rel = "pages/Merged.md";
    let file = dir.join("pages").join("Merged.md");
    fs::write(&file, NO_BASE).unwrap();
    let graph = Graph::open(&dir);
    let diff = graph.vcs_marker_conflict_diff(rel).unwrap().unwrap().diff;
    assert!(!diff.three_way);
    assert!(diff.rows.iter().all(|row| row.merged.is_none()));
    let decidable = collect_decidable_ids(&diff.rows);
    let decisions: std::collections::HashMap<String, String> = decidable
        .iter()
        .map(|id| (id.clone(), "merged".to_string()))
        .collect();
    let error = graph
        .resolve_vcs_marker_conflict(rel, &decisions, &diff.base_rev, "union")
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{error}");
    assert_eq!(fs::read_to_string(&file).unwrap(), NO_BASE);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn marker_resolution_is_guarded_and_never_leaves_the_file_writable() {
    let dir = scratch("concord-marker-guards");
    let rel = "pages/Merged.md";
    let file = dir.join("pages").join("Merged.md");
    fs::write(&file, P4_DIFF3_MARKERS).unwrap();
    let graph = Graph::open(&dir);
    let diff = graph.vcs_marker_conflict_diff(rel).unwrap().unwrap().diff;
    let decisions = std::collections::HashMap::new();

    // Stale base_rev → refuse without writing (the VCS moved under the UI).
    let err = graph
        .resolve_vcs_marker_conflict(rel, &decisions, "not-the-current-rev", "union")
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read_to_string(&file).unwrap(), P4_DIFF3_MARKERS);

    // A page with no markers is not a resolution target.
    fs::write(dir.join("pages").join("Calm.md"), "- fine\n").unwrap();
    let calm = Graph::open(&dir);
    let calm_rev = content_rev("- fine\n");
    assert_eq!(
        calm.resolve_vcs_marker_conflict("pages/Calm.md", &decisions, &calm_rev, "union")
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );

    // The exemption is scoped to the one resolution: after it, ordinary
    // saves to a still-marker-bearing page are refused again.
    fs::write(dir.join("pages").join("Other.md"), P4_DIFF3_MARKERS).unwrap();
    let graph = Graph::open(&dir);
    graph
        .resolve_vcs_marker_conflict(rel, &decisions, &diff.base_rev, "union")
        .expect("the real resolution succeeds");
    let other_entry = graph.find_entry("Other", PageKind::Page).unwrap();
    let other = graph.load_page(&other_entry).unwrap();
    assert!(
        graph.save_page(&other, other.rev.as_deref()).is_err(),
        "the other marker page stays quarantined"
    );
    // And the resolved page is now an ordinary, savable page.
    let resolved_entry = graph.find_entry("Merged", PageKind::Page).unwrap();
    let resolved = graph.load_page(&resolved_entry).unwrap();
    assert!(graph.save_page(&resolved, resolved.rev.as_deref()).is_ok());
    let _ = fs::remove_dir_all(&dir);
}

fn collect_decidable_ids(rows: &[crate::sync_diff::DiffRow]) -> Vec<String> {
    let mut out = Vec::new();
    for row in rows {
        if row.kind != crate::sync_diff::RowKind::Unchanged {
            out.push(row.id.clone());
        }
        out.extend(collect_decidable_ids(&row.children));
    }
    out
}

#[test]
fn journals_desc_dedups_duplicate_day_to_canonical() {
    // The feed must show a day ONCE even when two files resolve to it — else
    // the same day renders twice (loaded from whichever file path_for picks).
    let dir = scratch("journal-dedup");
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq").join("config.edn"),
        "{:preferred-format \"Org\"\n :journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
    )
    .unwrap();
    fs::write(dir.join("journals").join("2026_06_26.org"), "* real day\n").unwrap();
    fs::write(
        dir.join("journals").join("Friday, 26-06-2026.org"),
        "* stray\n",
    )
    .unwrap();
    fs::write(dir.join("journals").join("2026_06_24.org"), "* other day\n").unwrap();

    let js = Graph::open(&dir).journals_desc();
    assert_eq!(
        js.len(),
        2,
        "one entry per day: {:?}",
        js.iter().map(|e| &e.name).collect::<Vec<_>>()
    );
    // The deduped 26th keeps the canonical date-stem file (what saves resolve to).
    let day26 = js
        .iter()
        .find(|e| e.name == "Friday, 26-06-2026")
        .expect("26th present");
    assert_eq!(
        day26.path.file_name().unwrap().to_str().unwrap(),
        "2026_06_26.org"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn cold_journal_inventory_does_not_read_or_parse_ordinary_pages() {
    let dir = scratch("cold-journal-inventory-metadata-only");
    for index in 0..128 {
        fs::write(
            dir.join("pages").join(format!("Ordinary {index}.md")),
            format!("- ordinary {index}\n"),
        )
        .unwrap();
    }
    fs::write(dir.join("journals/2026_08_22.md"), "- today\n").unwrap();
    fs::write(dir.join("journals/2026_08_21.md"), "- yesterday\n").unwrap();
    let graph = Graph::open(&dir);

    GRAPH_TEXT_CONTENT_READS.with(|reads| reads.set(0));
    GRAPH_TEXT_PARSE_ATTEMPTS.with(|attempts| attempts.set(0));
    let journals = graph.journals_desc();

    assert_eq!(journals.len(), 2);
    assert_eq!(GRAPH_TEXT_CONTENT_READS.with(Cell::get), 0);
    assert_eq!(GRAPH_TEXT_PARSE_ATTEMPTS.with(Cell::get), 0);
    assert!(graph.cache.read().unwrap().is_none());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn future_journals_are_feed_only_excluded_but_keep_raw_identity() {
    let dir = scratch("future-feed-raw-identity");
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
            dir.join("logseq/config.edn"),
            "{:journal/file-name-format \"dd-MM-yyyy\"\n :journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
        )
        .unwrap();
    let future = dir.join("journals/17-07-2030.md");
    let future_bytes = b"- future-search-sentinel\n";
    fs::write(&future, future_bytes).unwrap();
    fs::write(dir.join("journals/15-07-2030.md"), "- today sentinel\n").unwrap();
    fs::write(dir.join("journals/14-07-2030.md"), "- past sentinel\n").unwrap();
    let g = ready_graph(&dir);
    let future_title = "Wednesday, 17-07-2030";
    assert_eq!(
        g.journals_desc().len(),
        3,
        "raw inventory retains future journals"
    );
    let feed = g.feed_journals_desc_through(JournalDate {
        year: 2030,
        month: 7,
        day: 15,
    });
    assert_eq!(
        feed.iter().map(|e| e.date_key).collect::<Vec<_>>(),
        vec![Some(20300715), Some(20300714)]
    );
    let future_entry = g
        .journals_desc()
        .into_iter()
        .find(|e| e.date_key == Some(20300717))
        .unwrap();
    assert_eq!(future_entry.path, future);
    assert_eq!(
        g.load_page(&future_entry).unwrap().blocks[0].raw,
        "future-search-sentinel"
    );
    assert!(g.list_pages().iter().any(|e| e.path == future));
    assert_eq!(
        g.find_entry(future_title, PageKind::Journal).unwrap().path,
        future
    );
    assert_eq!(
        g.load_named(future_title, PageKind::Journal)
            .unwrap()
            .unwrap()
            .blocks[0]
            .raw,
        "future-search-sentinel"
    );
    // Ctrl-K uses the current combined latest-wins graph-search path, not
    // the legacy quick_switch adapter. Its whole-graph inventory remains
    // deliberately separate from the filtered Journals feed.
    assert!(g
        .run_graph_search_latest("future-feed-test", future_title, 8, 8, false)
        .unwrap()
        .hits
        .iter()
        .any(|hit| matches!(hit,
            crate::query_plan::QueryHit::Page { page, .. } if page.path == future
        )));
    assert!(!g.search("future-search-sentinel", 8).unwrap().is_empty());
    assert_eq!(g.path_for(future_title, PageKind::Journal), future);
    assert_eq!(
        g.page_source_file(future_title, PageKind::Journal, None)
            .unwrap(),
        future.canonicalize().unwrap()
    );
    assert_eq!(
        fs::read(&future).unwrap(),
        future_bytes,
        "feed/list/search performed no write"
    );

    // The warmed cache retains exactly the cold membership/order and later
    // whole-graph lookups still see the excluded future page.
    g.warm_cache();
    assert_eq!(
        g.feed_journals_desc_through(JournalDate {
            year: 2030,
            month: 7,
            day: 15
        })
        .iter()
        .map(|e| e.date_key)
        .collect::<Vec<_>>(),
        vec![Some(20300715), Some(20300714)]
    );
    assert!(g
        .run_graph_search_latest("future-feed-test", future_title, 8, 8, false)
        .unwrap()
        .hits
        .iter()
        .any(|hit| matches!(hit,
            crate::query_plan::QueryHit::Page { page, .. } if page.path == future
        )));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn warmed_save_cache_upsert_keeps_future_and_duplicate_days_out_of_feed() {
    let dir = scratch("future-feed-warm-save");
    let g = Graph::open(&dir);
    g.warm_cache();

    let mut past = jdto("Jul 14th, 2030");
    past.blocks[0].raw = "past after warm cache".into();
    g.save_page(&past, None).unwrap();
    let mut today = jdto("Jul 15th, 2030");
    today.blocks[0].raw = "today after warm cache".into();
    g.save_page(&today, None).unwrap();
    let mut future = jdto("Jul 17th, 2030");
    future.blocks[0].raw = "future after warm cache".into();
    g.save_page(&future, None).unwrap();

    let cutoff = JournalDate {
        year: 2030,
        month: 7,
        day: 15,
    };
    assert_eq!(
        g.feed_journals_desc_through(cutoff)
            .iter()
            .map(|e| e.date_key)
            .collect::<Vec<_>>(),
        vec![Some(20300715), Some(20300714)],
        "guarded save/cache-upsert must not leak a future day into warm feed membership"
    );
    assert!(g
        .load_named("Jul 17th, 2030", PageKind::Journal)
        .unwrap()
        .is_some());
    assert!(g.list_pages().iter().any(|e| e.name == "Jul 17th, 2030"));

    // The raw inventory retains duplicate future files for conflict discovery,
    // while date deduplication still leaves no future feed row at all.
    fs::write(dir.join("journals/2030_07_17.org"), "* future twin\n").unwrap();
    let duplicate = Graph::open(&dir);
    assert_eq!(
        duplicate
            .journals_desc()
            .iter()
            .filter(|e| e.date_key == Some(20300717))
            .count(),
        1
    );
    assert!(duplicate
        .feed_journals_desc_through(cutoff)
        .iter()
        .all(|e| e.date_key != Some(20300717)));
    assert!(
        !duplicate.journal_conflicts().is_empty(),
        "future duplicate remains discoverable outside feed"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn rename_transaction_moves_file_and_rewrites_refs() {
    let dir = scratch("rename");
    fs::write(dir.join("pages").join("Alpha.md"), "- alpha body\n").unwrap();
    fs::write(dir.join("pages").join("Other.md"), "- see [[Alpha]] here\n").unwrap();
    let g = Graph::open(&dir);
    g.warm_cache();
    g.rename_page("Alpha", "Beta").unwrap();
    // The page file moved (content preserved) and the old file is gone.
    assert!(!dir.join("pages").join("Alpha.md").exists());
    assert_eq!(
        fs::read_to_string(dir.join("pages").join("Beta.md")).unwrap(),
        "- alpha body\n"
    );
    // Every reference was rewritten across the graph.
    let other = fs::read_to_string(dir.join("pages").join("Other.md")).unwrap();
    assert!(other.contains("[[Beta]]"), "ref rewritten to [[Beta]]");
    assert!(!other.contains("[[Alpha]]"), "no stale [[Alpha]] left");
    let _ = fs::remove_dir_all(&dir);
}

/// REG-DIRECT-RENAME-RETAINED-SHADOW-LIMIT-364 causal witness. A rename
/// already performs one bounded, no-follow inventory and exact per-file
/// rechecks. Its nested publication primitives must not attempt to build a
/// second whole-graph retained-shadow index merely to write those files.
#[test]
fn rename_transaction_does_not_build_the_guarded_graph_index() {
    let dir = scratch("rename-without-guarded-graph-index");
    fs::write(dir.join("pages/Alpha.md"), "- alpha body\n").unwrap();
    fs::write(dir.join("pages/Other.md"), "- see [[Alpha]] here\n").unwrap();
    let graph = Graph::open(&dir);

    let before = graph.guarded_graph_text_identity_report();
    GRAPH_TEXT_FIRST_CAPTURE_CHARGE_OVERRIDE
        .with(|charge| charge.set(Some(GRAPH_TEXT_CAPTURE_LIMITS.peak_build_bytes)));
    graph
        .rename_page("Alpha", "Beta")
        .expect("bounded rename must not enter retained-shadow construction");
    let after = graph.guarded_graph_text_identity_report();

    assert_eq!(after.complete_builds, before.complete_builds);
    assert_eq!(
        GRAPH_TEXT_FIRST_CAPTURE_CHARGE_OVERRIDE.with(Cell::take),
        Some(GRAPH_TEXT_CAPTURE_LIMITS.peak_build_bytes),
        "rename consumed the retained capture hook"
    );
    assert!(!dir.join("pages/Alpha.md").exists());
    assert_eq!(
        fs::read_to_string(dir.join("pages/Beta.md")).unwrap(),
        "- alpha body\n"
    );
    assert_eq!(
        fs::read_to_string(dir.join("pages/Other.md")).unwrap(),
        "- see [[Beta]] here\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(any(unix, windows))]
#[test]
fn rename_transaction_inventory_still_refuses_physical_file_aliases() {
    let dir = scratch("rename-inventory-hardlink-refusal");
    let alpha = dir.join("pages/Alpha.md");
    let alias = dir.join("pages/Alias.md");
    fs::write(&alpha, "- alpha body\n").unwrap();
    fs::hard_link(&alpha, &alias).unwrap();
    let before = regular_file_tree(&dir.join("pages"));

    let error = Graph::open(&dir).rename_page("Alpha", "Beta").unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{error}");
    assert!(error.to_string().contains("alias one resource"));
    assert_eq!(regular_file_tree(&dir.join("pages")), before);
    assert!(!dir.join("pages/Beta.md").exists());
    let _ = fs::remove_dir_all(&dir);
}

/// Reporter-shape receipt for GH #364. This is intentionally an explicit
/// release-gate probe rather than a per-commit unit test: creating 13,000
/// files is real filesystem work, while the causal no-index test above is
/// fast enough for the ordinary suite.
#[test]
#[ignore = "large reporter-shape regression; run before release"]
fn rename_transaction_succeeds_on_thirteen_thousand_page_graph() {
    const PAGE_COUNT: usize = 13_000;
    let dir = scratch("rename-thirteen-thousand-pages");
    fs::write(dir.join("pages/Alpha.md"), "- alpha body\n").unwrap();
    for index in 1..PAGE_COUNT {
        let body = if index == PAGE_COUNT - 1 {
            "- final reference [[Alpha]]\n"
        } else {
            "- unrelated\n"
        };
        fs::write(dir.join("pages").join(format!("Page{index:05}.md")), body).unwrap();
    }
    let graph = Graph::open(&dir);
    let started = std::time::Instant::now();
    graph
        .rename_page("Alpha", "Beta")
        .expect("13k-page Direct Files rename must remain bounded");
    let elapsed = started.elapsed();

    assert!(!dir.join("pages/Alpha.md").exists());
    assert_eq!(
        fs::read_to_string(dir.join("pages/Beta.md")).unwrap(),
        "- alpha body\n"
    );
    assert_eq!(
        fs::read_to_string(dir.join("pages/Page12999.md")).unwrap(),
        "- final reference [[Beta]]\n"
    );
    assert_eq!(regular_file_tree(&dir.join("pages")).len(), PAGE_COUNT);
    eprintln!("GH #364 13k-page rename completed in {elapsed:?}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn rename_rolls_back_destination_when_source_remove_fails() {
    let dir = scratch("rename-remove-failure");
    let original = "- alpha body\n";
    let ref_original = "- see [[Alpha]] here\n";
    fs::write(dir.join("pages/Alpha.md"), original).unwrap();
    fs::write(dir.join("pages/Other.md"), ref_original).unwrap();
    let g = Graph::open(&dir);
    g.warm_cache();
    FAIL_NEXT_RENAME_SOURCE_REMOVE.with(|flag| flag.set(true));
    WITHDRAW_RACE_REPLACEMENT.with(|replacement| {
        *replacement.borrow_mut() = Some(b"- external replacement\n".to_vec());
    });
    assert!(g.rename_page("Alpha", "Beta").is_err());
    assert_eq!(
        fs::read_to_string(dir.join("pages/Alpha.md")).unwrap(),
        original
    );
    assert_eq!(
        fs::read_to_string(dir.join("pages/Beta.md")).unwrap(),
        "- external replacement\n",
        "rollback must not unlink a destination replaced after its check"
    );
    assert_eq!(
        fs::read_to_string(dir.join("pages/Other.md")).unwrap(),
        ref_original
    );
    let _ = fs::remove_dir_all(dir);
}

#[cfg(any(unix, windows))]
fn assert_failed_editor_publication_is_searchable(ending: &str) {
    let dir = scratch(ending);
    let path = dir.join("pages/Alpha.md");
    fs::write(&path, "- numbat original\n").unwrap();
    let graph = Graph::open(&dir);
    graph
        .attach_direct_projection(dir.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    graph.with_pages(|_| ());
    let projection = graph.direct_projection_test().unwrap();
    let wait = || {
        let deadline = Instant::now() + Duration::from_secs(5);
        assert!(projection
            .wait_until_ready_at(graph.cache_generation(), &|| { Instant::now() >= deadline }));
    };
    wait();
    assert_eq!(graph.search("numbat", 20).unwrap().len(), 1);
    let mut page = graph.load_by_path("pages/Alpha.md").unwrap().unwrap();
    page.blocks[0].raw = "quokka edited".to_string();
    let injected_error = || Err(io::Error::other("editor publication probe"));
    match ending {
        "editor-post-publication-error" => JOURNAL_PROJECTION_AFTER_PUBLISH.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(injected_error));
        }),
        "editor-final-read-error" => EDITOR_COMMIT_BEFORE_FINAL_REREAD.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(injected_error));
        }),
        "editor-external-winner" => JOURNAL_PROJECTION_AFTER_PUBLISH.with(|hook| {
            let path = path.clone();
            *hook.borrow_mut() = Some(Box::new(move || {
                let replacement = path.with_file_name(".external-winner");
                fs::write(&replacement, "- quokka external\n")?;
                gh254_replace(&path, &replacement)
            }));
        }),
        _ => unreachable!(),
    }
    let error = graph.save_page(&page, page.rev.as_deref()).unwrap_err();
    let external = ending == "editor-external-winner";
    if external {
        assert_eq!(gh254_code(&error), "conflict.replace_post_publication");
    } else {
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(error.to_string(), "editor publication probe");
    }
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        if external {
            "- quokka external\n"
        } else {
            "- quokka edited\n"
        }
    );
    let assert_search = || {
        wait();
        for _ in 0..5 {
            let groups = graph.search("quokka", 20).unwrap();
            assert_eq!(
                groups
                    .iter()
                    .map(|group| group.page.as_str())
                    .collect::<Vec<_>>(),
                vec!["Alpha"]
            );
            assert!(graph.search("numbat", 20).unwrap().is_empty());
            assert_eq!(
                graph
                    .search(if external { "external" } else { "edited" }, 20)
                    .unwrap()
                    .len(),
                1
            );
            if external {
                assert!(graph.search("edited", 20).unwrap().is_empty());
            }
        }
    };
    assert_search();
    graph.sync_file_checked(&path).unwrap();
    assert_search();
}

#[cfg(any(unix, windows))]
#[test]
fn a_save_that_fails_after_editor_publication_still_indexes_the_live_text() {
    assert_failed_editor_publication_is_searchable("editor-post-publication-error");
}

#[cfg(any(unix, windows))]
#[test]
fn a_save_that_fails_at_the_editor_final_read_still_indexes_the_live_text() {
    assert_failed_editor_publication_is_searchable("editor-final-read-error");
}

#[cfg(any(unix, windows))]
#[test]
fn a_save_that_loses_editor_publication_to_an_external_owner_still_indexes_the_live_text() {
    assert_failed_editor_publication_is_searchable("editor-external-winner");
}

#[test]
fn a_failed_rename_publishes_the_destination_that_survived_it() {
    let dir = scratch("failed-rename-compensation");
    fs::write(dir.join("pages/Alpha.md"), "- numbat original\n").unwrap();
    let graph = Graph::open(&dir);
    graph
        .attach_direct_projection(dir.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    graph.with_pages(|_| ());
    let projection = graph.direct_projection_test().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    assert!(
        projection.wait_until_ready_at(graph.cache_generation(), &|| Instant::now() >= deadline)
    );
    FAIL_NEXT_RENAME_SOURCE_REMOVE.with(|flag| flag.set(true));
    WITHDRAW_RACE_REPLACEMENT.with(|replacement| {
        *replacement.borrow_mut() = Some(b"- quokka occupant\n".to_vec());
    });
    assert!(graph.rename_page("Alpha", "Beta").is_err());
    assert_eq!(
        fs::read_to_string(dir.join("pages/Beta.md")).unwrap(),
        "- quokka occupant\n"
    );
    for _ in 0..600 {
        if graph
            .search("quokka", 20)
            .unwrap()
            .iter()
            .any(|g| g.page == "Beta")
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    for _ in 0..5 {
        let groups = graph.search("quokka", 20).unwrap();
        assert_eq!(
            groups.iter().map(|g| g.page.as_str()).collect::<Vec<_>>(),
            vec!["Beta"],
            "failed rename left surviving bytes unindexed: {}",
            projection.debug_state_test()
        );
    }
}

#[test]
fn a_failed_duplicate_resolution_publishes_the_occupant_it_could_not_remove() {
    let dir = scratch("failed-conflict-compensation");
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq/config.edn"),
        "{:journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
    )
    .unwrap();
    let winner = "journals/2026_06_26.md";
    let stray = "journals/Friday, 26-06-2026.md";
    fs::write(dir.join(winner), "- shared\n").unwrap();
    fs::write(dir.join(stray), "- shared\n- numbat old\n").unwrap();
    let graph = Graph::open(&dir);
    graph
        .attach_direct_projection(dir.join("private/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    graph.with_pages(|_| ());
    let projection = graph.direct_projection_test().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    assert!(
        projection.wait_until_ready_at(graph.cache_generation(), &|| Instant::now() >= deadline)
    );
    let diff = graph
        .duplicate_journal_diff(winner, stray)
        .unwrap()
        .unwrap();
    fn decisions(
        rows: &[crate::sync_diff::DiffRow],
        out: &mut std::collections::HashMap<String, String>,
    ) {
        for row in rows {
            if row.kind != crate::sync_diff::RowKind::Unchanged {
                out.insert(row.id.clone(), "both".into());
            }
            decisions(&row.children, out);
        }
    }
    let mut choices = std::collections::HashMap::new();
    decisions(&diff.rows, &mut choices);
    let dst = dir.join(winner);
    EDITOR_COMMIT_BEFORE_RECHECK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || fs::write(&dst, "- winner changed\n")));
    });
    let src = dir.join(stray);
    GRAPH_TEXT_WRITE_DURING_ROLLBACK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || fs::write(&src, "- quokka occupant\n")));
    });
    assert!(graph
        .resolve_duplicate_journal_day(
            winner,
            stray,
            &choices,
            &diff.base_rev,
            &diff.conflict_rev,
            "union"
        )
        .is_err());
    assert_eq!(
        fs::read_to_string(dir.join(stray)).unwrap(),
        "- quokka occupant\n"
    );
    for _ in 0..600 {
        if !graph.search("quokka", 20).unwrap().is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    for _ in 0..5 {
        assert!(
            !graph.search("quokka", 20).unwrap().is_empty(),
            "current occupant absent: {}",
            projection.debug_state_test()
        );
        assert!(
            graph.search("numbat", 20).unwrap().is_empty(),
            "historical stray still searchable"
        );
    }
    graph.with_pages(|pages| {
        assert!(pages
            .iter()
            .any(|(_, document)| doc::serialize(document).contains("quokka")))
    });
}

#[test]
fn rename_namespace_rewrites_all_descendant_refs_in_one_pass() {
    // A namespace rename (`Project` -> `Archive`) moves the primary page AND
    // every file-backed descendant, and rewrites every reference to ANY of
    // them across the graph in a SINGLE multi-target pass per file (perf
    // Codex#2). Default file-name format is Legacy, so `Project/Alpha` lives
    // on disk as `Project%2FAlpha.md`.
    let dir = scratch("rename-ns");
    fs::write(dir.join("pages").join("Project.md"), "- project body\n").unwrap();
    fs::write(
        dir.join("pages").join("Project%2FAlpha.md"),
        "- alpha body\n",
    )
    .unwrap();
    fs::write(dir.join("pages").join("Project%2FBeta.md"), "- beta body\n").unwrap();
    // One file references the primary AND both descendants (inline) plus two
    // bare `tags::` values — all rewritten in the single multi-target pass.
    fs::write(
            dir.join("pages").join("Refs.md"),
            "tags:: Project, Project/Beta\n- see [[Project]], [[Project/Alpha]] and #[[Project/Beta]]\n",
        )
        .unwrap();
    let g = Graph::open(&dir);
    g.warm_cache();
    g.rename_page("Project", "Archive").unwrap();

    // Primary + every descendant file moved (content preserved), old names gone.
    assert!(!dir.join("pages").join("Project.md").exists());
    assert!(!dir.join("pages").join("Project%2FAlpha.md").exists());
    assert!(!dir.join("pages").join("Project%2FBeta.md").exists());
    assert_eq!(
        fs::read_to_string(dir.join("pages").join("Archive.md")).unwrap(),
        "- project body\n"
    );
    assert_eq!(
        fs::read_to_string(dir.join("pages").join("Archive%2FAlpha.md")).unwrap(),
        "- alpha body\n"
    );
    assert_eq!(
        fs::read_to_string(dir.join("pages").join("Archive%2FBeta.md")).unwrap(),
        "- beta body\n"
    );

    // Every inline ref AND both bare tag values rewritten; no stale `Project`.
    let refs = fs::read_to_string(dir.join("pages").join("Refs.md")).unwrap();
    assert!(refs.contains("[[Archive]]"), "primary inline ref: {refs:?}");
    assert!(
        refs.contains("[[Archive/Alpha]]"),
        "descendant inline ref: {refs:?}"
    );
    // `Archive/Beta` is bare-tag-safe (`/` is a tag char), so `#[[..]]`
    // collapses to the bare `#Archive/Beta` form, matching Logseq.
    assert!(
        refs.contains("#Archive/Beta"),
        "descendant tag ref: {refs:?}"
    );
    assert!(
        refs.contains("tags:: Archive, Archive/Beta"),
        "bare tags rewritten: {refs:?}"
    );
    assert!(
        !refs.contains("Project"),
        "no stale Project anywhere: {refs:?}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn org_page_lists_loads_edits_and_round_trips() {
    let dir = scratch("org-page");
    let src = "* TODO Buy milk\nSCHEDULED: <2026-06-25 Thu>\n* second block\n";
    fs::write(dir.join("pages").join("Org Notes.org"), src).unwrap();
    let g = Graph::open(&dir);
    g.warm_cache();

    // Listed, recognized as an org page.
    let entry = g
        .list_pages()
        .into_iter()
        .find(|e| e.name == "Org Notes")
        .expect("org page listed");
    assert_eq!(Format::from_path(&entry.path), Format::Org);

    // Loaded: format=org, editable, headlines decomposed into blocks.
    let dto = g.load_named("Org Notes", PageKind::Page).unwrap().unwrap();
    assert_eq!(dto.format, Format::Org);
    assert!(!dto.read_only);
    assert_eq!(dto.blocks.len(), 2);
    assert_eq!(
        dto.blocks[0].raw,
        "TODO Buy milk\nSCHEDULED: <2026-06-25 Thu>"
    );
    assert_eq!(dto.blocks[1].raw, "second block");

    // No-op save leaves the file byte-identical (no churn).
    let rev = g.save_page(&dto, dto.rev.as_deref()).unwrap();
    assert_eq!(
        fs::read_to_string(dir.join("pages").join("Org Notes.org")).unwrap(),
        src
    );

    // Edit a block and save → file updated, still org, byte-faithful.
    let mut edited = dto.clone();
    edited.blocks[1].raw = "second block edited".into();
    g.save_page(&edited, Some(&rev)).unwrap();
    let on_disk = fs::read_to_string(dir.join("pages").join("Org Notes.org")).unwrap();
    assert_eq!(
        on_disk,
        "* TODO Buy milk\nSCHEDULED: <2026-06-25 Thu>\n* second block edited\n"
    );
    // No stray .md twin was created.
    assert!(!dir.join("pages").join("Org Notes.md").exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn guide_flagged_pages_are_never_written_to_graph_files() {
    let dir = scratch("guide-no-save");
    let g = Graph::open(&dir);
    let page = PageDto {
        activation: None,
        name: "Tine-guide/Features/Sheets".into(),
        kind: PageKind::Page,
        title: "Features/Sheets".into(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: "guide-block".into(),
            raw: "This is an ephemeral guide block".into(),
            collapsed: false,
            ..Default::default()
        }],
        rev: None,
        format: Format::Md,
        read_only: true,
        path: String::new(),
        guide: true,
    };

    assert_eq!(g.save_page(&page, None).unwrap(), "guide-ephemeral");
    assert_eq!(g.force_save_page(&page).unwrap(), "guide-ephemeral");
    let files: Vec<_> = fs::read_dir(dir.join("pages"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        files.is_empty(),
        "guide save guard must be load-bearing; wrote files: {files:?}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn pinned_missing_paths_never_gain_creation_authority() {
    for (scope, relative_path) in [
        ("root", "Pinned.md"),
        ("external", "external/deep/Pinned.md"),
        ("configured", "pages/deep/Pinned.md"),
    ] {
        for forced in [false, true] {
            let dir = scratch(&format!("pinned-missing-{scope}-{forced}"));
            let graph = Graph::open(&dir);
            graph.warm_cache();
            let generation = graph.cache_generation();
            let disk_revs = graph.disk_revs.read().unwrap().clone();
            let loaded_identities = graph.loaded_file_identities.read().unwrap().clone();
            let failures = graph.page_index_failures();
            let target = dir.join(relative_path);
            let parent_existed = target.parent().unwrap().exists();
            let mut page =
                markdown_page_dto("Pinned Missing", "Pinned Missing", "- must not exist\n")
                    .unwrap();
            page.path = relative_path.to_owned();

            let error = if forced {
                graph.force_save_page(&page).unwrap_err()
            } else {
                graph.save_page(&page, None).unwrap_err()
            };

            assert!(
                matches!(
                    error.kind(),
                    io::ErrorKind::NotFound
                        | io::ErrorKind::AlreadyExists
                        | io::ErrorKind::PermissionDenied
                ),
                "{scope} {forced}: {error}"
            );
            assert!(!target.exists(), "{scope} {forced} created a pinned file");
            if !parent_existed {
                assert!(
                    !target.parent().unwrap().exists(),
                    "{scope} {forced} created a pinned parent directory"
                );
            }
            assert_eq!(graph.cache_generation(), generation);
            assert_eq!(*graph.disk_revs.read().unwrap(), disk_revs);
            assert_eq!(
                *graph.loaded_file_identities.read().unwrap(),
                loaded_identities
            );
            assert_eq!(graph.page_index_failures(), failures);
            assert!(graph.recent_writes.lock().unwrap().is_empty());
            let _ = fs::remove_dir_all(&dir);
        }
    }
}

#[test]
fn generation_bound_identity_validation_avoids_save_time_graph_reparse() {
    let dir = scratch("generation-bound-name-only-identity");
    for i in 0..24 {
        fs::write(
            dir.join("pages").join(format!("Unrelated {i}.md")),
            format!("- unrelated {i}\n"),
        )
        .unwrap();
    }
    let graph = Graph::open(&dir);
    graph.warm_cache();

    GRAPH_TEXT_CONTENT_READS.with(|reads| reads.set(0));
    GRAPH_TEXT_PARSE_ATTEMPTS.with(|attempts| attempts.set(0));
    GRAPH_TEXT_VALIDATION_TARGET_READS.with(|reads| reads.set(0));
    let fresh = markdown_page_dto("Fresh Indexed", "Fresh Indexed", "- fresh\n").unwrap();
    graph.save_page(&fresh, None).unwrap();
    assert!(
        graph
            .list_pages()
            .iter()
            .any(|entry| entry.name == "Fresh Indexed"),
        "the generation-retagged page inventory must contain the new page"
    );
    assert_eq!(
        GRAPH_TEXT_CONTENT_READS.with(Cell::get),
        1,
        "only the post-publication projection receipt may reread the new target"
    );
    assert_eq!(
        GRAPH_TEXT_PARSE_ATTEMPTS.with(Cell::get),
        0,
        "name-only creation must not run a per-document parse pass"
    );
    assert_eq!(
        GRAPH_TEXT_VALIDATION_TARGET_READS.with(Cell::get),
        0,
        "name-only validation must use the effective-identity index"
    );

    let mut exact = graph.load_by_path("pages/Unrelated 0.md").unwrap().unwrap();
    exact.blocks[0].raw = "exact saved".into();
    GRAPH_TEXT_CONTENT_READS.with(|reads| reads.set(0));
    GRAPH_TEXT_PARSE_ATTEMPTS.with(|attempts| attempts.set(0));
    GRAPH_TEXT_VALIDATION_TARGET_READS.with(|reads| reads.set(0));
    graph.save_page(&exact, exact.rev.as_deref()).unwrap();
    assert_eq!(
            GRAPH_TEXT_CONTENT_READS.with(Cell::get),
            2,
            "exact save reads initial validation and the final receipt; the atomic retirement validates the baseline without a pre-retirement reread"
        );
    assert_eq!(
        GRAPH_TEXT_PARSE_ATTEMPTS.with(Cell::get),
        1,
        "exact-owner validation parses only its captured target"
    );
    assert_eq!(GRAPH_TEXT_VALIDATION_TARGET_READS.with(Cell::get), 1);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn warm_page_inventory_survives_delete_and_watcher_lifecycle_without_graph_reread() {
    let dir = scratch("warm-page-inventory-lifecycle");
    for index in 0..24 {
        fs::write(
            dir.join("pages").join(format!("Unrelated {index}.md")),
            format!("- unrelated {index}\n"),
        )
        .unwrap();
    }
    fs::write(dir.join("pages/Delete Me.md"), "- delete me\n").unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    assert_eq!(graph.list_pages().len(), 25);

    graph.delete_page("Delete Me", PageKind::Page).unwrap();
    GRAPH_TEXT_CONTENT_READS.with(|reads| reads.set(0));
    GRAPH_TEXT_PARSE_ATTEMPTS.with(|attempts| attempts.set(0));
    let after_delete = graph.list_pages();
    assert_eq!(after_delete.len(), 24);
    assert!(!after_delete.iter().any(|entry| entry.name == "Delete Me"));
    assert_eq!(GRAPH_TEXT_CONTENT_READS.with(Cell::get), 0);
    assert_eq!(GRAPH_TEXT_PARSE_ATTEMPTS.with(Cell::get), 0);

    let watched = dir.join("pages/Watched.md");
    fs::write(&watched, "title:: Watched Identity\n\n- watched\n").unwrap();
    graph.sync_file_checked(&watched).unwrap();
    GRAPH_TEXT_CONTENT_READS.with(|reads| reads.set(0));
    GRAPH_TEXT_PARSE_ATTEMPTS.with(|attempts| attempts.set(0));
    let after_create = graph.list_pages();
    assert_eq!(after_create.len(), 25);
    assert!(after_create
        .iter()
        .any(|entry| entry.name == "Watched Identity" && entry.path == watched));
    assert_eq!(GRAPH_TEXT_CONTENT_READS.with(Cell::get), 0);
    assert_eq!(GRAPH_TEXT_PARSE_ATTEMPTS.with(Cell::get), 0);

    fs::remove_file(&watched).unwrap();
    graph.sync_deleted_file(&watched).unwrap();
    GRAPH_TEXT_CONTENT_READS.with(|reads| reads.set(0));
    GRAPH_TEXT_PARSE_ATTEMPTS.with(|attempts| attempts.set(0));
    let after_remove = graph.list_pages();
    assert_eq!(after_remove.len(), 24);
    assert!(!after_remove.iter().any(|entry| entry.path == watched));
    assert_eq!(GRAPH_TEXT_CONTENT_READS.with(Cell::get), 0);
    assert_eq!(GRAPH_TEXT_PARSE_ATTEMPTS.with(Cell::get), 0);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn warm_page_inventory_survives_rename_without_graph_reread_or_reparse() {
    let dir = scratch("warm-page-inventory-rename");
    for index in 0..24 {
        fs::write(
            dir.join("pages").join(format!("Unrelated {index}.md")),
            format!("- unrelated {index}\n"),
        )
        .unwrap();
    }
    fs::write(
        dir.join("pages/Original.md"),
        "title:: Original\n\n- [[Original]]\n",
    )
    .unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    assert_eq!(graph.list_pages().len(), 25);

    graph.rename_page("Original", "Renamed").unwrap();
    GRAPH_TEXT_CONTENT_READS.with(|reads| reads.set(0));
    GRAPH_TEXT_PARSE_ATTEMPTS.with(|attempts| attempts.set(0));
    let after_rename = graph.list_pages();
    assert_eq!(after_rename.len(), 25);
    assert!(!after_rename
        .iter()
        .any(|entry| entry.rel_path == "pages/Original.md"));
    assert!(after_rename.iter().any(|entry| {
        // A `title::` that named the page being renamed is rebound by the
        // rename itself (GH #451), so the effective identity moves with the
        // file instead of stranding the page under its old name. A title that
        // named something else is user content and is left alone; that is
        // `gh451_research.rs`.
        entry.name == "Renamed" && entry.rel_path == "pages/Renamed.md"
    }));
    assert!(fs::read_to_string(dir.join("pages/Renamed.md"))
        .unwrap()
        .starts_with("title:: Renamed\n"));
    assert_eq!(GRAPH_TEXT_CONTENT_READS.with(Cell::get), 0);
    assert_eq!(GRAPH_TEXT_PARSE_ATTEMPTS.with(Cell::get), 0);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn watcher_parse_failure_cannot_republish_stale_warm_page_inventory() {
    let dir = scratch("watcher-failure-page-inventory");
    fs::write(dir.join("pages/Good.md"), "- good\n").unwrap();
    let failed = dir.join("pages/Failed.md");
    fs::write(&failed, "- initially valid\n").unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    assert_eq!(graph.list_pages().len(), 2);

    fs::write(&failed, [0xff, 0xfe, b'\n']).unwrap();
    assert!(graph.sync_file_checked(&failed).is_err());
    let mut good = graph.load_by_path("pages/Good.md").unwrap().unwrap();
    good.blocks[0].raw = "saved while sibling failed".into();
    graph.save_page(&good, good.rev.as_deref()).unwrap();

    GRAPH_TEXT_CONTENT_READS.with(|reads| reads.set(0));
    let inventory = graph.list_pages();
    assert_eq!(inventory.len(), 1);
    assert_eq!(inventory[0].name, "Good");
    // GH #543 IT-07: the known failure is re-read from disk; the healthy
    // sibling is not, so a single unreadable page costs one read per listing
    // instead of a whole-graph reparse.
    assert_eq!(
        GRAPH_TEXT_CONTENT_READS.with(Cell::get),
        1,
        "a known watcher parse failure must revalidate exactly the failed path"
    );

    // Repaired and delivered by the watcher: the page is listed again.
    fs::write(&failed, "- repaired\n").unwrap();
    graph.sync_file(&failed);
    let mut names = graph
        .list_pages()
        .into_iter()
        .map(|entry| entry.name)
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(names, vec!["Failed".to_owned(), "Good".to_owned()]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn changed_existing_save_has_one_portable_traversal_and_no_graph_capture() {
    let dir = scratch("existing-save-portable-traversal-count");
    for index in 0..24 {
        fs::write(
            dir.join("pages").join(format!("Unrelated {index}.md")),
            format!("- unrelated {index}\n"),
        )
        .unwrap();
    }
    fs::write(dir.join("pages/Target.md"), b"- before\n").unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let mut page = graph.load_by_path("pages/Target.md").unwrap().unwrap();
    page.blocks[0].raw = "after".into();

    reset_graph_text_admission_test_counters();
    GRAPH_TEXT_PORTABLE_TRAVERSALS.with(|count| count.set(0));
    GRAPH_TEXT_CONTENT_READS.with(|reads| reads.set(0));
    GRAPH_TEXT_PARSE_ATTEMPTS.with(|attempts| attempts.set(0));
    GRAPH_TEXT_VALIDATION_TARGET_READS.with(|reads| reads.set(0));
    let builds_before = graph.guarded_graph_text_identity_report().complete_builds;

    graph.save_page(&page, page.rev.as_deref()).unwrap();

    assert_eq!(GRAPH_TEXT_PORTABLE_TRAVERSALS.with(Cell::get), 1);
    assert_eq!(
        graph_text_admission_test_counters().builder_enumerations,
        0,
        "existing save must not enter complete graph capture"
    );
    assert_eq!(
        graph.guarded_graph_text_identity_report().complete_builds,
        builds_before
    );
    assert_eq!(
        GRAPH_TEXT_PARSE_ATTEMPTS.with(Cell::get),
        1,
        "only the exact target may be parsed"
    );
    assert_eq!(GRAPH_TEXT_VALIDATION_TARGET_READS.with(Cell::get), 1);
    assert_eq!(
            GRAPH_TEXT_CONTENT_READS.with(Cell::get),
            2,
            "only exact validation and the target receipt may read content; retirement itself validates the baseline"
        );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn portable_prefix_branching_limit_fails_before_mutation() {
    let dir = scratch("portable-prefix-branch-limit");
    for ancestor in ["External", "external", "EXTERNAL"] {
        fs::create_dir_all(dir.join(ancestor)).unwrap();
    }
    fs::write(dir.join("External/Target.md"), b"- before\n").unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let mut page = graph.load_by_path("External/Target.md").unwrap().unwrap();
    page.blocks[0].raw = "must not publish".into();

    GRAPH_TEXT_INVENTORY_LIMITS_OVERRIDE.with(|override_limits| {
        *override_limits.borrow_mut() = Some(GraphTextInventoryLimits {
            directories: 2,
            ..GRAPH_TEXT_INVENTORY_LIMITS
        });
    });
    let error = graph.save_page(&page, page.rev.as_deref()).unwrap_err();
    GRAPH_TEXT_INVENTORY_LIMITS_OVERRIDE.with(|override_limits| {
        *override_limits.borrow_mut() = None;
    });

    assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{error}");
    assert_eq!(
        fs::read_to_string(dir.join("External/Target.md")).unwrap(),
        "- before\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

fn reset_page_build_test_counters(graph: &Graph) {
    graph
        .page_build_test
        .enumerations
        .store(0, std::sync::atomic::Ordering::Relaxed);
    graph
        .page_build_test
        .parses
        .store(0, std::sync::atomic::Ordering::Relaxed);
    graph
        .page_build_test
        .installs
        .store(0, std::sync::atomic::Ordering::Relaxed);
    graph
        .page_build_test
        .censuses
        .store(0, std::sync::atomic::Ordering::Relaxed);
    *graph.page_build_test.joined.lock().unwrap() = 0;
}

fn wait_for_page_build_join(graph: &Graph) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut joined = graph.page_build_test.joined.lock().unwrap();
    while *joined == 0 {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .expect("creator did not join the active page build");
        let (next, timeout) = graph
            .page_build_test
            .joined_changed
            .wait_timeout(joined, remaining)
            .unwrap();
        joined = next;
        assert!(
            !timeout.timed_out(),
            "creator did not join the active page build"
        );
    }
}

fn wait_for_identity_mutation_waiter(graph: &Graph) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let gate = &graph
        .graph_text_write_binding()
        .expect("test graph has graph writer binding")
        .gate;
    let mut state = gate.identity_mutation.lock().unwrap();
    while state.waiters == 0 {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .expect("second creator did not reach resource identity serialization");
        let (next, timeout) = gate
            .identity_mutation_changed
            .wait_timeout(state, remaining)
            .unwrap();
        state = next;
        assert!(
            !timeout.timed_out(),
            "second creator did not reach resource identity serialization"
        );
    }
}

#[test]
fn cold_graph_creation_repairs_identity_evidence_once() {
    let dir = scratch("cold-generation-creation-repair");
    for index in 0..4 {
        fs::write(
            dir.join("pages").join(format!("Existing {index}.md")),
            format!("title:: Existing {index}\n\n- body\n"),
        )
        .unwrap();
    }
    let graph = Graph::open(&dir);
    reset_page_build_test_counters(&graph);

    let page = markdown_page_dto("Cold Created", "Cold Created", "- body\n").unwrap();
    graph.save_page(&page, None).unwrap();

    assert!(dir.join("pages/Cold Created.md").is_file());
    assert_eq!(
        graph
            .page_build_test
            .enumerations
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    assert_eq!(
        graph
            .page_build_test
            .parses
            .load(std::sync::atomic::Ordering::Relaxed),
        4,
        "one generation build parses each existing document once"
    );
    assert_eq!(
        graph
            .page_build_test
            .censuses
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    assert!(graph.cache.read().unwrap().is_some());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn empty_cold_graph_uses_the_bounded_build_before_creation() {
    let dir = scratch("cold-empty-effective-identity");
    let graph = Graph::open(&dir);
    reset_page_build_test_counters(&graph);
    let cold = markdown_page_dto("Cold Created", "Cold Created", "- body\n").unwrap();

    graph.save_page(&cold, None).unwrap();

    assert!(dir.join("pages/Cold Created.md").is_file());
    assert_eq!(
        graph
            .page_build_test
            .enumerations
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    assert_eq!(
        graph
            .page_build_test
            .parses
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    assert_eq!(
        graph
            .page_build_test
            .censuses
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn direct_cold_creation_joins_a_paused_paced_warm() {
    let dir = scratch("cold-creation-joins-warm");
    for index in 0..3 {
        fs::write(
            dir.join("pages").join(format!("Existing {index}.md")),
            format!("- body {index}\n"),
        )
        .unwrap();
    }
    let graph = Arc::new(Graph::open(&dir));
    reset_page_build_test_counters(&graph);
    let pause = Arc::new(PageBuildTestPause::new());
    *graph.page_build_test.owner_pause.lock().unwrap() = Some(Arc::clone(&pause));
    let warm_graph = Arc::clone(&graph);
    let warm = std::thread::spawn(move || warm_graph.warm_page_cache_cancellable(&|| false));
    pause.reached.wait();

    let creator_graph = Arc::clone(&graph);
    let creator = std::thread::spawn(move || {
        creator_graph.save_page(
            &markdown_page_dto("Joined Creator", "Joined Creator", "- body\n").unwrap(),
            None,
        )
    });
    wait_for_page_build_join(&graph);
    pause.release.wait();

    assert!(warm.join().unwrap());
    creator.join().unwrap().unwrap();
    assert!(dir.join("pages/Joined Creator.md").is_file());
    assert_eq!(
        graph
            .page_build_test
            .enumerations
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    assert_eq!(
        graph
            .page_build_test
            .parses
            .load(std::sync::atomic::Ordering::Relaxed),
        3
    );
    assert_eq!(
        graph
            .page_build_test
            .censuses
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn concurrent_direct_creation_proofs_join_one_build_without_graph_censuses() {
    let dir = scratch("concurrent-direct-creation-proofs");
    fs::write(dir.join("pages/Existing.md"), "- existing\n").unwrap();
    let graph = Arc::new(Graph::open(&dir));
    reset_page_build_test_counters(&graph);
    let pause = Arc::new(PageBuildTestPause::new());
    *graph.page_build_test.owner_pause.lock().unwrap() = Some(Arc::clone(&pause));

    let first_graph = Arc::clone(&graph);
    let first = std::thread::spawn(move || {
        let permit = first_graph.admit_retained_graph_text_writer()?;
        first_graph.direct_creation_proof(
            &permit,
            &first_graph.root.join("pages/First Proof.md"),
            PageKind::Page,
            "First Proof",
        )
    });
    pause.reached.wait();
    let second_graph = Arc::clone(&graph);
    let second = std::thread::spawn(move || {
        let permit = second_graph.admit_retained_graph_text_writer()?;
        second_graph.direct_creation_proof(
            &permit,
            &second_graph.root.join("pages/Second Proof.md"),
            PageKind::Page,
            "Second Proof",
        )
    });
    wait_for_page_build_join(&graph);
    pause.release.wait();

    let (first_proof, first_owned_elsewhere) = first.join().unwrap().unwrap();
    let (second_proof, second_owned_elsewhere) = second.join().unwrap().unwrap();
    assert!(!first_owned_elsewhere);
    assert!(!second_owned_elsewhere);
    assert_eq!(first_proof.generation, second_proof.generation);
    assert_ne!(first_proof.target, second_proof.target);
    assert_eq!(*graph.page_build_test.joined.lock().unwrap(), 1);
    assert_eq!(
        graph
            .page_build_test
            .enumerations
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    assert_eq!(
        graph
            .page_build_test
            .parses
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    assert_eq!(
        graph
            .page_build_test
            .installs
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    assert_eq!(
        graph
            .page_build_test
            .censuses
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn racing_save_page_creators_serialize_and_reuse_one_published_build() {
    let dir = scratch("racing-cold-creators");
    fs::write(dir.join("pages/Existing.md"), "- existing\n").unwrap();
    let graph = Arc::new(Graph::open(&dir));
    reset_page_build_test_counters(&graph);
    let pause = Arc::new(PageBuildTestPause::new());
    *graph.page_build_test.owner_pause.lock().unwrap() = Some(Arc::clone(&pause));

    let first_graph = Arc::clone(&graph);
    let first = std::thread::spawn(move || {
        first_graph.save_page(
            &markdown_page_dto("First Racer", "First Racer", "- first\n").unwrap(),
            None,
        )
    });
    pause.reached.wait();
    let second_graph = Arc::clone(&graph);
    let second = std::thread::spawn(move || {
        second_graph.save_page(
            &markdown_page_dto("Second Racer", "Second Racer", "- second\n").unwrap(),
            None,
        )
    });
    wait_for_identity_mutation_waiter(&graph);
    assert_eq!(
        *graph.page_build_test.joined.lock().unwrap(),
        0,
        "save_page serializes creators before either can join the other's flight"
    );
    pause.release.wait();

    first.join().unwrap().unwrap();
    second.join().unwrap().unwrap();
    assert!(dir.join("pages/First Racer.md").is_file());
    assert!(dir.join("pages/Second Racer.md").is_file());
    assert_eq!(
        graph
            .page_build_test
            .enumerations
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    assert_eq!(
        graph
            .page_build_test
            .parses
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    assert_eq!(
        graph
            .page_build_test
            .installs
            .load(std::sync::atomic::Ordering::Relaxed),
        1,
        "serialized creators reuse one published cold-cache build"
    );
    assert_eq!(
        graph
            .page_build_test
            .censuses
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    assert_eq!(*graph.page_build_test.joined.lock().unwrap(), 0);
    assert_eq!(
        graph
            .effective_identity_index
            .read()
            .unwrap()
            .as_ref()
            .expect("serialized saves retain published identity evidence")
            .generation(),
        graph.cache_generation()
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn late_page_build_claim_after_completed_install_is_non_owner() {
    let dir = scratch("late-page-build-claim");
    fs::write(dir.join("pages/Existing.md"), "- existing\n").unwrap();
    let graph = Graph::open(&dir);
    reset_page_build_test_counters(&graph);
    assert!(matches!(
        graph.direct_creation_evidence().unwrap(),
        DirectCreationEvidence::Cold
    ));
    let expected_generation = graph.cache_generation();
    let permit = graph.admit_retained_graph_text_writer().unwrap();

    assert_eq!(
        graph.repair_page_cache_once(&permit),
        PageBuildOutcome::Installed
    );
    assert!(graph.page_build_flight.lock().unwrap().is_none());
    let before = (
        graph
            .page_build_test
            .enumerations
            .load(std::sync::atomic::Ordering::Relaxed),
        graph
            .page_build_test
            .parses
            .load(std::sync::atomic::Ordering::Relaxed),
        graph
            .page_build_test
            .installs
            .load(std::sync::atomic::Ordering::Relaxed),
    );

    // This expected generation was captured with the earlier cold evidence,
    // but the actual claim happens only after the first flight has finished.
    let (late_flight, owner) = graph.claim_page_build(expected_generation);

    assert!(!owner);
    assert_eq!(late_flight.wait(), PageBuildOutcome::AlreadyAvailable);
    assert!(graph.page_build_flight.lock().unwrap().is_none());
    graph.invalidate_cache_after_tine_mutation();
    let (drifted_flight, owner) = graph.claim_page_build(expected_generation);
    assert!(!owner);
    assert_eq!(drifted_flight.wait(), PageBuildOutcome::GenerationDrift);
    assert!(graph.page_build_flight.lock().unwrap().is_none());
    assert_eq!(
        (
            graph
                .page_build_test
                .enumerations
                .load(std::sync::atomic::Ordering::Relaxed),
            graph
                .page_build_test
                .parses
                .load(std::sync::atomic::Ordering::Relaxed),
            graph
                .page_build_test
                .installs
                .load(std::sync::atomic::Ordering::Relaxed),
        ),
        before,
        "a late completed claim must not enumerate, parse, or install again"
    );
    assert_eq!(before, (1, 1, 1));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn generation_drift_before_direct_install_refuses_without_retry_or_census() {
    let dir = scratch("cold-creation-install-drift");
    fs::write(dir.join("pages/Existing.md"), "- existing\n").unwrap();
    let graph = Graph::open(&dir);
    reset_page_build_test_counters(&graph);
    graph
        .page_build_test
        .drift_before_install
        .store(true, std::sync::atomic::Ordering::Release);
    let target = dir.join("pages/Drift Refused.md");

    let error = graph
        .save_page(
            &markdown_page_dto("Drift Refused", "Drift Refused", "- no\n").unwrap(),
            None,
        )
        .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::Interrupted, "{error}");
    assert!(!target.exists());
    assert!(graph.cache.read().unwrap().is_none());
    assert!(graph.cache_index.read().unwrap().is_none());
    assert!(graph.effective_identity_index.read().unwrap().is_none());
    assert!(graph.disk_revs.read().unwrap().is_empty());
    assert_eq!(
        graph
            .page_build_test
            .enumerations
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    assert_eq!(
        graph
            .page_build_test
            .parses
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    assert_eq!(
        graph
            .page_build_test
            .censuses
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn failed_or_cancelled_warm_flight_wakes_creator_without_takeover() {
    for failed in [false, true] {
        let dir = scratch(if failed {
            "joined-failed-warm"
        } else {
            "joined-cancelled-warm"
        });
        fs::write(dir.join("pages/Existing.md"), "- existing\n").unwrap();
        let graph = Arc::new(Graph::open(&dir));
        reset_page_build_test_counters(&graph);
        let pause = Arc::new(PageBuildTestPause::new());
        *graph.page_build_test.owner_pause.lock().unwrap() = Some(Arc::clone(&pause));
        graph
            .page_build_test
            .force_warm_failure
            .store(failed, std::sync::atomic::Ordering::Release);
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let warm_graph = Arc::clone(&graph);
        let warm_cancelled = Arc::clone(&cancelled);
        let warm = std::thread::spawn(move || {
            warm_graph.warm_page_cache_cancellable(&|| {
                warm_cancelled.load(std::sync::atomic::Ordering::Acquire)
            })
        });
        pause.reached.wait();
        let creator_graph = Arc::clone(&graph);
        let creator = std::thread::spawn(move || {
            creator_graph.save_page(
                &markdown_page_dto("No Takeover", "No Takeover", "- no\n").unwrap(),
                None,
            )
        });
        wait_for_page_build_join(&graph);
        if !failed {
            cancelled.store(true, std::sync::atomic::Ordering::Release);
        }
        pause.release.wait();

        assert!(!warm.join().unwrap());
        let error = creator.join().unwrap().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted, "{error}");
        assert!(!dir.join("pages/No Takeover.md").exists());
        assert!(graph.cache.read().unwrap().is_none());
        assert_eq!(
            graph
                .page_build_test
                .enumerations
                .load(std::sync::atomic::Ordering::Relaxed),
            usize::from(failed)
        );
        assert_eq!(
            graph
                .page_build_test
                .censuses
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        let _ = fs::remove_dir_all(&dir);
    }
}

#[test]
fn install_built_publishes_only_at_its_exact_generation() {
    for drift in [false, true] {
        let dir = scratch(if drift {
            "install-boundary-drift"
        } else {
            "install-boundary-exact"
        });
        fs::write(dir.join("pages/Existing.md"), "- existing\n").unwrap();
        let graph = Graph::open(&dir);
        let permit = graph.admit_retained_graph_text_writer().unwrap();
        let expected = graph.cache_generation();
        let built = graph.load_all_pages_with_permit(&permit);
        let flight = PageBuildFlight::new(
            expected,
            graph
                .cache_structural_gen
                .load(std::sync::atomic::Ordering::Acquire),
        );
        if drift {
            // A structural move (removal or invalidation) is real drift.
            graph
                .cache_gen
                .fetch_add(1, std::sync::atomic::Ordering::Release);
            graph
                .cache_structural_gen
                .fetch_add(1, std::sync::atomic::Ordering::Release);
        }

        let outcome = graph.install_built(&flight, built);

        if drift {
            assert_eq!(outcome, PageCacheInstallOutcome::GenerationDrift);
            assert!(graph.cache.read().unwrap().is_none());
            assert!(graph.cache_index.read().unwrap().is_none());
            assert!(graph.disk_revs.read().unwrap().is_empty());
            assert!(graph.effective_identity_index.read().unwrap().is_none());
        } else {
            assert_eq!(outcome, PageCacheInstallOutcome::Installed);
            assert!(graph.cache.read().unwrap().is_some());
            assert!(graph.cache_index.read().unwrap().is_some());
            assert_eq!(graph.disk_revs.read().unwrap().len(), 1);
            let identity = graph
                .effective_identity_index
                .read()
                .unwrap()
                .as_ref()
                .cloned()
                .unwrap();
            assert_eq!(identity.generation(), expected);
        }
        let _ = fs::remove_dir_all(&dir);
    }
}

#[test]
fn failed_and_stale_effective_identity_evidence_blocks_name_only_creation() {
    // A failed identity can hide any effective title, so it blocks only the
    // ambiguous name-only creation path. An unrelated exact owner remains
    // writable through its retained bytes/revision/file identity.
    let dir = scratch("failed-effective-identity");
    fs::write(dir.join("pages/Good.md"), "- good\n").unwrap();
    fs::write(dir.join("pages/Invalid.md"), [0xff, 0xfe, b'\n']).unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let mut good = graph.load_by_path("pages/Good.md").unwrap().unwrap();
    good.blocks[0].raw = "good exact save".into();
    graph.save_page(&good, good.rev.as_deref()).unwrap();
    assert_eq!(
        fs::read_to_string(dir.join("pages/Good.md")).unwrap(),
        "- good exact save\n"
    );
    let fresh = markdown_page_dto("Could Be Hidden", "Could Be Hidden", "- no\n").unwrap();
    assert_eq!(
        graph.save_page(&fresh, None).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert!(!dir.join("pages/Could Be Hidden.md").exists());
    let _ = fs::remove_dir_all(&dir);

    // A graph file arriving after the indexed generation has unknown
    // effective identity until reconciliation advances/rebuilds the cache.
    let dir = scratch("stale-effective-identity");
    fs::write(dir.join("pages/Indexed.md"), "- indexed\n").unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    fs::create_dir_all(dir.join("external")).unwrap();
    fs::write(
        dir.join("external/Late.md"),
        "title:: Could Be Hidden\n\n- late\n",
    )
    .unwrap();
    let late = dir.join("external/Late.md");
    graph.note_graph_text_external_observation();
    let observed = graph.graph_text_external_observation_ticket();
    graph.sync_file_checked(&late).unwrap();
    graph.acknowledge_graph_text_external_observations(observed);
    GRAPH_TEXT_CONTENT_READS.with(|reads| reads.set(0));
    GRAPH_TEXT_PARSE_ATTEMPTS.with(|attempts| attempts.set(0));
    let fresh = markdown_page_dto("Could Be Hidden", "Could Be Hidden", "- no\n").unwrap();
    assert_eq!(
        graph.save_page(&fresh, None).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert!(!dir.join("pages/Could Be Hidden.md").exists());
    assert_eq!(GRAPH_TEXT_CONTENT_READS.with(Cell::get), 0);
    assert_eq!(GRAPH_TEXT_PARSE_ATTEMPTS.with(Cell::get), 0);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn broad_invalidation_rebuilds_current_titles_once_before_creation() {
    let dir = scratch("broad-invalidation-current-title");
    let owner = dir.join("pages/Physical Owner.md");
    fs::write(&owner, "title:: Old Identity\n\n- owner\n").unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    reset_page_build_test_counters(&graph);
    fs::write(&owner, "title:: Current Identity\n\n- owner\n").unwrap();
    graph.invalidate_cache();

    let collision = graph
        .save_page(
            &markdown_page_dto("Current Identity", "Current Identity", "- no\n").unwrap(),
            None,
        )
        .unwrap_err();
    assert_eq!(
        collision.kind(),
        io::ErrorKind::AlreadyExists,
        "{collision}"
    );
    assert!(!dir.join("pages/Current Identity.md").exists());
    assert_eq!(
        fs::read_to_string(&owner).unwrap(),
        "title:: Current Identity\n\n- owner\n"
    );
    assert_eq!(
        graph
            .page_build_test
            .enumerations
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    assert_eq!(
        graph
            .page_build_test
            .parses
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );

    graph
        .save_page(
            &markdown_page_dto(
                "Unrelated After Repair",
                "Unrelated After Repair",
                "- yes\n",
            )
            .unwrap(),
            None,
        )
        .unwrap();
    assert!(dir.join("pages/Unrelated After Repair.md").is_file());
    assert_eq!(
        graph
            .page_build_test
            .enumerations
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    assert_eq!(
        graph
            .page_build_test
            .parses
            .load(std::sync::atomic::Ordering::Relaxed),
        1,
        "the repaired normal cache serves the later creation"
    );
    assert_eq!(
        graph
            .page_build_test
            .censuses
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn exact_cache_removal_keeps_current_effective_identity_evidence() {
    let dir = scratch("exact-removal-identity-coherence");
    let removed = dir.join("pages/Removed.md");
    fs::write(&removed, "title:: Removed Identity\n\n- before\n").unwrap();
    fs::write(dir.join("pages/Survivor.md"), "- survivor\n").unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let entry = graph
        .list_pages()
        .into_iter()
        .find(|entry| entry.path == removed)
        .unwrap();
    reset_page_build_test_counters(&graph);
    fs::remove_file(&removed).unwrap();

    graph.cache_remove_path(&entry);

    let generation = graph.cache_generation();
    let identity = graph
        .effective_identity_index
        .read()
        .unwrap()
        .as_ref()
        .cloned()
        .expect("exact removal keeps a warm identity index");
    assert_eq!(identity.generation(), generation);
    assert!(!identity.physical_paths.contains(&removed));
    assert!(!identity
        .owners
        .contains_key(&page_cache_key(PageKind::Page, "Removed Identity")));

    graph
        .save_page(
            &markdown_page_dto("Removed Identity", "Removed Identity", "- recreated\n").unwrap(),
            None,
        )
        .unwrap();
    assert!(dir.join("pages/Removed Identity.md").is_file());
    assert_eq!(
        graph
            .page_build_test
            .enumerations
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    assert_eq!(
        graph
            .page_build_test
            .parses
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    assert_eq!(
        graph
            .page_build_test
            .censuses
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn cold_malformed_owner_refuses_without_census_or_mutation() {
    let dir = scratch("cold-malformed-owner");
    let malformed = dir.join("pages/Malformed.md");
    fs::write(&malformed, [0xff, 0xfe, b'\n']).unwrap();
    let graph = Graph::open(&dir);
    reset_page_build_test_counters(&graph);
    let target = dir.join("pages/Must Not Exist.md");

    let error = graph
        .save_page(
            &markdown_page_dto("Must Not Exist", "Must Not Exist", "- no\n").unwrap(),
            None,
        )
        .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{error}");
    assert_eq!(fs::read(&malformed).unwrap(), [0xff, 0xfe, b'\n']);
    assert!(!target.exists());
    assert_eq!(graph.page_index_failures(), vec!["pages/Malformed.md"]);
    assert_eq!(
        graph
            .page_build_test
            .enumerations
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    assert_eq!(
        graph
            .page_build_test
            .censuses
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn warm_install_and_watcher_failures_replace_effective_identity_evidence() {
    // Seed the permitted empty-cold index, then prove a later warm install
    // replaces it with the failure set discovered from disk.
    let dir = scratch("empty-cold-then-warm-identity-failure");
    let graph = Graph::open(&dir);
    assert!(!graph
        .validate_name_only_effective_identity(&[], PageKind::Page, "unused")
        .unwrap());
    let invalid = dir.join("pages/Invalid.md");
    fs::write(&invalid, [0xff, 0xfe, b'\n']).unwrap();
    graph.warm_cache();
    assert_eq!(graph.page_index_failures(), vec!["pages/Invalid.md"]);
    let blocked = markdown_page_dto("Blocked Warm", "Blocked Warm", "- no\n").unwrap();
    assert_eq!(
        graph.save_page(&blocked, None).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert!(!dir.join("pages/Blocked Warm.md").exists());
    let installed = graph
        .effective_identity_index
        .read()
        .unwrap()
        .as_ref()
        .cloned()
        .unwrap();
    assert_eq!(installed.generation(), graph.cache_generation());
    assert_eq!(installed.failures, vec!["pages/Invalid.md"]);
    let _ = fs::remove_dir_all(&dir);

    // Invalid UTF-8 and parser rejection both become same-generation,
    // per-path failure evidence. A successful watcher reconciliation first
    // installs the repaired cache/identity state and only then clears it.
    for (case, rejected) in [
        ("utf8", vec![0xff, 0xfe, b'\n']),
        (
            "parser",
            format!("- {TEST_PAGE_PARSE_PANIC_SENTINEL}\n").into_bytes(),
        ),
    ] {
        let dir = scratch(&format!("watcher-effective-failure-{case}"));
        let path = dir.join("pages/Mutable.md");
        fs::write(&path, b"- before\n").unwrap();
        let graph = Graph::open(&dir);
        graph.warm_cache();
        fs::write(&path, rejected).unwrap();
        assert!(graph.sync_file_checked(&path).is_err());
        assert_eq!(graph.page_index_failures(), vec!["pages/Mutable.md"]);
        let failed = graph
            .effective_identity_index
            .read()
            .unwrap()
            .as_ref()
            .cloned()
            .unwrap();
        assert_eq!(failed.generation(), graph.cache_generation());
        assert_eq!(failed.failures, vec!["pages/Mutable.md"]);
        let blocked = markdown_page_dto("Blocked Watcher", "Blocked Watcher", "- no\n").unwrap();
        let error = graph.save_page(&blocked, None).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{error}");

        fs::write(&path, b"title:: Repaired Identity\n\n- after\n").unwrap();
        graph.sync_file_checked(&path).unwrap();
        assert!(graph.page_index_failures().is_empty());
        let repaired = graph
            .effective_identity_index
            .read()
            .unwrap()
            .as_ref()
            .cloned()
            .unwrap();
        assert_eq!(repaired.generation(), graph.cache_generation());
        assert!(repaired.failures.is_empty());
        assert!(repaired
            .owners
            .contains_key(&page_cache_key(PageKind::Page, "Repaired Identity")));
        let allowed =
            markdown_page_dto("Allowed After Repair", "Allowed After Repair", "- yes\n").unwrap();
        graph.save_page(&allowed, None).unwrap();
        assert!(dir.join("pages/Allowed After Repair.md").is_file());
        let _ = fs::remove_dir_all(&dir);
    }
}

#[cfg(unix)]
#[test]
fn watcher_missing_and_changed_identity_record_failure_before_return() {
    for case in ["missing", "changed-identity"] {
        let dir = scratch(&format!("watcher-snapshot-{case}"));
        let path = dir.join("pages/Mutable.md");
        fs::write(&path, b"- before\n").unwrap();
        let graph = Graph::open(&dir);
        graph.warm_cache();

        BOUNDED_READ_AFTER_METADATA.with(|hook| {
            let path = path.clone();
            *hook.borrow_mut() = Some(Box::new(move || {
                if case == "missing" {
                    fs::remove_file(path)
                } else {
                    let replacement = path.with_extension("replacement");
                    fs::write(&replacement, b"- replacement\n")?;
                    fs::rename(replacement, path)
                }
            }));
        });
        let result = graph.sync_file_checked(&path);
        if case == "missing" {
            assert!(result.unwrap().is_none());
        } else {
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
        }
        assert_eq!(graph.page_index_failures(), vec!["pages/Mutable.md"]);
        let failed = graph
            .effective_identity_index
            .read()
            .unwrap()
            .as_ref()
            .cloned()
            .unwrap();
        assert_eq!(failed.generation(), graph.cache_generation());
        assert_eq!(failed.failures, vec!["pages/Mutable.md"]);
        let blocked = markdown_page_dto("Blocked Snapshot", "Blocked Snapshot", "- no\n").unwrap();
        assert_eq!(
            graph.save_page(&blocked, None).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );

        fs::write(&path, b"- repaired\n").unwrap();
        graph.sync_file_checked(&path).unwrap();
        assert!(graph.page_index_failures().is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}

#[test]
fn cold_watcher_failures_publish_generation_bound_identity_evidence_until_repair() {
    for case in ["missing", "invalid-utf8"] {
        let dir = scratch(&format!("cold-watcher-effective-failure-{case}"));
        let path = dir.join("pages/Cold Failure.md");
        fs::write(&path, b"- before\n").unwrap();
        let graph = Graph::open(&dir);
        assert!(graph.cache.read().unwrap().is_none());
        assert!(graph.effective_identity_index.read().unwrap().is_none());

        if case == "missing" {
            BOUNDED_READ_AFTER_METADATA.with(|hook| {
                let path = path.clone();
                *hook.borrow_mut() = Some(Box::new(move || fs::remove_file(path)));
            });
            assert!(graph.sync_file_checked(&path).unwrap().is_none());
        } else {
            fs::write(&path, [0xff, 0xfe, b'\n']).unwrap();
            assert_eq!(
                graph.sync_file_checked(&path).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }

        assert!(graph.cache.read().unwrap().is_none());
        assert_eq!(graph.page_index_failures(), vec!["pages/Cold Failure.md"]);
        let failed = graph
            .effective_identity_index
            .read()
            .unwrap()
            .as_ref()
            .cloned()
            .expect("cold watcher failure must install effective evidence");
        assert_eq!(failed.generation(), graph.cache_generation());
        assert_eq!(failed.failures, vec!["pages/Cold Failure.md"]);
        assert!(failed.physical_paths.contains(&path));

        let blocked = markdown_page_dto("Blocked Cold", "Blocked Cold", "- no\n").unwrap();
        let error = graph.save_page(&blocked, None).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{error}");
        assert!(!dir.join("pages/Blocked Cold.md").exists());

        fs::write(&path, "title:: Repaired Cold\n\n- after\n").unwrap();
        graph.sync_file_checked(&path).unwrap();
        assert!(graph.page_index_failures().is_empty());
        let repaired = graph
            .effective_identity_index
            .read()
            .unwrap()
            .as_ref()
            .cloned()
            .unwrap();
        assert_eq!(repaired.generation(), graph.cache_generation());
        assert!(repaired.failures.is_empty());
        assert!(repaired
            .owners
            .contains_key(&page_cache_key(PageKind::Page, "Repaired Cold")));
        let exact = graph
            .load_by_path("pages/Cold Failure.md")
            .unwrap()
            .expect("repaired exact owner remains available");
        assert_eq!(exact.blocks[0].raw, "after");

        let allowed = markdown_page_dto("Allowed Cold", "Allowed Cold", "- yes\n").unwrap();
        graph.save_page(&allowed, None).unwrap();
        assert!(dir.join("pages/Allowed Cold.md").is_file());
        let _ = fs::remove_dir_all(&dir);
    }
}

#[test]
fn org_journal_recognized_and_listed() {
    let dir = scratch("org-journal");
    fs::write(
        dir.join("journals").join("2026_06_24.org"),
        "* woke up\n* TODO ship\n",
    )
    .unwrap();
    let g = Graph::open(&dir);
    let j = g
        .journals_desc()
        .into_iter()
        .find(|e| e.kind == PageKind::Journal)
        .expect("org journal listed");
    assert_eq!(Format::from_path(&j.path), Format::Org);
    assert!(j.date_key.is_some(), "journal date parsed from .org stem");
    let dto = g.load_page(&j).unwrap();
    assert_eq!(dto.blocks.len(), 2);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn non_round_trip_org_is_read_only_and_save_refused() {
    let dir = scratch("org-ro");
    // Skipped heading level (`*` then `***`) cannot be reproduced from tree
    // depth → not round-trip safe → must load read-only and refuse writes.
    let src = "* a\n*** c\n";
    fs::write(dir.join("pages").join("Weird.org"), src).unwrap();
    let g = Graph::open(&dir);
    let dto = g.load_named("Weird", PageKind::Page).unwrap().unwrap();
    assert_eq!(dto.format, Format::Org);
    assert!(dto.read_only, "non-round-tripping org loads read-only");
    // Even a forced save must refuse (defense in depth) and leave bytes intact.
    let err = g.force_save_page(&dto).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(
        fs::read_to_string(dir.join("pages").join("Weird.org")).unwrap(),
        src
    );
    let _ = fs::remove_dir_all(&dir);
}

/// "Keep mine" must work on the DTO the frontend actually sends.
///
/// `pageToDto` (`src/store.ts`) builds every saved page without a `rev`
/// field — ordinary saves carry their load revision in the separate
/// `base_rev` argument instead. Every Rust test, though, force-saves a DTO
/// straight from `load_named`/`load_by_path`, which DOES carry
/// `rev: Some(..)`. So the whole suite exercised a shape the wire never
/// produces, and the one exit offered to a user in a conflict — keep my
/// edits — could not succeed for any page loaded from disk.
#[test]
fn force_save_succeeds_on_the_revless_dto_the_frontend_sends() {
    let dir = scratch("force-save-wire-shape");
    let path = dir.join("pages").join("A.md");
    fs::write(&path, "- original\n").unwrap();
    let g = Graph::open(&dir);
    let mut dto = g.load_named("A", PageKind::Page).unwrap().unwrap();
    let base_rev = dto.rev.clone().unwrap();
    assert!(!dto.path.is_empty(), "a loaded page is path-pinned");
    dto.blocks[0].raw = "mine".into();
    as_editor(&g, &mut dto);
    // The wire shape: the working store has no revision to send.
    dto.rev = None;
    fs::write(&path, "- theirs\n").unwrap();
    let shown = g.save_page(&dto, Some(&base_rev)).unwrap_err();

    g.force_save_page_at_revision(&dto, Some(&base_rev), gh254_shown(&shown))
        .unwrap();

    assert_eq!(fs::read_to_string(&path).unwrap(), "- mine\n");
    let _ = fs::remove_dir_all(&dir);
}

/// The same override after a real external change — the situation that
/// actually raises the conflict banner. The load-time identity pin is stale
/// by construction here, because a foreign writer replaced the file; that
/// staleness is the conflict, not a reason to refuse the resolution.
#[test]
fn force_save_overrides_a_real_external_change_with_the_wire_dto() {
    let dir = scratch("force-save-wire-shape-external");
    let path = dir.join("pages").join("A.md");
    fs::write(&path, "- original\n").unwrap();
    let g = Graph::open(&dir);
    let mut dto = g.load_named("A", PageKind::Page).unwrap().unwrap();
    let base_rev = dto.rev.clone().unwrap();
    dto.blocks[0].raw = "mine".into();
    as_editor(&g, &mut dto);
    dto.rev = None;
    fs::write(&path, "- theirs\n").unwrap();
    let shown = g.save_page(&dto, Some(&base_rev)).unwrap_err();

    g.force_save_page_at_revision(&dto, Some(&base_rev), gh254_shown(&shown))
        .unwrap();

    assert_eq!(fs::read_to_string(&path).unwrap(), "- mine\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn force_save_refuses_unreadable_existing_bytes() {
    let dir = scratch("force-save-invalid-utf8");
    let path = dir.join("pages").join("A.md");
    fs::write(&path, "- original\n").unwrap();
    let g = Graph::open(&dir);
    let mut dto = g.load_named("A", PageKind::Page).unwrap().unwrap();
    dto.blocks[0].raw = "replacement".into();
    let unknown = b"\xff\xfeunknown on-disk bytes";
    fs::write(&path, unknown).unwrap();

    let err = g.save_page(&dto, dto.rev.as_deref()).unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert!(
        g.force_save_page(&dto).is_err(),
        "a hard refusal must not mint override authority"
    );
    assert_eq!(fs::read(&path).unwrap(), unknown);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn twin_md_org_refuses_writes() {
    // M1: a page that exists as BOTH Foo.md and Foo.org is ambiguous — save,
    // force-save, rename, and delete must all refuse (no clobber of either).
    let dir = scratch("org-twin");
    fs::write(dir.join("pages").join("Foo.md"), "- md body\n").unwrap();
    fs::write(dir.join("pages").join("Foo.org"), "* org body\n").unwrap();
    let g = Graph::open(&dir);
    g.warm_cache();
    let page = PageDto {
        activation: None,
        name: "Foo".into(),
        kind: PageKind::Page,
        title: "Foo".into(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: "x".into(),
            raw: "edited".into(),
            ..Default::default()
        }],
        rev: None,
        format: Format::Md,
        read_only: false,
        path: String::new(),
        guide: false,
    };
    assert!(g.save_page(&page, None).is_err(), "save refused on twin");
    assert!(
        g.force_save_page(&page).is_err(),
        "force_save refused on twin"
    );
    assert!(
        g.rename_page("Foo", "Bar").is_err(),
        "rename refused on twin"
    );
    assert!(
        g.delete_page("Foo", PageKind::Page).is_err(),
        "delete refused on twin"
    );
    // Both files are byte-intact (nothing was written/moved/trashed).
    assert_eq!(
        fs::read_to_string(dir.join("pages").join("Foo.md")).unwrap(),
        "- md body\n"
    );
    assert_eq!(
        fs::read_to_string(dir.join("pages").join("Foo.org")).unwrap(),
        "* org body\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

fn guarded_test_resave(graph: &Graph, page: &mut PageDto, marker: &str) -> io::Result<()> {
    page.blocks[0].raw = marker.to_owned();
    let revision = graph.save_page(page, page.rev.as_deref())?;
    page.rev = Some(revision);
    Ok(())
}

fn guarded_test_prime_identity(graph: &Graph) {
    let _identity = graph.lock_graph_text_identity_mutation().unwrap();
    graph.guarded_graph_text_identity_index().unwrap();
}

fn guarded_test_warm_pair(dir: &Path) -> (Graph, Graph) {
    let graph_a = Graph::open(dir);
    let graph_b = Graph::open(dir);
    graph_a.warm_cache();
    graph_b.warm_cache();
    guarded_test_prime_identity(&graph_a);
    guarded_test_prime_identity(&graph_b);
    assert_eq!(graph_a.guarded_graph_text_identity_epochs().0, Some(0));
    assert_eq!(graph_b.guarded_graph_text_identity_epochs().0, Some(0));
    (graph_a, graph_b)
}

/// Poll mode publishes an exact empty set when a complete scan observes no
/// changes. That must advance the exact feed without poisoning or rebuilding
/// an already-live identity index.
#[test]
fn quiet_external_observation_keeps_guarded_identity_warm() {
    let dir = scratch("guarded-identity-quiet-observation");
    fs::write(dir.join("pages/Anchor.md"), b"- anchor\n").unwrap();
    let graph = Graph::open(&dir);
    guarded_test_prime_identity(&graph);
    let before = graph.guarded_graph_text_identity_report();

    graph
        .observe_graph_text_external_paths(std::iter::empty::<&Path>(), false)
        .unwrap();

    let after = graph.guarded_graph_text_identity_report();
    assert!(!after.invalidated, "{after:?}");
    assert_eq!(after.complete_builds, before.complete_builds);
    assert_eq!(after.exact_updates, before.exact_updates + 1);
    let _ = fs::remove_dir_all(&dir);
}

/// GH #374 native-platform witness. ReadDirectoryChangesW may echo Tine's
/// atomic create several times; the exact completed publication and its
/// atomic create during its publication-to-final-reread window. The callback
/// must wait for the same-path writer rather than treating the not-yet-minted
/// completed receipt as an external change.
/// Exact completed and reconciled states are safe no-ops, but neither
/// matching bytes on a replacement inode nor changed bytes on the original
/// inode are ownership proof.
#[test]
fn windows_direct_publication_event_waits_for_inflight_writer_receipt() {
    let dir = scratch("windows-direct-publication-inflight-event");
    fs::write(dir.join("pages/Anchor.md"), b"- anchor\n").unwrap();
    let graph = Arc::new(Graph::open(&dir));
    graph.warm_cache();
    let path = dir.join("pages/Inflight Publication.md");
    let page = direct_save_bench_new_page("Inflight Publication");
    let (published_tx, published_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let writer_graph = Arc::clone(&graph);
    let writer = std::thread::spawn(move || {
        EDITOR_COMMIT_BEFORE_FINAL_REREAD.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                published_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(())
            }));
        });
        writer_graph.save_page(&page, None)
    });
    published_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("writer reached the publication-to-final-reread window");

    let (candidate_tx, candidate_rx) = std::sync::mpsc::channel();
    let observer_graph = Arc::clone(&graph);
    let observer_path = path.clone();
    let observer = std::thread::spawn(move || {
        EXACT_GRAPH_TEXT_EVENT_AFTER_CANDIDATE.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || candidate_tx.send(()).unwrap()));
        });
        observer_graph.exact_graph_text_event_matches_tine_state(&observer_path)
    });
    let reached_candidate = candidate_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .is_ok();
    release_tx.send(()).unwrap();

    writer.join().unwrap().unwrap();
    assert!(
        reached_candidate,
        "the in-flight self-write marker must make the callback wait for the completed receipt"
    );
    assert!(
        observer.join().unwrap(),
        "after the writer releases its lock, exact bytes and identity must prove the self echo"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// GH #374 negative follow-up.  The first fix serialized the exact-path
/// self-echo proof, but an ambiguous Windows callback still published its
/// graph-wide epoch *before* waiting for the same writer.  The create then
/// observed that premature epoch at its last pre-publication check and
/// refused its own save.  The callback frontier must wait behind the writer;
/// it may remain pending for the debounced reconciler only after the create
/// is durably complete.
#[test]
fn windows_ambiguous_callback_cannot_interrupt_inflight_direct_creation() {
    let dir = scratch("windows-direct-ambiguous-callback-inflight");
    fs::write(dir.join("pages/Anchor.md"), b"- anchor\n").unwrap();
    let graph = Arc::new(Graph::open(&dir));
    graph.warm_cache();
    let page = direct_save_bench_new_page("Ambiguous Callback Publication");
    let (paused_tx, paused_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();

    let writer_graph = Arc::clone(&graph);
    let writer = std::thread::spawn(move || {
        GRAPH_TEXT_WRITE_BEFORE_MUTATION.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                paused_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(())
            }));
        });
        writer_graph.save_page(&page, None)
    });
    paused_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("writer reached its final pre-publication boundary");

    let observer_graph = Arc::clone(&graph);
    let observer =
        std::thread::spawn(move || observer_graph.note_graph_text_external_observation());
    wait_for_identity_mutation_waiter(&graph);
    release_tx.send(()).unwrap();

    writer
        .join()
        .unwrap()
        .expect("an overlapping ambiguous callback must not interrupt Tine's create");
    let observed = observer.join().unwrap();
    assert!(
        graph.acknowledge_graph_text_external_observations(observed),
        "the callback still belongs to ordinary reconciliation after publication"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn windows_direct_publication_receipt_requires_revision_and_file_identity() {
    for same_bytes_new_identity in [false, true] {
        let dir = scratch(if same_bytes_new_identity {
            "windows-direct-publication-replaced-identity"
        } else {
            "windows-direct-publication-changed-bytes"
        });
        fs::write(dir.join("pages/Anchor.md"), b"- anchor\n").unwrap();
        let graph = Graph::open(&dir);
        graph.warm_cache();
        let page = direct_save_bench_new_page("Owned Publication");
        graph.save_page(&page, None).unwrap();
        let path = dir.join("pages/Owned Publication.md");
        assert!(
            graph.exact_graph_text_event_matches_tine_state(&path),
            "the completed exact publication receipt must match"
        );
        graph.sync_file_checked(&path).unwrap();
        assert!(
            graph.exact_graph_text_event_matches_tine_state(&path),
            "after reconciliation, exact admitted bytes and identity must match"
        );

        if same_bytes_new_identity {
            let replacement = dir.join("external-winner.tmp");
            fs::write(&replacement, fs::read(&path).unwrap()).unwrap();
            fs::remove_file(&path).unwrap();
            fs::rename(replacement, &path).unwrap();
        } else {
            fs::write(&path, b"- external winner\n").unwrap();
        }
        assert!(
            !graph.exact_graph_text_event_matches_tine_state(&path),
            "an external byte or physical-identity winner must take the guarded external lane"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}

/// A debounced batch may finish after a newer raw callback has arrived. Its
/// acknowledgement must not clear that newer callback's creation barrier.
#[test]
fn older_watcher_batch_cannot_acknowledge_a_newer_observation() {
    let dir = scratch("watcher-observation-frontier");
    fs::write(dir.join("pages/Anchor.md"), b"- anchor\n").unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();

    let older = graph.note_graph_text_external_observation();
    let newer = graph.note_graph_text_external_observation();
    graph.acknowledge_graph_text_external_observations(older);

    let blocked = graph
        .save_page(&direct_save_bench_new_page("Still Pending"), None)
        .expect_err("the newer raw callback must remain pending");
    assert_eq!(blocked.kind(), io::ErrorKind::WouldBlock, "{blocked}");
    assert!(!dir.join("pages/Still Pending.md").exists());

    graph.acknowledge_graph_text_external_observations(newer);
    graph
        .save_page(&direct_save_bench_new_page("Now Reconciled"), None)
        .unwrap();
    assert!(dir.join("pages/Now Reconciled.md").exists());
    let _ = fs::remove_dir_all(&dir);
}

/// Same-root config refresh creates a new `Graph`. A frontier from the
/// retired instance must not advance or wedge the replacement's counters.
#[test]
fn watcher_ticket_cannot_cross_a_same_root_graph_refresh() {
    let dir = scratch("watcher-ticket-same-root-refresh");
    fs::write(dir.join("pages/Anchor.md"), b"- anchor\n").unwrap();
    let retired = Graph::open(&dir);
    retired.warm_cache();
    let retired_ticket = retired.note_graph_text_external_observation();

    let replacement = Graph::open(&dir);
    replacement.warm_cache();
    assert!(!replacement.owns_graph_text_external_observation_ticket(retired_ticket));
    assert!(!replacement.acknowledge_graph_text_external_observations(retired_ticket));
    replacement
        .save_page(&direct_save_bench_new_page("Fresh Instance"), None)
        .unwrap();
    assert!(dir.join("pages/Fresh Instance.md").exists());
    let _ = fs::remove_dir_all(&dir);
}

/// An exact watcher observation updates the retained semantic owner without
/// rebuilding the complete index.
#[test]
fn exact_external_observation_updates_guarded_identity() {
    let dir = scratch("guarded-identity-exact-observation");
    fs::write(dir.join("pages/Anchor.md"), b"- anchor\n").unwrap();
    let graph = Graph::open(&dir);
    guarded_test_prime_identity(&graph);
    let before = graph.guarded_graph_text_identity_report();

    let external = dir.join("Root note.md");
    fs::write(&external, b"title:: Root note\n\n- external\n").unwrap();
    graph
        .observe_graph_text_external_paths(std::iter::once(external.as_path()), false)
        .unwrap();

    let after = graph.guarded_graph_text_identity_report();
    assert!(!after.invalidated, "{after:?}");
    assert_eq!(after.complete_builds, before.complete_builds);
    assert_eq!(after.exact_updates, before.exact_updates + 1);
    let _identity = graph.lock_graph_text_identity_mutation().unwrap();
    let index = graph.guarded_graph_text_identity_index().unwrap();
    assert!(index
        .paths_by_semantic_key
        .contains_key(&(0, crate::refs::page_key("Root note"))));
    assert_eq!(
        graph.guarded_graph_text_identity_report().complete_builds,
        before.complete_builds
    );
    let _ = fs::remove_dir_all(&dir);
}

/// An incomplete poll scan cannot publish an exact final state. Its
/// uncertainty invalidates the retained generation so no later write trusts
/// partial evidence.
#[test]
fn uncertain_external_observation_invalidates_guarded_identity() {
    let dir = scratch("guarded-identity-uncertain-observation");
    fs::write(dir.join("pages/Anchor.md"), b"- anchor\n").unwrap();
    let graph = Graph::open(&dir);
    guarded_test_prime_identity(&graph);
    assert!(!graph.guarded_graph_text_identity_report().invalidated);

    graph
        .observe_graph_text_external_paths(std::iter::empty::<&Path>(), true)
        .unwrap();

    assert!(graph.guarded_graph_text_identity_report().invalidated);
    let _ = fs::remove_dir_all(&dir);
}

/// Watcher routing is resource scoped: observing graph A must not mutate a
/// separate graph B's retained identity generation.
#[test]
fn external_observation_isolated_between_graph_resources() {
    let dir_a = scratch("guarded-identity-resource-a");
    let dir_b = scratch("guarded-identity-resource-b");
    fs::write(dir_a.join("pages/Anchor.md"), b"- anchor A\n").unwrap();
    fs::write(dir_b.join("pages/Anchor.md"), b"- anchor B\n").unwrap();
    let graph_a = Graph::open(&dir_a);
    let graph_b = Graph::open(&dir_b);
    guarded_test_prime_identity(&graph_a);
    guarded_test_prime_identity(&graph_b);
    let before_b = graph_b.guarded_graph_text_identity_report();

    let external_a = dir_a.join("Observed.md");
    fs::write(&external_a, b"- observed A\n").unwrap();
    graph_a
        .observe_graph_text_external_paths(std::iter::once(external_a.as_path()), false)
        .unwrap();

    assert_eq!(graph_b.guarded_graph_text_identity_report(), before_b);
    let _ = fs::remove_dir_all(&dir_a);
    let _ = fs::remove_dir_all(&dir_b);
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// the source is never mutated.
#[test]
#[ignore = "manual real-graph probe: set TINE_REAL_GRAPH to a graph directory"]
fn real_graph_direct_save_does_not_rebuild_the_identity_index() {
    let Some(source) = std::env::var_os("TINE_REAL_GRAPH") else {
        eprintln!("skipped: set TINE_REAL_GRAPH to a graph directory");
        return;
    };
    let dir = scratch("realgraph-identity-probe");
    copy_tree(Path::new(&source), &dir);
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::write(dir.join("pages/Identity Probe.md"), "- probe\n").unwrap();

    let graph = Graph::open(&dir);
    graph.warm_cache();
    guarded_test_prime_identity(&graph);
    let mut page = graph
        .load_by_path("pages/Identity Probe.md")
        .unwrap()
        .unwrap();

    let (builds_before, updates_before, _, generation_before) =
        graph.guarded_graph_text_identity_stats();
    for round in 0..10 {
        guarded_test_resave(&graph, &mut page, &format!("probe {round}")).unwrap();
    }
    let (builds_after, updates_after, invalidated, generation_after) =
        graph.guarded_graph_text_identity_stats();

    println!(
        "REAL-GRAPH IDENTITY PROBE over 10 saves: complete_builds {builds_before} -> \
             {builds_after}, exact_updates {updates_before} -> {updates_after}, \
             invalidated={invalidated}, generation {generation_before} -> {generation_after}"
    );
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(
        builds_before, builds_after,
        "a warm Direct save rebuilt the whole-graph identity index on a real graph; \
             the inventory's Direct-path verdict is wrong"
    );
}

#[test]
#[ignore = "manual W4-E4 gate: unchanged Direct saves on an anonymized corpus copy"]
fn direct_save_typed_errors_accept_anonymized_corpus_copy() {
    let root = fs::canonicalize(PathBuf::from(
        std::env::var_os("TINE_DIRECT_SAVE_CORPUS_COPY")
            .expect("set TINE_DIRECT_SAVE_CORPUS_COPY to a disposable anonymized graph copy"),
    ))
    .expect("the disposable anonymized graph copy must be readable");
    let graph = Graph::open(&root);
    graph.warm_cache();
    let mut attempted = 0_usize;
    let mut failures = 0_usize;

    for entry in graph.list_pages() {
        let Ok(Some(page)) = graph.load_by_path(&entry.rel_path) else {
            failures += 1;
            continue;
        };
        if page.read_only || page.guide {
            continue;
        }
        attempted += 1;
        if graph.save_page(&page, page.rev.as_deref()).is_err() {
            failures += 1;
        }
    }

    eprintln!("direct_save_corpus_copy attempted={attempted} failures={failures}");
    assert!(attempted > 0, "the corpus copy contained no writable pages");
    assert_eq!(
        failures, 0,
        "unchanged Direct saves failed on the corpus copy"
    );
}

#[cfg(unix)]
#[test]
fn resource_epoch_uses_local_existing_proofs_and_cached_creation_proof() {
    // Portable path identity.
    let dir = scratch("guarded-resource-epoch-portable");
    let primary = dir.join("pages/Case.md");
    let collision = dir.join("pages/case.md");
    fs::write(&primary, "- primary\n").unwrap();
    let (graph_a, graph_b) = guarded_test_warm_pair(&dir);
    let mut page_b = graph_b.load_by_path("pages/Case.md").unwrap().unwrap();
    fs::write(&collision, "- collision\n").unwrap();
    graph_a
        .observe_graph_text_external_paths(std::iter::once(collision.as_path()), false)
        .unwrap();
    assert_ne!(
        graph_b.guarded_graph_text_identity_epochs().0,
        Some(graph_b.guarded_graph_text_identity_epochs().1)
    );
    assert_eq!(
        guarded_test_resave(&graph_b, &mut page_b, "refused")
            .unwrap_err()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(
        graph_b.guarded_graph_text_identity_stats().0,
        1,
        "portable refusal must not rebuild a sibling's complete index"
    );
    let _ = fs::remove_dir_all(&dir);

    // Physical resource identity.
    let dir = scratch("guarded-resource-epoch-hardlink");
    let primary = dir.join("pages/A.md");
    let alias = dir.join("pages/Alias.md");
    fs::write(&primary, "- primary\n").unwrap();
    let (graph_a, graph_b) = guarded_test_warm_pair(&dir);
    let mut page_b = graph_b.load_by_path("pages/A.md").unwrap().unwrap();
    fs::hard_link(&primary, &alias).unwrap();
    graph_a
        .observe_graph_text_external_paths(std::iter::once(alias.as_path()), false)
        .unwrap();
    guarded_test_resave(&graph_b, &mut page_b, "allowed").unwrap();
    assert_eq!(
        fs::read_to_string(&primary).unwrap(),
        "- allowed\n",
        "a second link to the target no longer refuses the save (GH #571)"
    );
    assert_eq!(
        graph_b.guarded_graph_text_identity_stats().0,
        1,
        "observing a new link must not rebuild a sibling's complete index"
    );
    let _ = fs::remove_dir_all(&dir);

    // Content-derived semantic identity.
    let dir = scratch("guarded-resource-epoch-semantic");
    fs::write(dir.join("pages/Anchor.md"), "- anchor\n").unwrap();
    let collision = dir.join("pages/Physical Name.md");
    let (graph_a, graph_b) = guarded_test_warm_pair(&dir);
    fs::write(&collision, "title:: Claimed Name\n\n- external\n").unwrap();
    graph_a
        .observe_graph_text_external_paths(std::iter::once(collision.as_path()), false)
        .unwrap();
    graph_b.note_graph_text_external_observation();
    let observed = graph_b.graph_text_external_observation_ticket();
    graph_b.sync_file_checked(&collision).unwrap();
    graph_b.acknowledge_graph_text_external_observations(observed);
    let claimed = markdown_page_dto("Claimed Name", "Claimed Name", "- local\n").unwrap();
    assert_eq!(
        graph_b.save_page(&claimed, None).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(
        graph_b.guarded_graph_text_identity_stats().0,
        1,
        "creation must not rebuild the retained complete index"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn resource_epoch_propagates_uncertain_observation_to_a_sibling() {
    let dir = scratch("guarded-resource-epoch-uncertain");
    fs::write(dir.join("pages/A.md"), "- a\n").unwrap();
    let (graph_a, graph_b) = guarded_test_warm_pair(&dir);
    let mut page_b = graph_b.load_by_path("pages/A.md").unwrap().unwrap();

    graph_a
        .observe_graph_text_external_paths(std::iter::empty::<&Path>(), true)
        .unwrap();
    assert_ne!(
        graph_b.guarded_graph_text_identity_epochs().0,
        Some(graph_b.guarded_graph_text_identity_epochs().1)
    );
    guarded_test_resave(&graph_b, &mut page_b, "saved after uncertainty").unwrap();
    assert_eq!(graph_b.guarded_graph_text_identity_stats().0, 1);
    assert!(graph_b.guarded_graph_text_identity_stats().2);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn resource_epoch_survives_post_filesystem_publication_failure_across_graphs() {
    let dir = scratch("guarded-resource-epoch-publication-failure");
    fs::write(dir.join("pages/A.md"), "- a\n").unwrap();
    fs::write(dir.join("pages/B.md"), "- b\n").unwrap();
    let (graph_a, graph_b) = guarded_test_warm_pair(&dir);
    let mut page_a = graph_a.load_by_path("pages/A.md").unwrap().unwrap();
    let mut page_b = graph_b.load_by_path("pages/B.md").unwrap().unwrap();

    FAIL_NEXT_GUARDED_GRAPH_TEXT_IDENTITY_UPDATE.with(|fail| fail.set(true));
    guarded_test_resave(&graph_a, &mut page_a, "committed before publication failed").unwrap();
    assert!(graph_a.guarded_graph_text_identity_stats().2);
    assert_ne!(
        graph_b.guarded_graph_text_identity_epochs().0,
        Some(graph_b.guarded_graph_text_identity_epochs().1)
    );
    assert_eq!(
        Graph::open(&dir)
            .load_by_path("pages/A.md")
            .unwrap()
            .unwrap()
            .blocks[0]
            .raw,
        "committed before publication failed"
    );

    guarded_test_resave(&graph_b, &mut page_b, "sibling local save").unwrap();
    assert_eq!(graph_b.guarded_graph_text_identity_stats().0, 1);
    assert!(graph_b.guarded_graph_text_identity_stats().2);
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn existing_save_local_proofs_cover_hardlinks_and_index_uncertainty() {
    let dir = scratch("guarded-external-transitions");
    let primary = dir.join("pages/A.md");
    let other = dir.join("pages/Other.md");
    let renamed = dir.join("pages/Renamed.md");
    let alias = dir.join("pages/Alias.md");
    fs::write(&primary, "- original\n").unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let mut page = graph.load_by_path("pages/A.md").unwrap().unwrap();

    guarded_test_resave(&graph, &mut page, "baseline").unwrap();
    assert_eq!(graph.guarded_graph_text_identity_stats().0, 0);

    fs::write(&other, "- external\n").unwrap();
    graph
        .observe_graph_text_external_paths(std::iter::once(other.as_path()), false)
        .unwrap();
    guarded_test_resave(&graph, &mut page, "after create").unwrap();

    fs::rename(&other, &renamed).unwrap();
    graph
        .observe_graph_text_external_paths([other.as_path(), renamed.as_path()].into_iter(), false)
        .unwrap();
    guarded_test_resave(&graph, &mut page, "after rename").unwrap();

    fs::remove_file(&renamed).unwrap();
    graph
        .observe_graph_text_external_paths(std::iter::once(renamed.as_path()), false)
        .unwrap();
    guarded_test_resave(&graph, &mut page, "after delete").unwrap();
    assert_eq!(
        graph.guarded_graph_text_identity_stats().0,
        0,
        "exact external final states do not build the complete generation"
    );

    fs::hard_link(&primary, &alias).unwrap();
    graph
        .observe_graph_text_external_paths(std::iter::once(alias.as_path()), false)
        .unwrap();
    assert!(graph.guarded_graph_text_identity_stats().2);
    guarded_test_resave(&graph, &mut page, "hardlink allowed").unwrap();
    assert_eq!(
        fs::read_to_string(&primary).unwrap(),
        "- hardlink allowed\n",
        "a linked target saves through the ordinary local proofs (GH #571)"
    );
    assert_eq!(
        fs::read_to_string(&alias).unwrap(),
        "- after delete\n",
        "publication is temp + rename, so the other link keeps its own bytes"
    );
    assert_eq!(
        graph.guarded_graph_text_identity_stats().0,
        0,
        "allowing the link must not build the complete generation either"
    );

    fs::remove_file(&alias).unwrap();
    graph
        .observe_graph_text_external_paths(std::iter::empty::<&Path>(), true)
        .unwrap();
    assert!(graph.guarded_graph_text_identity_stats().2);
    guarded_test_resave(&graph, &mut page, "after uncertain observation").unwrap();
    let stats = graph.guarded_graph_text_identity_stats();
    assert_eq!(stats.0, 0, "uncertainty must not build on existing save");
    assert!(stats.2);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn local_portable_refusal_and_semantic_creation_survive_invalidation() {
    for (label, first, sibling) in [
        ("case", "Case.md", "case.md"),
        ("nfc", "Caf\u{e9}.md", "Cafe\u{301}.md"),
    ] {
        let dir = scratch(&format!("guarded-portable-{label}"));
        let first_path = dir.join("pages").join(first);
        let sibling_path = dir.join("pages").join(sibling);
        fs::write(&first_path, "- first\n").unwrap();
        let graph = Graph::open(&dir);
        graph.warm_cache();
        let mut page = graph
            .load_by_path(&format!("pages/{first}"))
            .unwrap()
            .unwrap();
        guarded_test_resave(&graph, &mut page, "baseline").unwrap();

        fs::write(&sibling_path, "- sibling\n").unwrap();
        graph
            .observe_graph_text_external_paths(std::iter::once(sibling_path.as_path()), false)
            .unwrap();
        assert_eq!(
            guarded_test_resave(&graph, &mut page, "retained refusal")
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists,
            "{label} collision must be refused by retained-parent enumeration"
        );

        graph
            .observe_graph_text_external_paths(std::iter::empty::<&Path>(), true)
            .unwrap();
        assert_eq!(
            guarded_test_resave(&graph, &mut page, "rebuilt refusal")
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists,
            "{label} collision must be refused after index invalidation"
        );
        assert_eq!(
            fs::read_to_string(&first_path).unwrap(),
            "- baseline\n",
            "{label} refusal must not write the target"
        );
        assert_eq!(graph.guarded_graph_text_identity_stats().0, 0);
        let _ = fs::remove_dir_all(&dir);
    }

    // Semantic ownership is content-derived and must be visible at the
    // watcher callback boundary, before deferred cache reconciliation.
    let dir = scratch("guarded-semantic-collision");
    fs::write(dir.join("pages/Anchor.md"), "- anchor\n").unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let mut anchor = graph.load_by_path("pages/Anchor.md").unwrap().unwrap();
    guarded_test_resave(&graph, &mut anchor, "baseline").unwrap();
    let external = dir.join("pages/Physical Name.md");
    fs::write(&external, "title:: Claimed Name\n\n- external\n").unwrap();
    graph
        .observe_graph_text_external_paths(std::iter::once(external.as_path()), false)
        .unwrap();
    let observed = graph.graph_text_external_observation_ticket();
    graph.sync_file_checked(&external).unwrap();
    graph.acknowledge_graph_text_external_observations(observed);
    let claimed = markdown_page_dto("Claimed Name", "Claimed Name", "- local\n").unwrap();
    assert_eq!(
        graph.save_page(&claimed, None).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    graph
        .observe_graph_text_external_paths(std::iter::empty::<&Path>(), true)
        .unwrap();
    let rescanned = graph.graph_text_external_observation_ticket();
    graph.sync_file_checked(&external).unwrap();
    graph.acknowledge_graph_text_external_observations(rescanned);
    assert_eq!(
        graph.save_page(&claimed, None).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn guarded_identity_update_failure_does_not_reopen_existing_save_cut() {
    let dir = scratch("guarded-index-update-failure");
    fs::write(dir.join("pages/A.md"), "- original\n").unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    guarded_test_prime_identity(&graph);
    let mut page = graph.load_by_path("pages/A.md").unwrap().unwrap();
    guarded_test_resave(&graph, &mut page, "baseline").unwrap();
    assert_eq!(graph.guarded_graph_text_identity_stats().0, 1);

    FAIL_NEXT_GUARDED_GRAPH_TEXT_IDENTITY_UPDATE.with(|fail| fail.set(true));
    guarded_test_resave(&graph, &mut page, "committed across index failure").unwrap();
    assert!(graph.guarded_graph_text_identity_stats().2);
    assert_eq!(
        Graph::open(&dir)
            .load_by_path("pages/A.md")
            .unwrap()
            .unwrap()
            .blocks[0]
            .raw,
        "committed across index failure"
    );

    guarded_test_resave(&graph, &mut page, "after local recovery").unwrap();
    let stats = graph.guarded_graph_text_identity_stats();
    assert_eq!(stats.0, 1);
    assert!(stats.2);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn readonly_org_unchanged_does_not_reconcile() {
    // L2 check: an UNCHANGED read-only (non-round-tripping) .org file must not
    // spuriously reconcile (bump cache_gen) on a watcher tick — the disk_revs
    // fast path + structural normalize-compare should both treat it as "ours".
    let dir = scratch("org-ro-l2");
    let src = "* a\n*** c\n"; // skipped heading level → read-only
    let path = dir.join("pages").join("RO.org");
    fs::write(&path, src).unwrap();
    let g = Graph::open(&dir);
    g.warm_cache();
    // Confirm it loaded read-only.
    let dto = g.load_named("RO", PageKind::Page).unwrap().unwrap();
    assert!(dto.read_only);
    let gen0 = g.cache_generation();
    // Two watcher reconciles of the unchanged file must be no-ops.
    g.sync_file(&path);
    g.sync_file(&path);
    assert_eq!(
        g.cache_generation(),
        gen0,
        "unchanged read-only org reconciled spuriously"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn orphan_assets_lists_only_unreferenced_media() {
    let dir = scratch("orphans");
    let assets = dir.join("assets");
    fs::create_dir_all(&assets).unwrap();
    // Referenced by blocks (kept): an image, a pdf, a spaced-name clip.
    fs::write(assets.join("used.png"), b"x").unwrap();
    fs::write(assets.join("paper.pdf"), b"x").unwrap();
    fs::write(assets.join("my clip.mp4"), b"x").unwrap();
    // Not referenced (orphans).
    fs::write(assets.join("stray.png"), b"x").unwrap();
    fs::write(assets.join("old_video.webm"), b"x").unwrap();
    // Sidecars / non-media — never flagged.
    fs::write(assets.join("paper.edn"), b"{}").unwrap();
    fs::create_dir_all(assets.join("paper")).unwrap(); // PDF area-image dir
    fs::write(assets.join("paper").join("1_a_2.png"), b"x").unwrap();
    fs::write(
        dir.join("pages").join("P.md"),
        "- ![](../assets/used.png)\n- [paper](../assets/paper.pdf)\n- ![](../assets/my clip.mp4)\n",
    )
    .unwrap();
    let g = Graph::open(&dir);
    let orphans: Vec<String> = g.orphan_assets().into_iter().map(|a| a.name).collect();
    assert_eq!(
        orphans,
        vec!["old_video.webm".to_string(), "stray.png".to_string()]
    );
    // Trash one → it moves out of assets/ into the recoverable trash.
    g.trash_asset("stray.png").unwrap();
    assert!(!assets.join("stray.png").exists());
    assert!(dir.join("logseq").join(".tine-trash").exists());
    // A name with a separator is refused (can't escape assets/).
    assert!(g.trash_asset("../pages/P.md").is_err());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn orphan_assets_does_not_flag_percent_encoded_in_use_asset() {
    // A block links `../assets/my%20file.png` but the file on disk is named
    // `my file.png` (the space percent-encoded in the URL, valid Markdown).
    // The scanner must percent-decode the reference before comparing, so the
    // in-use file is NOT offered for trashing (DS Codex#7).
    let dir = scratch("orphan-pct");
    let assets = dir.join("assets");
    fs::create_dir_all(&assets).unwrap();
    fs::write(assets.join("my file.png"), b"x").unwrap(); // referenced via %20
    fs::write(assets.join("real orphan.png"), b"x").unwrap(); // genuinely unused
    fs::write(
        dir.join("pages").join("P.md"),
        "- ![pic](../assets/my%20file.png)\n",
    )
    .unwrap();
    let g = Graph::open(&dir);
    let orphans: Vec<String> = g.orphan_assets().into_iter().map(|a| a.name).collect();
    assert_eq!(
        orphans,
        vec!["real orphan.png".to_string()],
        "the percent-encoded in-use asset must not be flagged orphan"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn empty_asset_trash_clears_trashed_files() {
    let dir = scratch("empty-trash");
    let assets = dir.join("assets");
    fs::create_dir_all(&assets).unwrap();
    fs::write(assets.join("junk1.png"), b"xx").unwrap(); // 2 bytes
    fs::write(assets.join("junk2.png"), b"yyy").unwrap(); // 3 bytes
    let g = Graph::open(&dir);
    g.trash_asset("junk1.png").unwrap();
    g.trash_asset("junk2.png").unwrap();
    let s = g.asset_trash_stats();
    assert_eq!(s.count, 2, "two files in trash");
    assert_eq!(s.bytes, 5, "2 + 3 bytes preserved through the move");
    assert_eq!(g.empty_asset_trash().unwrap(), 2, "both removed");
    assert_eq!(g.asset_trash_stats().count, 0, "trash empty afterwards");
    // Emptying a never-created trash is a no-op, not an error.
    let dir2 = scratch("empty-trash-missing");
    assert_eq!(Graph::open(&dir2).empty_asset_trash().unwrap(), 0);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&dir2);
}

#[test]
fn empty_asset_trash_keeps_legacy_trashed_pages() {
    let dir = scratch("empty-trash-keeps-pages");
    let trash = dir.join("logseq").join(".tine-trash");
    fs::create_dir_all(&trash).unwrap();
    let asset = trash.join("123-0__unused.png");
    let page = trash.join("123-1__Recovered Page.md");
    fs::write(&asset, b"img").unwrap();
    fs::write(&page, b"- recovered page\n").unwrap();

    let g = Graph::open(&dir);
    let stats = g.asset_trash_stats();
    assert_eq!(stats.count, 1, "legacy asset trash is asset-counted");
    assert_eq!(stats.pages, 1, "legacy page trash is protected-counted");
    assert_eq!(g.empty_asset_trash().unwrap(), 1);
    assert!(
        !asset.exists(),
        "legacy asset trash entry should be deleted"
    );
    assert!(page.exists(), "legacy page trash entry must survive");
    let stats = g.asset_trash_stats();
    assert_eq!(stats.count, 0, "asset trash should be empty");
    assert_eq!(stats.pages, 1, "page trash should still be counted");

    let _ = fs::remove_dir_all(&dir);
}

/// Moving one exact Direct Files document to typed trash needs names and
/// retained file identities, never the contents of unrelated documents.
#[test]
fn direct_trash_move_does_not_capture_unrelated_graph_text_bytes() {
    let dir = scratch("direct-trash-metadata-only");
    fs::write(dir.join("journals/2026_08_25.md"), b"- discard me\n").unwrap();
    for index in 0..24 {
        fs::write(
            dir.join(format!("pages/Unrelated {index}.md")),
            format!("- unrelated {index}\n"),
        )
        .unwrap();
    }
    let graph = Graph::open(&dir);
    graph
        .observe_graph_text_external_paths(std::iter::empty::<&Path>(), true)
        .unwrap();
    let before_reads = graph_text_capture_reads();
    let before_builds = graph.guarded_graph_text_identity_report().complete_builds;

    graph.trash_journal_file("2026_08_25.md").unwrap();

    assert_eq!(graph_text_capture_reads(), before_reads);
    assert_eq!(
        graph.guarded_graph_text_identity_report().complete_builds,
        before_builds
    );
    assert!(!dir.join("journals/2026_08_25.md").exists());
    for index in 0..24 {
        assert_eq!(
            fs::read(dir.join(format!("pages/Unrelated {index}.md"))).unwrap(),
            format!("- unrelated {index}\n").as_bytes()
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn import_asset_uses_given_name() {
    let dir = scratch("import-name");
    let src = dir.join("source.png");
    fs::write(&src, b"img").unwrap();
    let g = Graph::open(&dir);
    let saved = g
        .import_asset(&src, Some("source_20260626_120000.png"))
        .unwrap();
    assert_eq!(saved, "source_20260626_120000.png");
    assert!(dir.join("assets").join(&saved).exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn read_asset_limited_rejects_before_returning_oversized_bytes() {
    let dir = scratch("read-asset-limited");
    let assets = dir.join("assets");
    fs::create_dir_all(&assets).unwrap();
    fs::write(assets.join("large.pdf"), b"12345").unwrap();
    let g = Graph::open(&dir);
    assert_eq!(g.read_asset_limited("large.pdf", 5).unwrap(), b"12345");
    let err = g.read_asset_limited("large.pdf", 4).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert_eq!(err.to_string(), r#"{"kind":"asset-too-large"}"#);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn nested_asset_reads_accept_relative_paths_but_not_traversal() {
    let dir = scratch("nested-asset-read");
    let graph = Graph::open(&dir);
    let nested = dir.join("assets/screenshots/quick-capture.png");
    fs::create_dir_all(nested.parent().unwrap()).unwrap();
    fs::write(&nested, b"nested image").unwrap();

    assert_eq!(
        graph.read_asset("screenshots/quick-capture.png").unwrap(),
        b"nested image"
    );
    assert_eq!(
        graph
            .read_asset_limited("screenshots/quick-capture.png", 32)
            .unwrap(),
        b"nested image"
    );
    assert_eq!(
        graph
            .stream_asset_path("screenshots/quick-capture.png")
            .unwrap(),
        nested.canonicalize().unwrap()
    );

    for bad in [
        "../outside.png",
        "screenshots/../../outside.png",
        "/outside.png",
        "screenshots//quick-capture.png",
        "screenshots/./quick-capture.png",
        "screenshots\\quick-capture.png",
        "",
    ] {
        assert!(graph.read_asset(bad).is_err(), "must reject {bad:?}");
    }

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn asset_path_for_open_accepts_files_directories_and_the_assets_root() {
    // GH #367: the OS opener accepts a regular file, a nested directory,
    // and the empty name (the assets root, OG's `[...](./assets/)`), while
    // keeping the read-path's regular-file gate and traversal rejection.
    let dir = scratch("asset-open");
    let graph = Graph::open(&dir);
    let nested_dir = dir.join("assets/some dir/报表");
    fs::create_dir_all(&nested_dir).unwrap();
    let file = dir.join("assets/some dir/报表/API ref.docx");
    fs::write(&file, b"doc").unwrap();
    let assets = dir.join("assets").canonicalize().unwrap();

    assert_eq!(graph.asset_path_for_open("").unwrap(), assets);
    assert_eq!(
        graph.asset_path_for_open("some dir").unwrap(),
        dir.join("assets/some dir").canonicalize().unwrap()
    );
    assert_eq!(
        graph.asset_path_for_open("some dir/报表").unwrap(),
        nested_dir
    );
    assert_eq!(
        graph
            .asset_path_for_open("some dir/报表/API ref.docx")
            .unwrap(),
        file
    );

    for bad in ["../outside", "/outside", "back\\slash.png", "missing.png"] {
        assert!(
            graph.asset_path_for_open(bad).is_err(),
            "must reject {bad:?}"
        );
    }
    // The regular-file gate for reads is unchanged by the opener route.
    assert!(graph.read_asset("").is_err());
    assert!(graph.read_asset("some dir").is_err());

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn nested_asset_reads_cannot_follow_a_symlink_outside_assets() {
    use std::os::unix::fs::symlink;

    let dir = scratch("nested-asset-symlink");
    let outside = scratch("nested-asset-symlink-outside");
    let graph = Graph::open(&dir);
    fs::create_dir_all(dir.join("assets")).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret.png"), b"private").unwrap();
    symlink(&outside, dir.join("assets/escape")).unwrap();

    assert!(graph.read_asset("escape/secret.png").is_err());
    assert!(graph.stream_asset_path("escape/secret.png").is_err());

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&outside);
}

#[test]
fn rename_aborts_on_readonly_org_referrer() {
    // H1: a rename must NOT rewrite a read-only (non-round-tripping) .org file.
    let dir = scratch("org-rename-ro");
    fs::write(dir.join("pages").join("Alpha.md"), "- alpha\n").unwrap();
    // `* a\n*** c` skips a heading level → not round-trip-safe → read-only.
    let ro = "* a\n*** c referencing [[Alpha]]\n";
    fs::write(dir.join("pages").join("Weird.org"), ro).unwrap();
    let g = Graph::open(&dir);
    g.warm_cache();
    let err = g.rename_page("Alpha", "Beta").unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    // All-or-nothing: neither file moved/changed.
    assert!(
        dir.join("pages").join("Alpha.md").exists(),
        "rename rolled back"
    );
    assert_eq!(
        fs::read_to_string(dir.join("pages").join("Weird.org")).unwrap(),
        ro
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn rename_skips_marker_bearing_referrers_and_reports_them() {
    // A11: a referrer whose file carries column-0 VCS conflict markers is
    // quarantined - the user still owes it a merge resolution. The rename
    // must not rewrite it behind their back; it must leave the bytes exactly
    // as they are, still complete for every other referrer, and report the
    // skipped path so the UI can say which pages still point at the old name.
    let dir = scratch("rename-marker-referrer");
    fs::write(dir.join("pages").join("Alpha.md"), "- alpha\n").unwrap();
    let conflicted = "- intro\n<<<<<<< HEAD\n- mine sees [[Alpha]]\n=======\n- theirs sees [[Alpha]]\n>>>>>>> branch\n";
    fs::write(dir.join("pages").join("Conflicted.md"), conflicted).unwrap();
    fs::write(
        dir.join("pages").join("Clean.md"),
        "- clean sees [[Alpha]]\n",
    )
    .unwrap();
    let g = Graph::open(&dir);
    g.warm_cache();

    let outcome = g.rename_page_reporting("Alpha", "Beta", None).unwrap();

    // The quarantined referrer is byte-identical and still quarantined.
    assert_eq!(
        fs::read_to_string(dir.join("pages").join("Conflicted.md")).unwrap(),
        conflicted,
        "a marker-bearing referrer must not be rewritten"
    );
    assert_eq!(
        g.list_vcs_marker_conflicts().len(),
        1,
        "quarantine must survive the rename"
    );
    // Everything else completed.
    assert_eq!(
        fs::read_to_string(dir.join("pages").join("Clean.md")).unwrap(),
        "- clean sees [[Beta]]\n",
        "clean referrers must still be rewritten"
    );
    assert!(dir.join("pages").join("Beta.md").exists(), "page renamed");
    assert!(!dir.join("pages").join("Alpha.md").exists());
    // And the skip is reported, not silent.
    assert_eq!(outcome.skipped_conflicted_referrers.len(), 1);
    assert!(
        outcome.skipped_conflicted_referrers[0].ends_with("Conflicted.md"),
        "reported path was {:?}",
        outcome.skipped_conflicted_referrers
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn namespace_rename_also_skips_marker_bearing_referrers() {
    // The cascade variant: renaming a parent renames its descendants too, so
    // one quarantined referrer can be hit by several (old, new) pairs in the
    // same pass. It must still come out byte-identical, and be reported once
    // rather than once per pair.
    let dir = scratch("rename-marker-namespace");
    fs::write(dir.join("pages").join("Parent.md"), "- parent\n").unwrap();
    fs::write(dir.join("pages").join("Parent%2FChild.md"), "- child\n").unwrap();
    let conflicted = "- intro\n<<<<<<< HEAD\n- [[Parent]] and [[Parent/Child]]\n=======\n- [[Parent/Child]] only\n>>>>>>> branch\n";
    fs::write(dir.join("pages").join("Conflicted.md"), conflicted).unwrap();
    let g = Graph::open(&dir);
    g.warm_cache();

    let outcome = g.rename_page_reporting("Parent", "Ancestor", None).unwrap();

    assert_eq!(
        fs::read_to_string(dir.join("pages").join("Conflicted.md")).unwrap(),
        conflicted,
        "namespace cascade must not rewrite a quarantined referrer either"
    );
    assert_eq!(
        outcome.skipped_conflicted_referrers.len(),
        1,
        "reported once per file, not once per rename pair: {:?}",
        outcome.skipped_conflicted_referrers
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn rename_org_skips_refs_in_src_block() {
    // H2 end-to-end: renaming a page leaves a `[[Old]]` literal inside an org
    // src block untouched while rewriting a real ref outside it.
    let dir = scratch("org-rename-src");
    fs::write(dir.join("pages").join("Old.md"), "- old body\n").unwrap();
    let org = "* note\nsee [[Old]]\n#+BEGIN_SRC clojure\n\"[[Old]]\"\n#+END_SRC\n";
    fs::write(dir.join("pages").join("Ref.org"), org).unwrap();
    let g = Graph::open(&dir);
    g.warm_cache();
    g.rename_page("Old", "New").unwrap();
    let got = fs::read_to_string(dir.join("pages").join("Ref.org")).unwrap();
    assert_eq!(
        got,
        "* note\nsee [[New]]\n#+BEGIN_SRC clojure\n\"[[Old]]\"\n#+END_SRC\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn org_save_with_typed_headline_caches_disk_tree() {
    // H4: typing a column-0 `* ` line into a block body makes the saved bytes
    // re-parse to a DIFFERENT tree; the cache must reflect what's on disk, not
    // the (now-stale) frontend doc — so reads after the save see the real shape.
    let dir = scratch("org-h4");
    fs::write(dir.join("pages").join("P.org"), "* one\n* two\n").unwrap();
    let g = Graph::open(&dir);
    g.warm_cache();
    let dto = g.load_named("P", PageKind::Page).unwrap().unwrap();
    assert_eq!(dto.blocks.len(), 2);
    // Edit block 0's body to contain a column-0 headline marker.
    let mut edited = dto.clone();
    edited.blocks[0].raw = "one\n* injected".into();
    let rev = g.save_page(&edited, dto.rev.as_deref()).unwrap();
    // Disk now has THREE headlines.
    let disk = fs::read_to_string(dir.join("pages").join("P.org")).unwrap();
    assert_eq!(disk, "* one\n* injected\n* two\n");
    // A fresh load (served from cache) must reflect the 3-block disk structure,
    // not the 2-block frontend doc that produced it.
    let again = g.load_named("P", PageKind::Page).unwrap().unwrap();
    assert_eq!(
        again.blocks.len(),
        3,
        "cache reflects disk structure after H4 reparse"
    );
    assert_eq!(again.rev.as_deref(), Some(rev.as_str()));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn new_page_uses_preferred_format_org() {
    let dir = scratch("org-pref");
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq").join("config.edn"),
        "{:preferred-format \"Org\"}\n",
    )
    .unwrap();
    let g = Graph::open(&dir);
    assert_eq!(g.preferred_format(), Format::Org);
    // Create a brand-new page via save (no baseline) — it must land as .org.
    let page = PageDto {
        activation: None,
        name: "Fresh".into(),
        kind: PageKind::Page,
        title: "Fresh".into(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: "x".into(),
            raw: "hello org".into(),
            ..Default::default()
        }],
        rev: None,
        format: Format::Org,
        read_only: false,
        path: String::new(),
        guide: false,
    };
    g.save_page(&page, None).unwrap();
    assert!(
        dir.join("pages").join("Fresh.org").exists(),
        "new page created as .org"
    );
    assert!(!dir.join("pages").join("Fresh.md").exists());
    assert_eq!(
        fs::read_to_string(dir.join("pages").join("Fresh.org")).unwrap(),
        "* hello org\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn save_skips_rewrite_when_only_whitespace_trivia_differs() {
    // The file has an empty bullet written `- ` (trailing space); the
    // serializer would re-emit it as `-`. A5: a load→save with no real edit
    // must NOT rewrite the file (no Syncthing churn) and must not bump the
    // cache generation — the parsed structure is identical.
    let dir = scratch("noop");
    let path = dir.join("pages").join("A.md");
    let original = "- a\n- \n"; // second bullet: dash + trailing space
    fs::write(&path, original).unwrap();
    let g = Graph::open(&dir);
    g.warm_cache();
    let entry = g.find_entry("A", PageKind::Page).unwrap();
    let dto = g.load_page(&entry).unwrap();
    let gen_before = g.cache_generation();
    let rev = g.save_page(&dto, dto.rev.as_deref()).unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        original,
        "bytes left untouched"
    );
    assert_eq!(
        rev,
        content_rev(original),
        "returned rev is the on-disk rev"
    );
    assert_eq!(
        g.cache_generation(),
        gen_before,
        "no cache_gen bump on a trivia-only no-op"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn save_refuses_page_header_properties_reclassified_as_outline() {
    // GH #163's v0.5.9 Windows follow-up.  The pure property-line helper was
    // innocent; the damaging shape arrived at the native save boundary.
    // Prove that even a contradictory frontend DTO cannot turn B/C into a
    // bullet and continuation line, for either common line-ending family.
    for (label, original) in [
        ("lf", "A:: XX\nB:: XX\nC:: XX\n"),
        ("crlf", "A:: XX\r\nB:: XX\r\nC:: XX\r\n"),
        ("unicode", "A:: XX\nklíč:: hodnota\nC:: XX\n"),
    ] {
        let dir = scratch(&format!("page-property-firewall-{label}"));
        let path = dir.join("pages").join("Property.md");
        fs::write(&path, original).unwrap();
        let g = Graph::open(&dir);
        let mut dto = g.load_named("Property", PageKind::Page).unwrap().unwrap();
        as_editor(&g, &mut dto);
        let normalized = original.replace("\r\n", "\n");
        let normalized = normalized.trim_end_matches('\n');
        assert_eq!(dto.pre_block.as_deref(), Some(normalized));
        assert!(dto.blocks.is_empty());

        let (kept, moved) = normalized.split_once('\n').unwrap();
        dto.pre_block = Some(kept.into());
        dto.blocks = vec![BlockDto {
            id: "corrupt-shape".into(),
            raw: moved.into(),
            ..Default::default()
        }];

        let err = g.save_page(&dto, dto.rev.as_deref()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("page-header property"));
        assert_eq!(fs::read_to_string(&path).unwrap(), original);

        let shown = arm_present_conflict_for_force(&g, &dto, &path);
        let err = g
            .force_save_page_at_revision(&dto, dto.rev.as_deref(), shown)
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        let _ = fs::remove_dir_all(&dir);
    }
}

#[test]
fn save_refuses_changed_page_header_properties_reclassified_as_outline() {
    // H7: the preservation firewall is structural, not an exact-text check.
    // A stale/buggy DTO must not evade it by changing the moved property's
    // value or key while reclassifying it as outline content. Exercise both
    // ordinary and force-save paths from a warm cache and prove neither the
    // bytes nor cached document move on validation failure.
    for (shape, original, kept, moved, childful) in [
        (
            "partial-value",
            "A:: old\nB:: old\n",
            Some("A:: old"),
            "B:: changed",
            false,
        ),
        (
            "partial-key",
            "A:: old\nB:: old\n",
            Some("A:: old"),
            "Renamed:: old",
            false,
        ),
        (
            "whole-key-value",
            "A:: old\nB:: old\n",
            None,
            "Renamed:: changed\nC:: newer",
            true,
        ),
        (
            "crlf",
            "A:: old\r\nB:: old\r\n",
            Some("A:: old"),
            "B:: changed",
            false,
        ),
        (
            "unicode-plugin",
            "A:: old\n插件/键:: old\n",
            Some("A:: old"),
            "插件/新:: changed",
            false,
        ),
    ] {
        for forced in [false, true] {
            let dir = scratch(&format!("page-property-firewall-changed-{shape}-{forced}"));
            let path = dir.join("pages").join("Property.md");
            fs::write(&path, original).unwrap();
            let g = Graph::open(&dir);
            g.warm_cache();
            let mut dto = g.load_named("Property", PageKind::Page).unwrap().unwrap();
            as_editor(&g, &mut dto);
            let cached_before = dto.clone();
            let generation_before = g.cache_generation();
            dto.pre_block = kept.map(str::to_string);
            dto.blocks = vec![BlockDto {
                id: "reclassified-header".into(),
                raw: moved.into(),
                children: childful
                    .then(|| BlockDto {
                        id: "body".into(),
                        raw: "Body".into(),
                        ..Default::default()
                    })
                    .into_iter()
                    .collect(),
                ..Default::default()
            }];

            let err = if forced {
                let shown = arm_present_conflict_for_force(&g, &dto, &path);
                g.force_save_page_at_revision(&dto, dto.rev.as_deref(), shown)
                    .unwrap_err()
            } else {
                g.save_page(&dto, dto.rev.as_deref()).unwrap_err()
            };
            assert_eq!(err.kind(), io::ErrorKind::InvalidData);
            assert_eq!(fs::read_to_string(&path).unwrap(), original);
            assert_eq!(g.cache_generation(), generation_before);
            let cached_after = g.load_named("Property", PageKind::Page).unwrap().unwrap();
            assert_eq!(cached_after.pre_block, cached_before.pre_block);
            assert_eq!(cached_after.blocks.len(), cached_before.blocks.len());
            assert_eq!(cached_after.rev, cached_before.rev);
            let _ = fs::remove_dir_all(&dir);
        }
    }
}

#[test]
fn existing_outline_property_root_remains_editable_beside_page_header() {
    // An outline block that already had page-property-shaped syntax is not a
    // reclassified header. Its structural provenance permits a duplicate
    // header line to be deleted without blaming the already-existing root,
    // and the root remains ordinarily editable afterwards.
    let dir = scratch("page-property-existing-outline-provenance");
    let path = dir.join("pages").join("Property.md");
    fs::write(&path, "A:: header\nB:: shared\n\n- B:: shared\n").unwrap();
    let g = Graph::open(&dir);
    g.warm_cache();
    let mut dto = g.load_named("Property", PageKind::Page).unwrap().unwrap();
    dto.pre_block = Some("A:: edited header".into());
    g.save_page(&dto, dto.rev.as_deref()).unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "A:: edited header\n\n- B:: shared\n"
    );
    let mut warm = g.load_named("Property", PageKind::Page).unwrap().unwrap();
    assert_eq!(warm.pre_block.as_deref(), Some("A:: edited header"));
    assert_eq!(warm.blocks[0].raw, "B:: shared");
    warm.blocks[0].raw = "Renamed:: edited outline".into();
    g.save_page(&warm, warm.rev.as_deref()).unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "A:: edited header\n\n- Renamed:: edited outline\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn page_header_property_save_reopens_as_metadata_with_original_line_endings() {
    // Complements the real gear-panel E2E: drive the native save and a fresh
    // Graph/parser instance so success cannot come from the just-written
    // frontend store or Graph cache.
    for (label, original, expected) in [
        (
            "lf",
            "A:: XX\nB:: XX\nC:: XX\n",
            "icon:: ★\nA:: XX\nB:: XX\nC:: XX\n",
        ),
        (
            "crlf",
            "A:: XX\r\nB:: XX\r\nC:: XX\r\n",
            "icon:: ★\r\nA:: XX\r\nB:: XX\r\nC:: XX\r\n",
        ),
    ] {
        let dir = scratch(&format!("page-property-positive-{label}"));
        let path = dir.join("pages").join("Property.md");
        fs::write(&path, original).unwrap();
        let g = Graph::open(&dir);
        let mut dto = g.load_named("Property", PageKind::Page).unwrap().unwrap();
        as_editor(&g, &mut dto);
        dto.pre_block = Some("icon:: ★\nA:: XX\nB:: XX\nC:: XX".into());
        g.save_page(&dto, dto.rev.as_deref()).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), expected);
        drop(g);

        let reopened = Graph::open(&dir)
            .load_named("Property", PageKind::Page)
            .unwrap()
            .unwrap();
        assert_eq!(
            reopened.pre_block.as_deref(),
            Some("icon:: ★\nA:: XX\nB:: XX\nC:: XX")
        );
        assert!(reopened.blocks.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}

#[test]
fn new_property_only_first_root_becomes_canonical_page_header() {
    let dir = scratch("page-property-authoring");
    let g = Graph::open(&dir);
    g.warm_cache();
    let page = PageDto {
        activation: None,
        name: "Property Authoring".into(),
        kind: PageKind::Page,
        title: "Property Authoring".into(),
        pre_block: None,
        blocks: vec![
            BlockDto {
                id: "transient-header".into(),
                raw: "alias:: book\n\nklíč:: hodnota".into(),
                ..Default::default()
            },
            BlockDto {
                id: "body".into(),
                raw: "Reading list".into(),
                ..Default::default()
            },
        ],
        rev: None,
        format: Format::Md,
        read_only: false,
        path: String::new(),
        guide: false,
    };
    g.save_page(&page, None).unwrap();
    let path = dir.join("pages").join("Property Authoring.md");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "alias:: book\n\nklíč:: hodnota\n\n- Reading list\n"
    );

    let warm = g
        .load_named("Property Authoring", PageKind::Page)
        .unwrap()
        .unwrap();
    assert_eq!(
        warm.pre_block.as_deref(),
        Some("alias:: book\n\nklíč:: hodnota")
    );
    assert_eq!(warm.blocks.len(), 1);
    assert_eq!(warm.blocks[0].raw, "Reading list");
    assert_eq!(
        warm.blocks[0].id, "body",
        "normalization changed the body root identity"
    );
    drop(g);
    let cold = Graph::open(&dir)
        .load_named("Property Authoring", PageKind::Page)
        .unwrap()
        .unwrap();
    assert_eq!(cold.pre_block, warm.pre_block);
    assert_eq!(cold.blocks.len(), warm.blocks.len());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn gh198_canonical_preamble_dto_resaves_cleanly_over_existing_preamble() {
    // GH #198 persistence-boundary complement. The store fix (pageToDto folds
    // a flagless properties-only first bullet into pre_block) makes the frontend
    // emit pre_block=properties + no bullet. Prove that this corrected DTO
    // shape resaves without tripping the GH #163 preservation firewall even
    // when disk already carries the identical unbulleted preamble — the exact
    // second-save that previously jammed the queue with "will retry".
    let dir = scratch("gh198-canonical-resave");
    let path = dir.join("pages").join("The Nazi Mind.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "title:: The Nazi Mind\ntags:: books\n").unwrap();
    let g = Graph::open(&dir);
    let loaded = g
        .load_named("The Nazi Mind", PageKind::Page)
        .unwrap()
        .unwrap();
    assert_eq!(
        loaded.pre_block.as_deref(),
        Some("title:: The Nazi Mind\ntags:: books")
    );
    assert!(loaded.blocks.is_empty());

    let dto = PageDto {
        activation: None,
        name: "The Nazi Mind".into(),
        kind: PageKind::Page,
        title: "The Nazi Mind".into(),
        pre_block: Some("title:: The Nazi Mind\ntags:: books".into()),
        blocks: vec![],
        rev: None,
        format: Format::Md,
        read_only: false,
        path: String::new(),
        guide: false,
    };
    g.save_page(&dto, loaded.rev.as_deref())
        .expect("corrected canonical-preamble DTO must save over an existing preamble");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "title:: The Nazi Mind\ntags:: books\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn page_header_authoring_is_bounded_and_preserves_existing_preambles() {
    assert!(page_header_properties_only(
        "alias:: book\n\ne\u{301}/plugin.key::value"
    ));
    for invalid in [
        " alias:: x",
        "#alias:: x",
        "alias key:: x",
        "alias:: x\nprose",
        "```\nalias:: x\n```",
        "alias:: x\n",
    ] {
        assert!(
            !page_header_properties_only(invalid),
            "accepted {invalid:?}"
        );
    }

    // A headerless CRLF page can add a canonical header; both the warm cache
    // and a fresh parser expose exactly the normalized document shape.
    let dir = scratch("page-property-existing-headerless");
    let path = dir.join("pages").join("Existing.md");
    fs::write(&path, "- Body\r\n").unwrap();
    let g = Graph::open(&dir);
    g.warm_cache();
    let mut dto = g.load_named("Existing", PageKind::Page).unwrap().unwrap();
    dto.blocks.insert(
        0,
        BlockDto {
            id: "transient-header".into(),
            raw: "custom/key:: exact value".into(),
            ..Default::default()
        },
    );
    g.save_page(&dto, dto.rev.as_deref()).unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "custom/key:: exact value\r\n\r\n- Body\r\n"
    );
    let warm = g.load_named("Existing", PageKind::Page).unwrap().unwrap();
    assert_eq!(warm.pre_block.as_deref(), Some("custom/key:: exact value"));
    assert_eq!(warm.blocks.len(), 1);
    drop(g);
    let cold = Graph::open(&dir)
        .load_named("Existing", PageKind::Page)
        .unwrap()
        .unwrap();
    assert_eq!(cold.pre_block, warm.pre_block);
    assert_eq!(cold.blocks.len(), warm.blocks.len());
    let _ = fs::remove_dir_all(&dir);

    // A non-property preamble may only move through GH #85's explicit prose
    // promotion. A property candidate cannot make that preamble disappear,
    // even through force-save, and the warm cache stays on the disk version.
    let dir = scratch("page-property-preamble-loss");
    let path = dir.join("pages").join("Imported.md");
    let original = "Intro before outline\n\n- Body\n";
    fs::write(&path, original).unwrap();
    let g = Graph::open(&dir);
    g.warm_cache();
    let mut dto = g.load_named("Imported", PageKind::Page).unwrap().unwrap();
    dto.pre_block = None;
    dto.blocks.insert(
        0,
        BlockDto {
            id: "candidate".into(),
            raw: "alias:: book".into(),
            ..Default::default()
        },
    );
    as_editor(&g, &mut dto);
    for forced in [false, true] {
        let err = if forced {
            let shown = arm_present_conflict_for_force(&g, &dto, &path);
            g.force_save_page_at_revision(&dto, dto.rev.as_deref(), shown)
                .unwrap_err()
        } else {
            g.save_page(&dto, dto.rev.as_deref()).unwrap_err()
        };
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("existing page preamble"));
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        let cached = g.load_named("Imported", PageKind::Page).unwrap().unwrap();
        assert_eq!(cached.pre_block.as_deref(), Some("Intro before outline"));
        assert_eq!(cached.blocks.len(), 1);
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn page_header_authoring_never_promotes_unsafe_or_nonfirst_roots() {
    let cases: Vec<(&str, Format, Vec<BlockDto>)> = vec![
        (
            "later",
            Format::Md,
            vec![
                BlockDto {
                    id: "body".into(),
                    raw: "Body".into(),
                    ..Default::default()
                },
                BlockDto {
                    id: "prop".into(),
                    raw: "alias:: book".into(),
                    ..Default::default()
                },
            ],
        ),
        (
            "mixed",
            Format::Md,
            vec![BlockDto {
                id: "mixed".into(),
                raw: "alias:: book\nprose".into(),
                ..Default::default()
            }],
        ),
        (
            "fenced",
            Format::Md,
            vec![BlockDto {
                id: "fenced".into(),
                raw: "```\nalias:: book\n```".into(),
                ..Default::default()
            }],
        ),
        (
            "empty",
            Format::Md,
            vec![BlockDto {
                id: "empty".into(),
                raw: "".into(),
                ..Default::default()
            }],
        ),
        (
            "childful",
            Format::Md,
            vec![BlockDto {
                id: "parent".into(),
                raw: "alias:: book".into(),
                children: vec![BlockDto {
                    id: "child".into(),
                    raw: "Child".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
        ),
        (
            "id-bearing",
            Format::Md,
            vec![BlockDto {
                id: "durable".into(),
                raw: "id:: 11111111-1111-4111-8111-111111111111".into(),
                ..Default::default()
            }],
        ),
        (
            "org",
            Format::Org,
            vec![BlockDto {
                id: "org".into(),
                raw: "alias:: book".into(),
                ..Default::default()
            }],
        ),
    ];
    for (label, format, blocks) in cases {
        let dir = scratch(&format!("page-property-negative-{label}"));
        if format == Format::Org {
            fs::create_dir_all(dir.join("logseq")).unwrap();
            fs::write(
                dir.join("logseq").join("config.edn"),
                "{:preferred-format \"Org\"}\n",
            )
            .unwrap();
        }
        let g = Graph::open(&dir);
        let page = PageDto {
            activation: None,
            name: format!("Negative {label}"),
            kind: PageKind::Page,
            title: format!("Negative {label}"),
            pre_block: None,
            blocks: blocks.clone(),
            rev: None,
            format,
            read_only: false,
            path: String::new(),
            guide: false,
        };
        g.save_page(&page, None).unwrap();
        let reopened = g.load_named(&page.name, PageKind::Page).unwrap().unwrap();
        assert!(reopened.pre_block.is_none(), "promoted unsafe case {label}");
        assert_eq!(
            reopened.blocks.len(),
            blocks.len(),
            "changed root count for {label}"
        );
        if label == "id-bearing" {
            assert!(
                g.resolve_block("11111111-1111-4111-8111-111111111111")
                    .is_some(),
                "ID-bearing root lost addressability"
            );
        }
        let _ = fs::remove_dir_all(&dir);
    }
}

fn mkhl(id: &str, page: i64, text: Option<&str>) -> crate::pdf::Highlight {
    let r = crate::pdf::Rect {
        top: 1.0,
        left: 2.0,
        width: 3.0,
        height: 4.0,
        source_width: None,
        source_height: None,
    };
    crate::pdf::Highlight {
        id: id.into(),
        page,
        position: crate::pdf::Position {
            page,
            bounding: r.clone(),
            rects: vec![r],
        },
        color: "yellow".into(),
        text: text.map(String::from),
        image: None,
    }
}

#[test]
fn write_highlights_refuses_unreadable_artifacts_without_partial_commit() {
    let dir = scratch("highlights-invalid-utf8");
    let g = Graph::open(&dir);
    let key = crate::pdf::asset_key("paper.pdf");
    let edn_path = dir.join("assets").join(format!("{key}.edn"));
    fs::create_dir_all(dir.join("assets")).unwrap();
    let unknown = b"\xff\xfeunknown sidecar bytes";
    fs::write(&edn_path, unknown).unwrap();
    let h = mkhl("11111111-1111-1111-1111-111111111111", 1, Some("text"));

    let err = g
        .write_highlights("paper.pdf", "Paper", &[h], &[])
        .unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert_eq!(fs::read(&edn_path).unwrap(), unknown);
    assert!(!dir.join("pages").join(format!("hls__{key}.md")).exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn opening_pdf_creates_og_artifacts_in_preferred_org_format() {
    let dir = scratch("pdf-open-org");
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq").join("config.edn"),
        "{:preferred-format \"Org\"}\n",
    )
    .unwrap();
    let g = Graph::open(&dir);
    let state = g.open_pdf("paper.pdf", "Paper").unwrap();
    assert!(state.highlights.is_empty());
    assert_eq!(state.page, None);
    assert_eq!(state.scale, None);

    let sidecar = fs::read_to_string(dir.join("assets").join("paper.edn")).unwrap();
    assert_eq!(crate::pdf::parse_pdf_state(&sidecar), state);
    let org_path = dir.join("pages").join("hls__paper.org");
    assert!(org_path.exists());
    assert!(!dir.join("pages").join("hls__paper.md").exists());
    let org = fs::read_to_string(org_path).unwrap();
    assert!(
        org.contains("#+FILE: [[../assets/paper.pdf][Paper]]"),
        "{org}"
    );
    assert!(org.contains("#+FILE-PATH: ../assets/paper.pdf"), "{org}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn pdf_view_state_update_preserves_highlights_and_foreign_edn() {
    let dir = scratch("pdf-view-state");
    let g = Graph::open(&dir);
    let key = crate::pdf::asset_key("paper.pdf");
    let sidecar_path = dir.join("assets").join(format!("{key}.edn"));
    fs::create_dir_all(dir.join("assets")).unwrap();
    let h = mkhl("11111111-1111-1111-1111-111111111111", 3, Some("text"));
    let original = crate::pdf::write_highlights(&[h.clone()], "{:extra {:plugin \"keep\"}}");
    fs::write(&sidecar_path, original).unwrap();

    g.write_pdf_view_state("paper.pdf", 8, 1.9).unwrap();

    let written = fs::read_to_string(&sidecar_path).unwrap();
    let state = crate::pdf::parse_pdf_state(&written);
    assert_eq!(state.highlights, vec![h]);
    assert_eq!(state.page, Some(8));
    assert_eq!(state.scale, Some(1.9));
    let root = crate::edn::parse_strict(&written).unwrap();
    assert_eq!(
        root.get("extra")
            .unwrap()
            .get("plugin")
            .and_then(crate::edn::Edn::as_str),
        Some("keep")
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn highlight_write_keeps_existing_hls_format_and_uses_org_drawers() {
    let dir = scratch("pdf-highlight-org");
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq").join("config.edn"),
        "{:preferred-format \"Org\"}\n",
    )
    .unwrap();
    let g = Graph::open(&dir);
    let h = mkhl("11111111-1111-1111-1111-111111111111", 3, Some("text"));
    g.write_highlights("paper.pdf", "Paper", &[h], &[]).unwrap();
    let org_path = dir.join("pages").join("hls__paper.org");
    let org = fs::read_to_string(&org_path).unwrap();
    assert!(org.contains("* text"), "{org}");
    assert!(org.contains(":PROPERTIES:"), "{org}");
    assert!(org.contains(":hl-page: 3"), "{org}");
    assert!(crate::org::org_round_trips(&org));

    // Preferred format changes later must not fork the existing annotation
    // page into a second extension.
    fs::write(
        dir.join("logseq").join("config.edn"),
        "{:preferred-format \"Markdown\"}\n",
    )
    .unwrap();
    let reopened = Graph::open(&dir);
    let h2 = mkhl("22222222-2222-2222-2222-222222222222", 4, Some("more"));
    reopened
        .write_highlights("paper.pdf", "Paper", &[h2], &[])
        .unwrap();
    assert!(org_path.exists());
    assert!(!dir.join("pages").join("hls__paper.md").exists());
    let _ = fs::remove_dir_all(&dir);
}

/// Concord invariant 4, as a standing guard rather than a per-defect test.
/// Opening a graph and READING every page in it must not touch one byte of
/// the tree — no reformat, no rename, no new file. A graph kept in git turns
/// every spurious write into a diff, and this is the one invariant a user
/// notices immediately.
///
/// The fixture is deliberately hostile to a default serializer: two-space
/// indent, no trailing newline, CRLF, an extra blank line after the page
/// preamble, a title-named journal the filename migration would rename, an
/// org page, and a `.markdown` spelling.
#[test]
fn opening_and_reading_a_graph_rewrites_nothing() {
    let dir = scratch("write-shy-open");
    fs::create_dir_all(dir.join("logseq")).unwrap();
    let files: &[(&str, &str)] = &[
        (
            "logseq/config.edn",
            "{:journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
        ),
        // two-space indent, no trailing newline
        (
            "pages/Two Space.md",
            "- parent\n  - child\n    - grandchild",
        ),
        // CRLF, three trailing newlines
        (
            "pages/Crlf.md",
            "title:: Crlf\r\n\r\n- one\r\n- two\r\n\r\n\r\n",
        ),
        // two blank lines after the preamble
        ("pages/Preamble.md", "alias:: p\ntags:: a, b\n\n\n- body\n"),
        ("pages/Org.org", "#+TITLE: Org\n* head\n** child\n"),
        ("pages/Long.markdown", "- long extension spelling\n"),
        // a journal whose name does not round-trip to its date
        (
            "journals/Thursday, 25-06-2026.md",
            "- title-named journal\n",
        ),
        ("journals/2026_06_26.md", "- canonical journal\n"),
    ];
    for (relative, content) in files {
        let path = dir.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
    }
    let before = graph_tree_snapshot(&dir);

    let g = Graph::open(&dir);
    let entries = g.list_pages();
    assert!(entries.len() >= 5, "the fixture pages were discovered");
    for entry in &entries {
        let _ = g.load_page(entry);
    }
    for entry in g.journals_desc() {
        let _ = g.load_page(&entry);
    }
    let _ = g.list_sync_conflicts();
    let _ = g.list_vcs_marker_conflicts();
    let _ = g.conflict_queue();
    let _ = g.journal_conflicts();
    let _ = g.journal_filename_migrations();

    assert_eq!(
        graph_tree_snapshot(&dir),
        before,
        "opening and reading a graph must leave every file byte-identical"
    );

    // ...and the same for a save that changes nothing: load each page, hand
    // the untouched DTO straight back to `save_page`. Anything the round
    // trip normalizes would be a rewrite of bytes the user did not change.
    for entry in &entries {
        let Ok(dto) = g.load_page(entry) else {
            continue;
        };
        let rev = dto.rev.clone();
        g.save_page(&dto, rev.as_deref()).unwrap_or_else(|error| {
            panic!("re-saving unchanged {} failed: {error}", entry.rel_path)
        });
    }
    assert_eq!(
        graph_tree_snapshot(&dir),
        before,
        "re-saving an unchanged page must leave every file byte-identical"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Every file under `root`, by graph-relative path, with its exact bytes.
fn graph_tree_snapshot(root: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    let mut out = std::collections::BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let Ok(read_dir) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                stack.push(path);
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(relative, fs::read(&path).unwrap_or_default());
        }
    }
    out
}

/// Concord invariant 4 (write-shyness). An `hls__` page is an ordinary
/// Logseq page: the user (or OG) may have written it with two-space
/// indentation and no trailing newline, and it may carry hand-written note
/// children. Re-saving the SAME highlight set is not a semantic change, so
/// it must not touch a single byte — every spurious rewrite is a diff in a
/// graph kept in git, and a wake for every sync tool watching the tree.
#[test]
fn write_highlights_leaves_an_unchanged_hls_page_byte_identical() {
    let dir = scratch("highlights-write-shy");
    let g = Graph::open(&dir);
    let key = crate::pdf::asset_key("paper.pdf");
    let page_path = dir.join("pages").join(format!("hls__{key}.md"));
    let h = mkhl("11111111-1111-1111-1111-111111111111", 1, Some("text"));
    g.write_highlights("paper.pdf", "Paper", &[h.clone()], &[])
        .unwrap();
    // Rewrite the generated page in the OTHER house style the ecosystem
    // uses: two-space indent, no trailing newline, plus a user note child.
    let generated = fs::read_to_string(&page_path).unwrap();
    let restyled = format!("{}\n  - my own note\n", generated.trim_end()).replace('\t', "  ");
    let restyled = restyled.trim_end().to_string();
    fs::write(&page_path, &restyled).unwrap();

    let reopened = Graph::open(&dir);
    reopened
        .write_highlights("paper.pdf", "Paper", &[h], &[])
        .unwrap();

    assert_eq!(
        fs::read_to_string(&page_path).unwrap(),
        restyled,
        "re-saving the same highlights must not rewrite the page"
    );
    assert!(
        !dir.join("logseq").join(".tine-trash").exists(),
        "a highlight save must not materialize a trash directory it never uses"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Concord invariant 3: a marker-bearing page is quarantined from EVERY
/// writer. This path used to bypass the refusal — one added highlight
/// rewrote the conflicted `hls__` page, re-indented the markers off column
/// 0, and silently LIFTED the quarantine while the VCS still considered
/// the merge unresolved.
#[test]
fn write_highlights_refuses_a_marker_bearing_hls_page() {
    let dir = scratch("highlights-marker-quarantine");
    let g = Graph::open(&dir);
    let key = crate::pdf::asset_key("paper.pdf");
    let page_path = dir.join("pages").join(format!("hls__{key}.md"));
    let h = mkhl("11111111-1111-1111-1111-111111111111", 1, Some("text"));
    g.write_highlights("paper.pdf", "Paper", &[h.clone()], &[])
        .unwrap();
    // A git merge left column-0 conflict markers in the hls page.
    let conflicted = format!(
        "<<<<<<< HEAD\n{}=======\n- the other merge side\n>>>>>>> feature\n",
        fs::read_to_string(&page_path).unwrap()
    );
    fs::write(&page_path, &conflicted).unwrap();

    let reopened = Graph::open(&dir);
    assert!(
        reopened
            .list_vcs_marker_conflicts()
            .iter()
            .any(|c| c.path == format!("pages/hls__{key}.md")),
        "the conflicted hls page is quarantined"
    );
    let before = graph_tree_snapshot(&dir);
    let h2 = mkhl("22222222-2222-2222-2222-222222222222", 4, Some("more"));
    let err = reopened
        .write_highlights("paper.pdf", "Paper", &[h, h2], &[])
        .expect_err("a highlight write to a conflicted page must refuse");
    assert!(
        err.to_string().contains("conflict markers"),
        "the refusal names the markers: {err}"
    );
    assert_eq!(
        graph_tree_snapshot(&dir),
        before,
        "the refusal leaves every file byte-identical (page AND sidecar)"
    );
    assert!(
        Graph::open(&dir)
            .list_vcs_marker_conflicts()
            .iter()
            .any(|c| c.path == format!("pages/hls__{key}.md")),
        "the quarantine still stands"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// The same invariant when there IS a semantic change: adding a highlight
/// appends one block and leaves the rest of the file's formatting alone.
#[test]
fn write_highlights_keeps_the_hls_pages_formatting_when_it_does_change() {
    let dir = scratch("highlights-write-shy-changed");
    let g = Graph::open(&dir);
    let key = crate::pdf::asset_key("paper.pdf");
    let page_path = dir.join("pages").join(format!("hls__{key}.md"));
    let h = mkhl("11111111-1111-1111-1111-111111111111", 1, Some("text"));
    g.write_highlights("paper.pdf", "Paper", &[h.clone()], &[])
        .unwrap();
    let restyled = fs::read_to_string(&page_path)
        .unwrap()
        .replace('\t', "  ")
        .trim_end()
        .to_string()
        + "\n  - my own note";
    fs::write(&page_path, &restyled).unwrap();

    let reopened = Graph::open(&dir);
    let h2 = mkhl("22222222-2222-2222-2222-222222222222", 4, Some("more"));
    reopened
        .write_highlights("paper.pdf", "Paper", &[h, h2], &[])
        .unwrap();

    let after = fs::read_to_string(&page_path).unwrap();
    assert!(
        after.contains("more"),
        "the new highlight landed: {after:?}"
    );
    assert!(
        after.contains("  - my own note"),
        "the user's note keeps its two-space indent: {after:?}"
    );
    assert!(
        !after.contains('\t'),
        "no line was re-indented with tabs: {after:?}"
    );
    assert!(
        !after.ends_with('\n'),
        "the file's missing trailing newline is preserved: {after:?}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn write_highlights_checks_notes_page_before_sidecar_commit() {
    let dir = scratch("highlights-invalid-page");
    let g = Graph::open(&dir);
    let key = crate::pdf::asset_key("paper.pdf");
    let page_path = dir.join("pages").join(format!("hls__{key}.md"));
    let unknown = b"\xff\xfeunknown notes bytes";
    fs::write(&page_path, unknown).unwrap();
    let h = mkhl("11111111-1111-1111-1111-111111111111", 1, Some("text"));

    let err = g
        .write_highlights("paper.pdf", "Paper", &[h], &[])
        .unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert_eq!(fs::read(&page_path).unwrap(), unknown);
    assert!(!dir.join("assets").join(format!("{key}.edn")).exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn write_highlights_checks_read_only_org_page_before_sidecar_commit() {
    let dir = scratch("highlights-readonly-org-page");
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq").join("config.edn"),
        "{:preferred-format \"Org\"}\n",
    )
    .unwrap();
    let key = crate::pdf::asset_key("paper.pdf");
    let page_path = dir.join("pages").join(format!("hls__{key}.org"));
    fs::write(&page_path, "* a\n*** c\n").unwrap();
    let sidecar_path = dir.join("assets").join(format!("{key}.edn"));
    fs::create_dir_all(dir.join("assets")).unwrap();
    let original = "{:highlights [] :extra {:plugin \"keep\"}}\n";
    fs::write(&sidecar_path, original).unwrap();
    let h = mkhl("11111111-1111-1111-1111-111111111111", 1, Some("text"));

    let err = Graph::open(&dir)
        .write_highlights("paper.pdf", "Paper", &[h], &[])
        .unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(fs::read_to_string(&sidecar_path).unwrap(), original);
    assert_eq!(fs::read_to_string(&page_path).unwrap(), "* a\n*** c\n");
    let _ = fs::remove_dir_all(&dir);
}

/// Make `dir` read-only and report whether the restriction is actually
/// enforced for this process. Root ignores directory permissions, so a test
/// that needs a write to FAIL cannot demonstrate anything when running as
/// uid 0 — it must skip rather than pass vacuously or fail spuriously.
#[cfg(unix)]
fn deny_writes_if_enforced(dir: &Path) -> Option<fs::Permissions> {
    use std::os::unix::fs::PermissionsExt;

    let original = fs::metadata(dir).unwrap().permissions();
    let mut read_only = original.clone();
    read_only.set_mode(0o555);
    fs::set_permissions(dir, read_only).unwrap();
    let probe = dir.join(".write-enforcement-probe");
    match fs::write(&probe, b"x") {
        Ok(()) => {
            let _ = fs::remove_file(&probe);
            fs::set_permissions(dir, original).unwrap();
            None
        }
        Err(_) => Some(original),
    }
}

#[cfg(unix)]
#[test]
fn write_highlights_rolls_back_sidecar_when_notes_page_commit_fails() {
    let dir = scratch("highlights-page-commit-rollback");
    let g = Graph::open(&dir);
    let key = crate::pdf::asset_key("paper.pdf");
    let page_path = dir.join("pages").join(format!("hls__{key}.md"));
    let page_before = "- Existing annotation note\n";
    fs::write(&page_path, page_before).unwrap();
    let sidecar_path = dir.join("assets").join(format!("{key}.edn"));
    fs::create_dir_all(dir.join("assets")).unwrap();
    let sidecar_before = "{:highlights [] :extra {:plugin \"keep\"}}\n";
    fs::write(&sidecar_path, sidecar_before).unwrap();
    let h = mkhl("11111111-1111-1111-1111-111111111111", 1, Some("text"));

    let pages = dir.join("pages");
    let Some(original_permissions) = deny_writes_if_enforced(&pages) else {
        let _ = fs::remove_dir_all(&dir);
        return; // running as root: a read-only directory proves nothing
    };
    let result = g.write_highlights("paper.pdf", "Paper", &[h], &[]);
    fs::set_permissions(&pages, original_permissions).unwrap();

    assert!(
        result.is_err(),
        "the notes-page commit must fail in a read-only directory"
    );
    assert_eq!(fs::read_to_string(&sidecar_path).unwrap(), sidecar_before);
    assert_eq!(fs::read_to_string(&page_path).unwrap(), page_before);
    assert!(
        !g.recent_writes.lock().unwrap().contains_key(&page_path),
        "a failed page commit must not leave a stale watcher suppression marker"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn write_highlights_quarantines_new_sidecar_when_notes_page_commit_fails() {
    let dir = scratch("highlights-new-sidecar-page-failure");
    let g = Graph::open(&dir);
    let key = crate::pdf::asset_key("paper.pdf");
    let page_path = dir.join("pages").join(format!("hls__{key}.md"));
    let page_before = "- Existing annotation note\n";
    fs::write(&page_path, page_before).unwrap();
    fs::create_dir_all(dir.join("assets")).unwrap();
    let sidecar_path = dir.join("assets").join(format!("{key}.edn"));
    let h = mkhl("11111111-1111-1111-1111-111111111111", 1, Some("text"));

    let pages = dir.join("pages");
    let Some(original_permissions) = deny_writes_if_enforced(&pages) else {
        let _ = fs::remove_dir_all(&dir);
        return; // running as root: a read-only directory proves nothing
    };
    let result = g.write_highlights("paper.pdf", "Paper", &[h], &[]);
    fs::set_permissions(&pages, original_permissions).unwrap();

    assert!(result.is_err());
    assert!(
        !sidecar_path.exists(),
        "the failed pair must leave the primary target absent"
    );
    assert_eq!(fs::read_to_string(&page_path).unwrap(), page_before);
    let trash = typed_trash_dir(&dir, TrashEntryKind::Conflict);
    assert!(
        fs::read_dir(trash).unwrap().flatten().any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .contains("failed-highlight-pair")
        }),
        "the exact new sidecar remains recoverable in conflict trash"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn write_highlights_preserves_malformed_utf8_sidecar() {
    let dir = scratch("highlights-malformed-edn");
    let g = Graph::open(&dir);
    let key = crate::pdf::asset_key("paper.pdf");
    let edn_path = dir.join("assets").join(format!("{key}.edn"));
    fs::create_dir_all(dir.join("assets")).unwrap();
    let malformed = "{:highlights [BROKEN :sentinel \"keep me\"";
    fs::write(&edn_path, malformed).unwrap();
    let h = mkhl("11111111-1111-1111-1111-111111111111", 1, Some("text"));

    let err = g
        .write_highlights("paper.pdf", "Paper", &[h], &[])
        .unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert_eq!(fs::read_to_string(&edn_path).unwrap(), malformed);
    assert!(!dir.join("pages").join(format!("hls__{key}.md")).exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn write_highlights_rejects_valid_map_with_trailing_sync_data() {
    let dir = scratch("highlights-trailing-edn");
    let g = Graph::open(&dir);
    let key = crate::pdf::asset_key("paper.pdf");
    let edn_path = dir.join("assets").join(format!("{key}.edn"));
    fs::create_dir_all(dir.join("assets")).unwrap();
    let malformed = "{:highlights [] :extra {}} TRAILING-SYNC-DATA";
    fs::write(&edn_path, malformed).unwrap();
    let h = mkhl("11111111-1111-1111-1111-111111111111", 1, Some("text"));

    let err = g
        .write_highlights("paper.pdf", "Paper", &[h], &[])
        .unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert_eq!(fs::read_to_string(&edn_path).unwrap(), malformed);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn write_highlights_migrates_legacy_key_forward() {
    // Old Tine wrote highlight files under a lowercase+underscore key
    // (`my_paper`); the OG-compatible key for "My Paper.pdf" is "My Paper". A
    // read must find the legacy file, and the next write must migrate the
    // artifacts to the new key (removing the stale legacy ones).
    let dir = scratch("hlmig");
    let pdf = "My Paper.pdf";
    let legacy_key = crate::pdf::legacy_asset_key(pdf); // "my_paper"
    let new_key = crate::pdf::asset_key(pdf); // "My Paper"
    assert_ne!(legacy_key, new_key);
    let assets = dir.join("assets");
    fs::create_dir_all(&assets).unwrap();
    let h1 = mkhl(
        "11111111-1111-1111-1111-111111111111",
        3,
        Some("legacy text"),
    );
    fs::write(
        assets.join(format!("{legacy_key}.edn")),
        crate::pdf::write_highlights(&[h1.clone()], ""),
    )
    .unwrap();
    let legacy_page = crate::pdf::hls_page_document(pdf, "My Paper", &[h1.clone()]);
    fs::write(
        dir.join("pages").join(format!("hls__{legacy_key}.md")),
        doc::serialize(&legacy_page),
    )
    .unwrap();

    let g = Graph::open(&dir);
    g.warm_cache();
    // Read-fallback: the legacy file is found under the new-key lookup.
    let read = g.read_highlights(pdf);
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].id, h1.id);

    // Write H1 + a newly-added H2 (editor baseline = [H1]).
    let h2 = mkhl("22222222-2222-2222-2222-222222222222", 4, Some("new text"));
    g.write_highlights(pdf, "My Paper", &[h1.clone(), h2.clone()], &[h1.id.clone()])
        .unwrap();

    // New-key artifacts exist with both highlights; the legacy ones are gone.
    let new_edn = assets.join(format!("{new_key}.edn"));
    assert!(new_edn.exists(), "new-key edn written");
    let migrated = crate::pdf::parse_highlights(&fs::read_to_string(&new_edn).unwrap());
    assert_eq!(migrated.len(), 2, "both highlights carried forward");
    assert!(
        dir.join("pages")
            .join(format!("hls__{new_key}.md"))
            .exists(),
        "new hls page"
    );
    assert!(
        !assets.join(format!("{legacy_key}.edn")).exists(),
        "legacy edn removed"
    );
    assert!(
        !dir.join("pages")
            .join(format!("hls__{legacy_key}.md"))
            .exists(),
        "legacy hls page removed"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn legacy_hls_migration_preserves_page_format_when_preference_changed() {
    let dir = scratch("hlmig-format");
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::create_dir_all(dir.join("assets")).unwrap();
    fs::write(
        dir.join("logseq").join("config.edn"),
        "{:preferred-format \"Org\"}\n",
    )
    .unwrap();
    let pdf = "My Paper.pdf";
    let legacy_key = crate::pdf::legacy_asset_key(pdf);
    let new_key = crate::pdf::asset_key(pdf);
    let h = mkhl("11111111-1111-1111-1111-111111111111", 3, Some("legacy"));
    fs::write(
        dir.join("assets").join(format!("{legacy_key}.edn")),
        crate::pdf::write_highlights(&[h.clone()], ""),
    )
    .unwrap();
    let mut legacy_page = crate::pdf::hls_page_document(pdf, "Paper", &[h.clone()]);
    legacy_page.roots[0]
        .children
        .push(DocBlock::new("private note"));
    fs::write(
        dir.join("pages").join(format!("hls__{legacy_key}.md")),
        doc::serialize(&legacy_page),
    )
    .unwrap();

    let g = Graph::open(&dir);
    g.write_highlights(pdf, "Paper", &[h.clone()], &[h.id.clone()])
        .unwrap();

    let migrated = dir.join("pages").join(format!("hls__{new_key}.md"));
    assert!(migrated.exists(), "legacy .md format should be retained");
    assert!(!dir
        .join("pages")
        .join(format!("hls__{new_key}.org"))
        .exists());
    assert!(fs::read_to_string(migrated)
        .unwrap()
        .contains("private note"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn write_highlights_does_not_migrate_legacy_key_used_by_another_pdf() {
    let dir = scratch("hl-legacy-collision");
    let assets = dir.join("assets");
    fs::create_dir_all(&assets).unwrap();

    let lower_pdf = "my_paper.pdf";
    let spaced_pdf = "My Paper.pdf";
    fs::write(assets.join(lower_pdf), b"lower pdf").unwrap();
    fs::write(assets.join(spaced_pdf), b"spaced pdf").unwrap();

    let lower_key = crate::pdf::asset_key(lower_pdf);
    let spaced_key = crate::pdf::asset_key(spaced_pdf);
    let spaced_legacy_key = crate::pdf::legacy_asset_key(spaced_pdf);
    assert_eq!(lower_key, spaced_legacy_key);
    assert_ne!(spaced_key, spaced_legacy_key);

    let lower_highlight = mkhl(
        "33333333-3333-3333-3333-333333333333",
        3,
        Some("lower pdf highlight"),
    );
    let lower_edn = crate::pdf::write_highlights(&[lower_highlight.clone()], "");
    let lower_edn_path = assets.join(format!("{lower_key}.edn"));
    fs::write(&lower_edn_path, &lower_edn).unwrap();

    let mut lower_page =
        crate::pdf::hls_page_document(lower_pdf, "Lower Paper", &[lower_highlight.clone()]);
    lower_page.roots[0]
        .children
        .push(DocBlock::new("lower pdf private note"));
    let lower_page_bytes = doc::serialize(&lower_page);
    let lower_page_path = dir
        .join("pages")
        .join(format!("{}.md", crate::pdf::hls_page_name(&lower_key)));
    fs::write(&lower_page_path, &lower_page_bytes).unwrap();

    let g = Graph::open(&dir);
    g.warm_cache();
    let spaced_highlight = mkhl(
        "44444444-4444-4444-4444-444444444444",
        4,
        Some("spaced pdf highlight"),
    );
    g.write_highlights(spaced_pdf, "My Paper", &[spaced_highlight], &[])
        .unwrap();

    assert!(
        lower_edn_path.exists(),
        "live colliding pdf edn must not be deleted"
    );
    assert_eq!(
        fs::read_to_string(&lower_edn_path).unwrap(),
        lower_edn,
        "live colliding pdf edn must remain byte-for-byte intact"
    );
    assert!(
        lower_page_path.exists(),
        "live colliding pdf hls page must not be deleted"
    );
    assert_eq!(
        fs::read_to_string(&lower_page_path).unwrap(),
        lower_page_bytes,
        "live colliding pdf hls page must remain byte-for-byte intact"
    );

    let spaced_edn_path = assets.join(format!("{spaced_key}.edn"));
    let spaced_edn = fs::read_to_string(&spaced_edn_path).unwrap();
    let spaced_highlights = crate::pdf::parse_highlights(&spaced_edn);
    assert_eq!(spaced_highlights.len(), 1);
    assert_eq!(
        spaced_highlights[0].id,
        "44444444-4444-4444-4444-444444444444"
    );

    let spaced_page_path = dir
        .join("pages")
        .join(format!("{}.md", crate::pdf::hls_page_name(&spaced_key)));
    let spaced_page = fs::read_to_string(&spaced_page_path).unwrap();
    assert!(
        !spaced_page.contains("lower pdf private note"),
        "colliding pdf note must not be merged into the spaced pdf hls page"
    );
    assert!(
        !spaced_page.contains(&lower_highlight.id),
        "colliding pdf highlight must not be merged into the spaced pdf hls page"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn deleting_highlight_write_is_not_seen_as_external() {
    // Repro for the "someone else edited the note" warning when deleting a
    // highlight while its hls__ page is open: the hls page write (and the
    // delete-rewrite) must be recognized as Tine's OWN write by the watcher,
    // not flagged as an external change.
    let dir = scratch("hldel");
    let g = Graph::open(&dir);
    g.warm_cache();
    let h1 = mkhl("aaaaaaaa-0000-0000-0000-000000000001", 1, Some("one"));
    let h2 = mkhl("bbbbbbbb-0000-0000-0000-000000000002", 2, Some("two"));
    let page_path = dir.join("pages").join("hls__paper.md");
    g.write_highlights("paper.pdf", "Paper", &[h1.clone(), h2.clone()], &[])
        .unwrap();
    assert!(
        g.sync_file(&page_path).is_none(),
        "initial highlight write looked external"
    );
    // Delete h2 (write just h1; baseline = both) — the rewrite must also be ours.
    g.write_highlights(
        "paper.pdf",
        "Paper",
        &[h1.clone()],
        &[h1.id.clone(), h2.id.clone()],
    )
    .unwrap();
    assert!(
        g.sync_file(&page_path).is_none(),
        "delete-rewrite looked external (false conflict)"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn write_pdf_area_image_uses_og_layout() {
    let dir = scratch("areaimg");
    let g = Graph::open(&dir);
    let rel = g
        .write_pdf_area_image("My Paper.pdf", 7, "abc-id", 1659920114630, &[1, 2, 3, 4])
        .unwrap();
    // OG layout: assets/<key>/<page>_<id>_<stamp>.png with the OG-compatible key.
    assert_eq!(rel, "My Paper/7_abc-id_1659920114630.png");
    let p = dir
        .join("assets")
        .join("My Paper")
        .join("7_abc-id_1659920114630.png");
    assert_eq!(fs::read(&p).unwrap(), vec![1, 2, 3, 4]);
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn write_pdf_area_image_rejects_nested_asset_symlink_escape() {
    use std::os::unix::fs::symlink;
    let dir = scratch("areaimg-nested-symlink");
    let outside = std::env::temp_dir().join(format!("tine-areaimg-outside-{}", std::process::id()));
    let _ = fs::remove_dir_all(&outside);
    fs::create_dir_all(&outside).unwrap();
    fs::create_dir_all(dir.join("assets")).unwrap();
    symlink(&outside, dir.join("assets").join("My Paper")).unwrap();
    let g = Graph::open(&dir);

    assert!(g
        .write_pdf_area_image("My Paper.pdf", 7, "abc-id", 1659920114630, &[1, 2, 3])
        .is_err());
    assert!(!outside.join("7_abc-id_1659920114630.png").exists());
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&outside);
}

fn jdto(name: &str) -> PageDto {
    PageDto {
        activation: None,
        name: name.into(),
        kind: PageKind::Journal,
        title: name.into(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: String::new(),
            raw: "hi".into(),
            collapsed: false,
            children: vec![],
            breadcrumb: vec![],
            ..Default::default()
        }],
        rev: None,
        format: Format::Md,
        read_only: false,
        path: String::new(),
        guide: false,
    }
}

#[test]
fn custom_journal_format_creates_in_user_format() {
    let dir = scratch("jfmt");
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq").join("config.edn"),
        "{:journal/file-name-format \"yyyy-MM-dd\"}\n",
    )
    .unwrap();
    let g = Graph::open(&dir);
    // A custom filename format now creates today's journal at the CORRECT path
    // (the user's format) — not a misplaced default `yyyy_MM_dd` duplicate.
    g.save_page(&jdto("Jun 24th, 2026"), None).unwrap();
    assert!(dir.join("journals").join("2026-06-24.md").exists());
    assert!(!dir.join("journals").join("2026_06_24.md").exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn custom_format_journal_files_load_and_display() {
    // THE reported bug: a graph whose journal files use a non-default format
    // must still load — the files are recognized and titled in the user's
    // page-title-format.
    let dir = scratch("jfmt-load");
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::create_dir_all(dir.join("journals")).unwrap();
    fs::write(
        dir.join("logseq").join("config.edn"),
        "{:journal/file-name-format \"dd-MM-yyyy\" :journal/page-title-format \"yyyy-MM-dd\"}\n",
    )
    .unwrap();
    // A real journal file in the user's dd-MM-yyyy filename format.
    fs::write(dir.join("journals").join("24-06-2026.md"), "- hi\n").unwrap();
    let g = Graph::open(&dir);
    let js = g.journals_desc();
    assert_eq!(
        js.len(),
        1,
        "custom-format journal must be recognized (was dropped before)"
    );
    assert_eq!(js[0].date_key, Some(20260624));
    assert_eq!(
        js[0].name, "2026-06-24",
        "title rendered in :journal/page-title-format"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn default_journal_format_creates_journal() {
    let dir = scratch("jfmt-default");
    // No config.edn → defaults → creation proceeds as before.
    let g = Graph::open(&dir);
    g.save_page(&jdto("Jun 24th, 2026"), None).unwrap();
    assert!(dir.join("journals").join("2026_06_24.md").exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn advanced_query_runs_supported_subset_flags_rest() {
    let dir = scratch("adv");
    fs::write(
        dir.join("journals").join("2026_06_20.md"),
        "- TODO ship it\n- DONE done\n",
    )
    .unwrap();
    fs::write(dir.join("pages").join("Note.md"), "- TODO not a journal\n").unwrap();
    let g = ready_graph(&dir);
    // (task ?b #{"TODO"}) maps to the existing Task predicate.
    let r = when_ready(|| {
        g.run_advanced_query(r#"[:find (pull ?b [*]) :where (task ?b #{"TODO"})]"#, None)
    });
    assert!(r.supported);
    assert!(r.ran.contains(&"task".to_string()));
    let total: usize = r.groups.iter().map(|grp| grp.blocks.len()).sum();
    assert_eq!(total, 2, "both TODO blocks match");
    // A clause outside the subset (a raw [?e :a ?v] join) → nothing supported.
    let u = when_ready(|| g.run_advanced_query("[:find ?b :where [?b :block/foo ?v]]", None));
    assert!(!u.supported);
    assert!(u.groups.is_empty());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn advanced_current_page_input_filters_real_graph_blocks() {
    let dir = scratch("advanced-current-page");
    fs::write(dir.join("pages/Focus A.md"), "- own A\n").unwrap();
    fs::write(dir.join("pages/Focus B.md"), "- own B\n").unwrap();
    fs::write(
        dir.join("pages/Source.md"),
        "- TODO pinned [[Focus A]]\n- TODO pinned [[Focus B]]\n- DONE [[Focus A]]\n",
    )
    .unwrap();
    let graph = ready_graph(&dir);
    let query = r#"[:find (pull ?b [*])
                        :in $ ?current-page
                        :where
                        [?p :block/name ?current-page]
                        [?b :block/refs ?p]
                        (task ?b #{"TODO"})]
                       :inputs [:current-page]"#;

    let result = when_ready(|| graph.run_advanced_query(query, Some("Focus A")));
    assert!(result.supported, "ignored={:?}", result.ignored);
    assert!(result.ignored.is_empty(), "{:?}", result.ignored);
    assert_eq!(result.ran, vec!["current-page-ref", "task"]);
    let raws = result
        .groups
        .iter()
        .flat_map(|group| group.blocks.iter().map(|block| block.raw.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(raws, vec!["TODO pinned [[Focus A]]"]);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn advanced_query_covers_widened_clause_subset() {
    // 1c: the advanced (datalog) parser maps the same heads the simple DSL
    // supports — page / namespace / page-tags / scheduled / deadline / journal
    // — not just the original task/priority/page-ref/property/between set.
    let dir = scratch("adv-wide");
    fs::write(
        dir.join("journals").join("2026_06_20.md"),
        "- TODO ship it\n  SCHEDULED: <2026-06-25 Thu>\n- pay rent\n  DEADLINE: <2026-06-30 Tue>\n",
    )
    .unwrap();
    fs::write(
        dir.join("pages").join("Proj.md"),
        "tags:: work, urgent\n\n- a task on a named page\n",
    )
    .unwrap();
    // Default file-name format is Legacy (`%2F`), so encode the namespace slash.
    fs::write(dir.join("pages").join("Proj%2FSub.md"), "- nested note\n").unwrap();
    let g = ready_graph(&dir);

    let count = |src: &str| -> usize {
        let r = when_ready(|| g.run_advanced_query(src, None));
        assert!(r.supported, "expected supported: {src} (ran={:?})", r.ran);
        r.groups.iter().map(|grp| grp.blocks.len()).sum()
    };

    // (scheduled) / (deadline) map to the planning predicates.
    assert_eq!(count("[:find (pull ?b [*]) :where (scheduled ?b)]"), 1);
    assert_eq!(count("[:find (pull ?b [*]) :where (deadline ?b)]"), 1);
    // (journal) restricts to blocks on journal pages.
    assert_eq!(count("[:find (pull ?b [*]) :where (journal ?b)]"), 2);
    // (page "Name") pins to one page.
    assert_eq!(count(r#"[:find (pull ?b [*]) :where (page ?b "Proj")]"#), 1);
    // (namespace "Proj") matches pages under the namespace.
    assert_eq!(
        count(r#"[:find (pull ?b [*]) :where (namespace ?b "Proj")]"#),
        1
    );
    // (page-tags "work") matches the tags:: page-property.
    assert_eq!(
        count(r#"[:find (pull ?b [*]) :where (page-tags ?b "work")]"#),
        1
    );
    // (between scheduled …) is now field-aware, not hardwired to journal-day.
    assert_eq!(
        count(r#"[:find (pull ?b [*]) :where (between scheduled ?b "2026-06-24" "2026-06-26")]"#),
        1
    );

    // Unknown heads still land in `ignored`, never guessed.
    let r = when_ready(|| g.run_advanced_query("[:find ?b :where (bogus ?b)]", None));
    assert!(r.ignored.contains(&"bogus".to_string()));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn advanced_query_skeleton_ignores_comment_hints() {
    // 1b: the "switch to advanced" skeleton lists supported heads as `;;` EDN
    // comments. Those example clauses must NOT be parsed as real filters — only
    // the single active clause runs. (Regression: scan_groups now skips `; …`.)
    let dir = scratch("adv-skel");
    fs::write(
        dir.join("journals").join("2026_06_20.md"),
        "- TODO ship it\n- DOING wire it\n- DONE done\n",
    )
    .unwrap();
    let g = ready_graph(&dir);
    let skeleton = "[:find (pull ?b [*])\n \
             :where\n \
             ;; supported: (priority ?b \"A\") (page-ref ?b \"Nope\") (property ?b :k \"v\")\n \
             ;; (scheduled ?b) (deadline ?b) (page ?b \"Nowhere\")\n \
             (task ?b #{\"TODO\" \"DOING\"})]";
    let r = when_ready(|| g.run_advanced_query(skeleton, None));
    assert!(r.supported, "ran: {:?} ignored: {:?}", r.ran, r.ignored);
    // Only the task clause ran — the commented priority/page-ref/etc. did not.
    assert_eq!(r.ran, vec!["task".to_string()]);
    assert!(
        r.ignored.is_empty(),
        "no clause should be ignored: {:?}",
        r.ignored
    );
    let total: usize = r.groups.iter().map(|grp| grp.blocks.len()).sum();
    assert_eq!(
        total, 2,
        "TODO + DOING match; the commented (page-ref \"Nope\") is inert"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn persisted_query_sources_are_refused_before_projection_or_parser_recursion() {
    let dir = scratch("query-source-recursion-bound");
    fs::write(dir.join("pages").join("P.md"), "- TODO ship\n").unwrap();
    let g = ready_graph(&dir);
    let reads_before = g.direct_projection_statement_reads_test();

    // This is the graph-authored shape that previously overflowed the Rust
    // stack when a persisted query macro rendered. Keep it below the byte
    // ceiling so the independent nesting guard is the reason it fails shut.
    let nested = format!("{}(task TODO){}", "(and ".repeat(1_000), ")".repeat(1_000));
    assert!(crate::query::query_source_within_limit(&nested));
    assert!(!crate::query::query_nesting_within_limit(&nested));
    let simple = g
        .run_query_bounded(&nested, 20_000, 32 * 1024 * 1024)
        .expect("a refused query source is answered before any dispatch");
    assert!(simple.groups.is_empty());

    let advanced = format!("[:find (pull ?b [*]) :where {nested}]");
    let result = g
        .run_advanced_query(&advanced, None)
        .expect("a refused query source is answered before any dispatch");
    assert!(!result.supported);
    assert_eq!(result.ignored, vec!["query-nesting-too-deep"]);
    assert_eq!(
        g.direct_projection_statement_reads_test(),
        reads_before,
        "refused simple and advanced sources must not execute a projection statement"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Ask the production advanced route for its acquired pre-view rows. The source
/// report remains outside this shared value and is checked through the public
/// route by the caller.
fn advanced_query_pre_view_rows(
    graph: &Graph,
    query_src: &str,
    max_rows: usize,
    max_bytes: usize,
) -> Vec<RefGroup> {
    let crate::query::ResolvedAdvanced::Executable { query, today, .. } =
        crate::query::resolve_advanced_source(query_src, None)
    else {
        panic!("the fixture advanced query resolves");
    };
    let query = crate::query::block_anchored_query(&query);
    when_ready(|| {
        graph.direct_simple_query_pre_view(
            &query,
            today,
            max_rows,
            max_bytes,
            crate::query::ConstructionProfile::default(),
            None,
        )
    })
    .groups
}

fn simple_query_pre_view_rows_at(
    graph: &Graph,
    query_src: &str,
    today: crate::date::JournalDate,
    max_rows: usize,
    max_bytes: usize,
) -> Vec<RefGroup> {
    let (query, view) = crate::query::parse_query_source(query_src, today);
    let query = crate::query::block_anchored_query(&query);
    let profile = crate::query::ConstructionProfile::from_view(&view);
    when_ready(|| {
        graph.direct_simple_query_pre_view(&query, today, max_rows, max_bytes, profile, None)
    })
    .groups
}

#[test]
fn repeated_query_executes_again_at_the_same_or_a_later_day() {
    let dir = scratch("query-repeat-execution-day");
    fs::write(dir.join("pages").join("Tasks.md"), "- TODO ship\n").unwrap();
    let graph = ready_graph(&dir);
    let today = crate::date::JournalDate::today();
    let first = simple_query_pre_view_rows_at(&graph, "(task TODO)", today, usize::MAX, usize::MAX);
    let after_first = graph.direct_projection_statement_reads_test();
    let same_day =
        simple_query_pre_view_rows_at(&graph, "(task TODO)", today, usize::MAX, usize::MAX);
    assert_eq!(
        graph.direct_projection_statement_reads_test(),
        after_first + 1,
        "the repeated valid query executes SQL again"
    );
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&same_day).unwrap(),
        "the repeated read returns the same ordered rows"
    );

    let next_day = simple_query_pre_view_rows_at(
        &graph,
        "(task TODO)",
        today.add_days(1),
        usize::MAX,
        usize::MAX,
    );
    assert_eq!(
        graph.direct_projection_statement_reads_test(),
        after_first + 2,
        "the later-day request also executes SQL"
    );
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&next_day).unwrap(),
        "a day-insensitive query keeps the same answer"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn advanced_query_reexecutes_and_observes_every_edit() {
    let dir = scratch("adv-repeat");
    fs::write(dir.join("pages").join("P.md"), "- TODO ship\n").unwrap();
    fs::write(
        dir.join("pages").join("Notes.md"),
        "alias:: Scratch\n- ordinary note\n",
    )
    .unwrap();
    let g = ready_graph(&dir);
    let q = r#"[:find (pull ?b [*]) :where (task ?b #{"TODO"})]"#;

    let first_result = when_ready(|| g.run_advanced_query_cached(q, None));
    let first = advanced_query_pre_view_rows(&g, q, usize::MAX, usize::MAX);
    let after_first = g.direct_projection_statement_reads_test();
    let second = advanced_query_pre_view_rows(&g, q, usize::MAX, usize::MAX);
    assert_eq!(g.direct_projection_statement_reads_test(), after_first + 1);
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&second).unwrap(),
        "repeated SQL reads return the same ordered rows"
    );
    let simple = when_ready(|| g.run_query_bounded("(task TODO)", usize::MAX, usize::MAX));
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(simple.groups.as_ref()).unwrap()
    );
    assert_eq!(first_result.groups.len(), 1);
    assert!(first_result.supported);
    assert_eq!(first_result.ran, vec!["task".to_string()]);
    assert!(first_result.ignored.is_empty());
    assert_eq!(
        serde_json::to_vec(&first_result.groups).unwrap(),
        serde_json::to_vec(&first).unwrap(),
        "the public route attaches its source report to the shared rows"
    );
    let bounded_first = advanced_query_pre_view_rows(&g, q, 20_000, 32 * 1024 * 1024);
    let after_bounded_first = g.direct_projection_statement_reads_test();
    let bounded_second = advanced_query_pre_view_rows(&g, q, 20_000, 32 * 1024 * 1024);
    assert_eq!(
        g.direct_projection_statement_reads_test(),
        after_bounded_first + 1
    );
    assert_eq!(
        serde_json::to_vec(&bounded_first).unwrap(),
        serde_json::to_vec(&bounded_second).unwrap()
    );

    let mut notes = g.load_named("Notes", PageKind::Page).unwrap().unwrap();
    notes.blocks[0].raw = "still unrelated".into();
    g.save_page(&notes, notes.rev.as_deref()).unwrap();
    let after_unrelated = advanced_query_pre_view_rows(&g, q, usize::MAX, usize::MAX);
    let bounded_after_unrelated = advanced_query_pre_view_rows(&g, q, 20_000, 32 * 1024 * 1024);
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&after_unrelated).unwrap(),
        "the unrelated edit leaves the ordered answer unchanged"
    );
    assert_eq!(
        serde_json::to_vec(&bounded_first).unwrap(),
        serde_json::to_vec(&bounded_after_unrelated).unwrap()
    );

    let mut notes = g.load_named("Notes", PageKind::Page).unwrap().unwrap();
    notes.pre_block = Some("alias:: Renamed Scratch\n".into());
    g.save_page(&notes, notes.rev.as_deref()).unwrap();
    let after_alias_change = advanced_query_pre_view_rows(&g, q, usize::MAX, usize::MAX);
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&after_alias_change).unwrap(),
        "the task answer is unchanged by an alias on an unrelated page"
    );

    let mut dto = g.load_named("P", PageKind::Page).unwrap().unwrap();
    dto.blocks[0].raw = dto.blocks[0].raw.replace("TODO", "DONE");
    let before_affected_edit = g.cache_generation();
    g.save_page(&dto, dto.rev.as_deref()).unwrap();
    when_current(&g, before_affected_edit);

    let third_result = when_ready(|| g.run_advanced_query_cached(q, None));
    let third = advanced_query_pre_view_rows(&g, q, usize::MAX, usize::MAX);
    let bounded_after_affected = advanced_query_pre_view_rows(&g, q, 20_000, 32 * 1024 * 1024);
    assert!(third.is_empty());
    assert!(bounded_after_affected.is_empty());
    assert!(third_result.groups.is_empty());
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(windows)]
#[test]
fn windows_first_save_and_ordinary_rename_preserve_exact_projection() {
    let dir = scratch("windows-first-save-ordinary-rename");
    let graph = Graph::open(&dir);
    let page = markdown_page_dto("Original", "Original", "- first save bytes\n").unwrap();

    graph.save_page(&page, None).unwrap();

    let original = dir.join("pages/Original.md");
    let renamed = dir.join("pages/Renamed.md");
    assert_eq!(fs::read(&original).unwrap(), b"- first save bytes\n");
    assert!(!renamed.exists());

    graph.rename_page("Original", "Renamed").unwrap();

    assert!(!original.exists());
    assert_eq!(fs::read(&renamed).unwrap(), b"- first save bytes\n");
    let page_names = fs::read_dir(dir.join("pages"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(page_names, [std::ffi::OsString::from("Renamed.md")]);
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(windows)]
#[test]
fn windows_directory_durability_limit_does_not_block_save_or_rename() {
    let dir = scratch("windows-directory-flush-save-rename");
    let original = dir.join("pages/Original.md");
    fs::write(&original, "- before\n").unwrap();
    let graph = Graph::open(&dir);

    let mut page = graph
        .load_named("Original", PageKind::Page)
        .unwrap()
        .unwrap();
    page.blocks[0].raw = "after".into();
    graph.save_page(&page, page.rev.as_deref()).unwrap();
    assert_eq!(fs::read(&original).unwrap(), b"- after\n");
    assert!(!dir.join("pages/Renamed.md").exists());

    graph.rename_page("Original", "Renamed").unwrap();

    assert!(!original.exists());
    assert_eq!(
        fs::read_to_string(dir.join("pages/Renamed.md")).unwrap(),
        "- after\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn bounded_query_reexecutes_and_observes_every_edit() {
    let dir = scratch("bounded-query-repeat");
    fs::write(dir.join("pages").join("Tasks.md"), "- TODO ship\n").unwrap();
    fs::write(
        dir.join("pages").join("Notes.md"),
        "alias:: Scratch\n- ordinary note\n",
    )
    .unwrap();
    let g = ready_graph(&dir);

    let todo_tasks = || when_ready(|| g.run_query_bounded("(task TODO)", 20_000, 32 * 1024 * 1024));
    let first = todo_tasks();
    let after_first = g.direct_projection_statement_reads_test();
    let second = todo_tasks();
    assert_eq!(g.direct_projection_statement_reads_test(), after_first + 1);
    assert_eq!(first.total, second.total);
    assert_eq!(first.exceeded, second.exceeded);
    assert_eq!(
        serde_json::to_vec(first.groups.as_ref()).unwrap(),
        serde_json::to_vec(second.groups.as_ref()).unwrap()
    );

    let mut notes = g.load_named("Notes", PageKind::Page).unwrap().unwrap();
    notes.blocks[0].raw = "still an ordinary note".into();
    g.save_page(&notes, notes.rev.as_deref()).unwrap();
    // This assertion deliberately observes the committed post-edit image.
    wait_for_direct_query_projection(&g);
    let after_unrelated = todo_tasks();
    assert_eq!(
        serde_json::to_vec(first.groups.as_ref()).unwrap(),
        serde_json::to_vec(after_unrelated.groups.as_ref()).unwrap(),
        "the unrelated edit leaves the meaningful answer unchanged"
    );
    assert_eq!(first.total, after_unrelated.total);
    assert_eq!(first.exceeded, after_unrelated.exceeded);

    let mut tasks = g.load_named("Tasks", PageKind::Page).unwrap().unwrap();
    tasks.blocks[0].raw = "DONE ship".into();
    g.save_page(&tasks, tasks.rev.as_deref()).unwrap();
    // This assertion deliberately observes the committed post-edit image.
    wait_for_direct_query_projection(&g);
    let after_affected = todo_tasks();
    assert!(after_affected.groups.is_empty());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn every_query_request_executes_again_and_reads_the_current_sql_image() {
    let dir = scratch("repeat-two-queries");
    fs::write(
        dir.join("pages").join("Roadmap.md"),
        "tags:: work\n- TODO ship\n",
    )
    .unwrap();
    fs::write(dir.join("pages").join("Errand.md"), "- TODO buy milk\n").unwrap();
    let g = ready_graph(&dir);

    let tagged =
        || when_ready(|| g.run_query_bounded("(page-tags work)", 20_000, 32 * 1024 * 1024));
    let tasks = || when_ready(|| g.run_query_bounded("(task TODO)", 20_000, 32 * 1024 * 1024));

    let tagged_first = tagged();
    let tasks_first = tasks();
    assert_eq!(tagged_first.groups.len(), 1, "one tagged page");
    assert_eq!(tasks_first.groups.len(), 2, "both pages carry a TODO");
    let before_repeats = g.direct_projection_statement_reads_test();
    let tagged_repeat = tagged();
    let tasks_repeat = tasks();
    assert_eq!(
        g.direct_projection_statement_reads_test(),
        before_repeats + 2
    );
    assert_eq!(
        serde_json::to_vec(tagged_first.groups.as_ref()).unwrap(),
        serde_json::to_vec(tagged_repeat.groups.as_ref()).unwrap()
    );
    assert_eq!(
        serde_json::to_vec(tasks_first.groups.as_ref()).unwrap(),
        serde_json::to_vec(tasks_repeat.groups.as_ref()).unwrap()
    );

    let mut errand = g.load_named("Errand", PageKind::Page).unwrap().unwrap();
    errand.blocks[0].raw = "DONE buy milk".into();
    g.save_page(&errand, errand.rev.as_deref()).unwrap();
    // This assertion deliberately observes the committed post-edit image.
    wait_for_direct_query_projection(&g);

    let tagged_after = tagged();
    let tasks_after = tasks();
    assert_eq!(
        serde_json::to_vec(tagged_first.groups.as_ref()).unwrap(),
        serde_json::to_vec(tagged_after.groups.as_ref()).unwrap(),
        "the page-tags answer remains meaningful-equal across the irrelevant edit"
    );
    assert_eq!(tagged_first.total, tagged_after.total);
    assert_eq!(tagged_first.exceeded, tagged_after.exceeded);
    assert_eq!(tasks_after.groups.len(), 1, "one TODO survives the edit");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn bounded_reference_memos_survive_unrelated_edits_and_recompute_all_families() {
    const TARGET: &str = "12345678-1234-1234-1234-123456789abc";
    let dir = scratch("bounded-reference-scoped-memos");
    fs::write(
        dir.join("pages").join("Referrer.md"),
        format!("- See [[Target]], plain Target, and (({TARGET}))\n"),
    )
    .unwrap();
    fs::write(dir.join("pages").join("Target.md"), "- target page\n").unwrap();
    fs::write(
        dir.join("pages").join("Notes.md"),
        "alias:: Scratch\n- ordinary note\n",
    )
    .unwrap();
    let g = Graph::open(&dir);
    g.warm_cache();

    let first_block = g.block_referrers_bounded(TARGET, 20_000, 32 * 1024 * 1024);
    let first_backlink = g.backlinks_bounded("Target", 20_000, 32 * 1024 * 1024);
    let first_unlinked = g.unlinked_refs_bounded("Target", 20_000, 32 * 1024 * 1024);
    assert_eq!(first_block.total, 1);
    assert_eq!(first_backlink.total, 1);
    assert_eq!(first_unlinked.total, 1);

    let mut notes = g.load_named("Notes", PageKind::Page).unwrap().unwrap();
    notes.blocks[0].raw = "still unrelated".into();
    g.save_page(&notes, notes.rev.as_deref()).unwrap();
    let after_block = g.block_referrers_bounded(TARGET, 20_000, 32 * 1024 * 1024);
    let after_backlink = g.backlinks_bounded("Target", 20_000, 32 * 1024 * 1024);
    let after_unlinked = g.unlinked_refs_bounded("Target", 20_000, 32 * 1024 * 1024);
    assert!(Arc::ptr_eq(&first_block.groups, &after_block.groups));
    assert!(Arc::ptr_eq(&first_backlink.groups, &after_backlink.groups));
    assert!(Arc::ptr_eq(&first_unlinked.groups, &after_unlinked.groups));

    let mut referrer = g.load_named("Referrer", PageKind::Page).unwrap().unwrap();
    referrer.blocks[0].raw = "No longer a referrer".into();
    g.save_page(&referrer, referrer.rev.as_deref()).unwrap();
    let affected_block = g.block_referrers_bounded(TARGET, 20_000, 32 * 1024 * 1024);
    let affected_backlink = g.backlinks_bounded("Target", 20_000, 32 * 1024 * 1024);
    let affected_unlinked = g.unlinked_refs_bounded("Target", 20_000, 32 * 1024 * 1024);
    assert!(!Arc::ptr_eq(&first_block.groups, &affected_block.groups));
    assert!(!Arc::ptr_eq(
        &first_backlink.groups,
        &affected_backlink.groups
    ));
    assert!(!Arc::ptr_eq(
        &first_unlinked.groups,
        &affected_unlinked.groups
    ));
    assert_eq!(affected_block.total, 0);
    assert_eq!(affected_backlink.total, 0);
    assert_eq!(affected_unlinked.total, 0);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn bounded_unlinked_reference_memo_keeps_interactive_and_exhaustive_membership_distinct() {
    let dir = scratch("bounded-unlinked-memo-mode");
    let window = crate::query::candidate::INTERACTIVE_VERIFIED_WINDOW;
    let match_count = window + 25;
    let mut source = String::new();
    for ordinal in 0..match_count {
        source.push_str(&format!("- target occurrence {ordinal}\n"));
    }
    fs::write(dir.join("pages").join("Source.md"), source).unwrap();
    fs::write(dir.join("pages").join("Target.md"), "- owner\n").unwrap();
    let g = ready_graph(&dir);
    let generation = g.cache_generation();
    let limits = (match_count + 10, 16 * 1024 * 1024);
    let membership = |groups: &[RefGroup]| {
        groups
            .iter()
            .flat_map(|group| group.blocks.iter().map(|block| block.raw.clone()))
            .collect::<std::collections::BTreeSet<_>>()
    };

    let exhaustive_expected = crate::query::unlinked_refs_bounded(&g, "Target", limits.0, limits.1);
    let interactive_expected =
        crate::query::unlinked_refs_bounded_indexed(&g, "Target", limits.0, limits.1)
            .expect("the ready interactive reference route answers");
    assert_eq!(exhaustive_expected.total, match_count);
    assert_eq!(interactive_expected.total, window);
    let exhaustive_expected = membership(&exhaustive_expected.groups);
    let interactive_expected = membership(&interactive_expected.groups);

    let interactive_first = g
        .unlinked_refs_bounded_indexed("Target", limits.0, limits.1)
        .expect("interactive first");
    assert_eq!(interactive_first.total, window);
    assert_eq!(
        membership(interactive_first.groups.as_ref()),
        interactive_expected
    );
    let exhaustive_second = g.unlinked_refs_bounded("Target", limits.0, limits.1);
    assert_eq!(exhaustive_second.total, match_count);
    assert_eq!(
        membership(exhaustive_second.groups.as_ref()),
        exhaustive_expected,
        "an Interactive memo entry must not truncate the Exhaustive route"
    );

    // Exercise the reverse order without changing the graph generation,
    // target, or limits. This clears only the existing derived memo, not any
    // source or projection state.
    *g.derived_cache.write().unwrap() = None;
    assert_eq!(g.cache_generation(), generation);
    let exhaustive_first = g.unlinked_refs_bounded("Target", limits.0, limits.1);
    assert_eq!(exhaustive_first.total, match_count);
    assert_eq!(
        membership(exhaustive_first.groups.as_ref()),
        exhaustive_expected
    );
    let interactive_second = g
        .unlinked_refs_bounded_indexed("Target", limits.0, limits.1)
        .expect("interactive second");
    assert_eq!(interactive_second.total, window);
    assert_eq!(
        membership(interactive_second.groups.as_ref()),
        interactive_expected,
        "an Exhaustive memo entry must not widen the Interactive route"
    );
    assert_eq!(g.cache_generation(), generation);
    let _ = fs::remove_dir_all(&dir);
}

/// Deterministically models the ready-race branch in
/// `reference_candidate_pages_indexed`: the Interactive lookup declined, but
/// its existing Exhaustive SQL fallback succeeded. All other graph behavior
/// stays on the real implementation.
struct ExhaustiveReferenceCandidateGraph<'a>(&'a Graph);

impl crate::query::graph::QueryGraph for ExhaustiveReferenceCandidateGraph<'_> {
    fn with_pages<T>(&self, f: impl FnOnce(&[(PageEntry, Arc<Document>)]) -> T) -> T {
        self.0.with_pages(f)
    }

    fn page_aliases(&self) -> Vec<(String, String)> {
        self.0.page_aliases()
    }

    fn block_page_hint(&self, uuid: &str) -> Option<String> {
        self.0.block_page_hint(uuid)
    }

    fn reference_candidate_pages(
        &self,
        names_norm: &[String],
        self_page: &str,
        kind: ReferenceKind,
    ) -> ReferenceCandidatePages {
        self.0
            .reference_candidate_pages(names_norm, self_page, kind)
    }

    fn reference_candidate_pages_indexed(
        &self,
        names_norm: &[String],
        self_page: &str,
        kind: ReferenceKind,
    ) -> Result<ReferenceCandidatePages, crate::query::QueryExecutionError> {
        let candidates = self
            .0
            .reference_candidate_pages(names_norm, self_page, kind);
        assert!(
            candidates.indexed,
            "the deterministic fallback must come from Exhaustive SQL"
        );
        assert!(
            candidates.page_owners.is_none(),
            "Exhaustive candidates must not claim Interactive provenance"
        );
        Ok(candidates)
    }

    fn backlink_filter_scope(
        &self,
        target: &str,
        requested_pages: &[(PageKind, String)],
    ) -> Result<crate::query::BacklinkFilterScope, crate::query::QueryExecutionError> {
        self.0.backlink_filter_scope(target, requested_pages)
    }

    fn direct_projection_block_referrer_candidate_pages(
        &self,
        uuid: &str,
    ) -> Option<Vec<(PageEntry, Arc<Document>)>> {
        self.0
            .direct_projection_block_referrer_candidate_pages(uuid)
    }

    fn list_pages(&self) -> Vec<PageEntry> {
        self.0.list_pages()
    }

    fn page_aliases_with_owners(&self) -> Vec<(String, String, String)> {
        self.0.page_aliases_with_owners()
    }

    fn referenced_page_names(&self) -> Vec<String> {
        self.0.referenced_page_names()
    }

    fn reference_real_page_names(&self) -> Option<crate::query::RealPageNames> {
        self.0.reference_real_page_names()
    }

    fn direct_ir_query_result(
        &self,
        resolved: &crate::query::ResolvedQuery,
        view: &crate::query::ir::ViewSettings,
        bounds: crate::query::ir::Bounds,
    ) -> Result<crate::query::ir::QueryResult, crate::query::QueryExecutionError> {
        self.0.direct_ir_query_result(resolved, view, bounds)
    }

    fn direct_ir_explain_empty(
        &self,
        resolved: &crate::query::ResolvedQuery,
        view: &crate::query::ir::ViewSettings,
        bounds: crate::query::ir::Bounds,
    ) -> Result<crate::query::ir::ExplainEmptyResult, crate::query::QueryExecutionError> {
        self.0.direct_ir_explain_empty(resolved, view, bounds)
    }

    fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<RefGroup>, crate::query::QueryExecutionError> {
        self.0.search(query, limit)
    }

    fn direct_projection_recover_after_failed_read(&self) {
        crate::direct_projection::recover_until_ready(self.0);
    }

    fn cache_generation(&self) -> u64 {
        self.0.cache_generation()
    }

    fn config(&self) -> &Config {
        &self.0.config
    }

    fn direct_projection_test(&self) -> Option<Arc<crate::direct_projection::DirectProjection>> {
        self.0.direct_projection_test()
    }

    fn direct_projection_ready_test(&self) -> bool {
        self.0.direct_projection_ready_test()
    }
}

#[test]
fn indexed_exhaustive_fallback_is_not_eligible_for_interactive_memo() {
    let dir = scratch("bounded-unlinked-memo-exhaustive-fallback");
    fs::write(dir.join("pages").join("Source.md"), "- target occurrence\n").unwrap();
    fs::write(dir.join("pages").join("Target.md"), "- owner\n").unwrap();
    let graph = ready_graph(&dir);

    let interactive = crate::query::unlinked_refs_bounded_indexed_with_source(
        &graph,
        "Target",
        20_000,
        32 * 1024 * 1024,
    )
    .expect("the verified Interactive candidates answer normally");
    assert_eq!(interactive.groups.total, 1);
    assert!(
        interactive.memo_eligible,
        "verified Interactive provenance remains eligible for the UI memo"
    );

    let answer = crate::query::unlinked_refs_bounded_indexed_with_source(
        &ExhaustiveReferenceCandidateGraph(&graph),
        "Target",
        20_000,
        32 * 1024 * 1024,
    )
    .expect("the indexed Exhaustive fallback answers normally");

    assert_eq!(answer.groups.total, 1);
    assert!(
        !answer.memo_eligible,
        "indexed Exhaustive provenance must not enter the Interactive memo"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn indexed_unlinked_fallback_is_not_reused_after_same_generation_becomes_ready() {
    let dir = scratch("bounded-unlinked-memo-readiness");
    let window = crate::query::candidate::INTERACTIVE_VERIFIED_WINDOW;
    let match_count = window + 25;
    let mut source = String::new();
    for ordinal in 0..match_count {
        source.push_str(&format!("- target occurrence {ordinal}\n"));
    }
    fs::write(dir.join("pages").join("Source.md"), source).unwrap();
    fs::write(dir.join("pages").join("Target.md"), "- owner\n").unwrap();

    let g = Graph::open(&dir);
    g.warm_cache();
    let generation = g.cache_generation();
    let limits = (match_count + 10, 16 * 1024 * 1024);
    let fallback = g
        .unlinked_refs_bounded_indexed("Target", limits.0, limits.1)
        .expect("no projection keeps the established parser fallback");
    assert_eq!(fallback.total, match_count);
    let fallback_membership = fallback
        .groups
        .iter()
        .flat_map(|group| group.blocks.iter().map(|block| block.raw.clone()))
        .collect::<std::collections::BTreeSet<_>>();

    g.attach_direct_projection(dir.join("private/projection.sqlite"))
        .unwrap();
    wait_for_direct_query_projection(&g);
    assert_eq!(
        g.cache_generation(),
        generation,
        "projection readiness alone must not need a source generation change"
    );
    let indexed = g
        .unlinked_refs_bounded_indexed("Target", limits.0, limits.1)
        .expect("the ready projection answers the same request");
    assert_eq!(
        indexed.total, window,
        "the parser fallback must not become the ready Interactive memo answer"
    );
    let indexed_membership = indexed
        .groups
        .iter()
        .flat_map(|group| group.blocks.iter().map(|block| block.raw.clone()))
        .collect::<std::collections::BTreeSet<_>>();
    assert!(indexed_membership.is_subset(&fallback_membership));
    assert_ne!(indexed_membership, fallback_membership);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn scoped_reference_invalidation_uses_real_page_before_colliding_alias() {
    let dir = scratch("reference-invalidation-real-page-first");
    fs::write(dir.join("pages").join("X.md"), "alias:: Q\n\n- real X\n").unwrap();
    fs::write(
        dir.join("pages").join("Y.md"),
        "alias:: X\n\n- alias owner\n",
    )
    .unwrap();
    fs::write(dir.join("pages").join("Source.md"), "- unrelated\n").unwrap();
    let g = Graph::open(&dir);
    g.warm_cache();

    let first_linked = g.backlinks("X");
    let first_unlinked = g.unlinked_refs("X");
    assert!(!first_linked.iter().any(|group| group.page == "Source"));
    assert!(!first_unlinked.iter().any(|group| group.page == "Source"));

    let mut source = g.load_named("Source", PageKind::Page).unwrap().unwrap();
    source.blocks[0].raw = "Q and [[Q]]".into();
    g.save_page(&source, source.rev.as_deref()).unwrap();

    let linked = g.backlinks("X");
    let unlinked = g.unlinked_refs("X");
    assert!(!Arc::ptr_eq(&first_linked, &linked));
    assert!(!Arc::ptr_eq(&first_unlinked, &unlinked));
    assert!(linked.iter().any(|group| group.page == "Source"));
    assert!(unlinked.iter().any(|group| group.page == "Source"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn nfd_alias_resolves_and_canonical_equivalent_alias_cannot_shadow_real_page() {
    let dir = scratch("nfd-alias-resolution");
    fs::write(
        dir.join("pages").join("Owner.md"),
        "alias:: Re\u{301}sume\u{301}\n\n- owner\n",
    )
    .unwrap();
    fs::write(
        dir.join("pages").join("Shadow.md"),
        "alias:: Cafe\u{301}\n\n- shadow\n",
    )
    .unwrap();
    fs::write(dir.join("pages").join("Café.md"), "- real page\n").unwrap();
    let g = Graph::open(&dir);
    g.warm_cache();

    assert_eq!(
        g.load_named("Re\u{301}sume\u{301}", PageKind::Page)
            .unwrap()
            .unwrap()
            .name,
        "Owner"
    );
    assert_eq!(
        g.load_named("Cafe\u{301}", PageKind::Page)
            .unwrap()
            .unwrap()
            .name,
        "Café",
        "the canonically equivalent real title must win before alias fallback"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn overflowed_bounded_query_reexecutes_when_an_omitted_match_stops_matching() {
    let dir = scratch("bounded-overflow-negative-transition");
    fs::write(dir.join("pages").join("A.md"), "- TODO first\n").unwrap();
    fs::write(dir.join("pages").join("B.md"), "- TODO second\n").unwrap();
    fs::write(dir.join("pages").join("Notes.md"), "- unrelated\n").unwrap();
    let g = ready_graph(&dir);

    let tasks = || when_ready(|| g.run_query_bounded("(task TODO)", 1, 32 * 1024 * 1024));
    let first = tasks();
    assert!(first.exceeded);
    assert_eq!(first.total, 2);
    let mut notes = g.load_named("Notes", PageKind::Page).unwrap().unwrap();
    notes.blocks[0].raw = "still unrelated".into();
    g.save_page(&notes, notes.rev.as_deref()).unwrap();
    // This assertion deliberately observes the committed post-edit image.
    wait_for_direct_query_projection(&g);
    let after_unrelated = tasks();
    assert!(after_unrelated.exceeded);
    assert_eq!(after_unrelated.total, 2);
    assert_eq!(
        serde_json::to_vec(first.groups.as_ref()).unwrap(),
        serde_json::to_vec(after_unrelated.groups.as_ref()).unwrap(),
        "the new SQL image has the same bounded answer after an irrelevant edit"
    );

    let admitted = first.groups[0].page.clone();
    let omitted = if admitted == "A" { "B" } else { "A" };
    let mut page = g.load_named(omitted, PageKind::Page).unwrap().unwrap();
    page.blocks[0].raw = "DONE no longer matches".into();
    g.save_page(&page, page.rev.as_deref()).unwrap();
    // This assertion deliberately observes the committed post-edit image.
    wait_for_direct_query_projection(&g);

    let after = tasks();
    assert!(!after.exceeded);
    assert_eq!(after.total, 1);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn advanced_query_recomputes_opaque_nul_source_after_sql_revision_change() {
    let dir = scratch("advanced-cache-nul-query");
    fs::write(dir.join("pages").join("P.md"), "- DONE ship\n").unwrap();
    let g = ready_graph(&dir);
    let query = "[:find (pull ?b [*]) :where \0 (task ?b #{\"TODO\"})]";
    let first = when_ready(|| g.run_advanced_query_cached(query, None));
    assert!(first.groups.is_empty());

    let mut page = g.load_named("P", PageKind::Page).unwrap().unwrap();
    page.blocks[0].raw = "TODO ship".into();
    g.save_page(&page, page.rev.as_deref()).unwrap();
    // This assertion deliberately observes the committed post-edit image.
    wait_for_direct_query_projection(&g);
    let warm = when_ready(|| g.run_advanced_query_cached(query, None));
    // The independent oracle: a fresh walk of the same graph, from the free
    // function, with no memo and no projection of its own to agree with.
    let fresh = crate::query::run_advanced_query(&Graph::open(&dir), query, None);
    assert_eq!(warm.groups.len(), 1);
    assert_eq!(warm.groups.len(), fresh.groups.len());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn derived_reference_memo_is_lru_bounded() {
    let dir = scratch("memo-lru-bound");
    let g = Graph::open(&dir);
    // This is the independent reference-cache bound; query answers are not retained.
    for i in 0..(DERIVED_CACHE_MAX_ENTRIES + 20) {
        let _ = g.derived_memo(format!("test\0{i}"), Vec::new);
    }
    let oversized_key = "x".repeat(DERIVED_CACHE_MAX_ENTRY_BYTES / 2 + 1);
    let _ = g.derived_memo(oversized_key.clone(), Vec::new);
    let derived = g.derived_cache.read().unwrap();
    assert_eq!(
        derived.as_ref().unwrap().results.len(),
        DERIVED_CACHE_MAX_ENTRIES
    );
    assert!(!derived
        .as_ref()
        .unwrap()
        .results
        .contains_key(&oversized_key));
    let oldest = format!("test\0{}", 0);
    assert!(!derived.as_ref().unwrap().results.contains_key(&oldest));
    let _ = fs::remove_dir_all(&dir);
}

// ---- sparse projection exact-write bridge ----

/// Pins every bounded save-failure string to the typed code assigned at its
/// production site. Rewording the display source cannot reclassify the error.
#[test]
fn direct_save_failure_codes_are_stable() {
    use std::io::{Error, ErrorKind};
    let typed = |code: &str, source: Error| {
        let code = DirectSaveFailureCode::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == code)
            .unwrap_or_else(|| panic!("missing DirectSaveFailureCode variant for {code}"));
        DirectSaveError::into_io(code, source)
    };
    for (code, error) in [
        // model.rs `capture_graph_text_entries` symlink arm.
        (
            "precheck.symlink",
            Error::new(
                ErrorKind::InvalidInput,
                "graph text entry is a symlink or reparse point: pages/Note.md",
            ),
        ),
        // `capture_retained_graph_text_identity_with_limits` two-pass equality.
        (
            "precheck.interrupted",
            Error::new(
                ErrorKind::Interrupted,
                "graph inventory changed during retained identity capture",
            ),
        ),
        // `validate_current_graph_text_collision_strict`, portable-key arm.
        (
            "precheck.portable_collision",
            Error::new(
                ErrorKind::AlreadyExists,
                "graph text paths share one portable case/NFC identity: pages/a.md and pages/A.md",
            ),
        ),
        // `validate_current_graph_text_collision_strict`, resource arm.
        (
            "precheck.resource_alias",
            Error::new(
                ErrorKind::AlreadyExists,
                "graph text files alias one physical resource: pages/a.md and pages/b.md",
            ),
        ),
        (
            "precheck.not_portable",
            Error::new(
                ErrorKind::InvalidInput,
                "guarded graph-text target is not portable: reserved name",
            ),
        ),
        (
            "precheck.nofollow",
            Error::new(
                ErrorKind::InvalidInput,
                "projection parent is not a real no-follow directory",
            ),
        ),
        (
            "precheck.limit",
            graph_text_capture_limit_error("peak build memory"),
        ),
        (
            "identity.owned_elsewhere",
            Error::new(
                ErrorKind::AlreadyExists,
                "another graph document owns this effective page identity",
            ),
        ),
        // `save_page`, base-rev arm.
        (
            "conflict.base_rev",
            Error::new(ErrorKind::AlreadyExists, "conflict"),
        ),
        // `consume_conflict_authority`, and the command boundary's refusal of
        // a force that names no observation. Their own family: a force whose
        // authority is dead is neither a fresh banner nor a transient
        // failure, and the frontend has to observe again to raise a live one.
        (
            "conflict_authority.superseded",
            Error::new(
                ErrorKind::PermissionDenied,
                "conflict override authority is newer than the conflict this request answers",
            ),
        ),
        (
            "conflict_authority.other_episode",
            Error::new(
                ErrorKind::PermissionDenied,
                "conflict override authority belongs to a different editor episode",
            ),
        ),
        (
            "conflict_authority.spent",
            Error::new(
                ErrorKind::PermissionDenied,
                "conflict override authority is missing or already consumed",
            ),
        ),
        // A page name is the user's to choose, and raw errors carry paths.
        // An unrelated failure that merely MENTIONS an authority sentence --
        // because a file is named after it -- must not inherit that family:
        // the frontend answers `conflict_authority.*` by re-observing, so a
        // permanent failure wearing that code would feed its own retry.
        // (GH #254 increment 2, fifth correction-delta re-verification.)
        (
            "unknown",
            Error::new(
                ErrorKind::Other,
                "exact-identity restore failed for pages/\
                     conflict override authority is missing or already consumed.md",
            ),
        ),
        // `require_pinned_save_owner`, LoadedRevision arm: the file moved
        // between load and save without the watcher seeing it. A real
        // conflict, and one "keep mine" resolves.
        (
            "conflict.pinned_owner",
            Error::new(
                ErrorKind::AlreadyExists,
                "path-pinned page does not match its captured exact owner",
            ),
        ),
        // Name collisions are real but are NOT content conflicts: the
        // keep-mine/use-disk prompt cannot resolve one.
        (
            "identity.name_taken",
            Error::new(
                ErrorKind::AlreadyExists,
                "a page with that name already exists",
            ),
        ),
        (
            "identity.name_taken",
            Error::new(
                ErrorKind::AlreadyExists,
                "target page exists in another supported text extension",
            ),
        ),
        // The inversion this classifier exists for: an UNCLASSIFIED
        // AlreadyExists must not become a conflict. It used to fall into a
        // `conflict.other` catch-all, which raised a prompt whose two
        // options could not resolve it and whose "use disk" arm discards
        // the user's edits -- and which replaced the message text, so a
        // failure that had RETAINED those edits under a recovery name
        // reached the user as an unexplained conflict.
        (
            "unknown",
            Error::new(
                ErrorKind::AlreadyExists,
                "displaced target retained as pages/Note.md.editor-recovery",
            ),
        ),
        (
            "unknown",
            Error::new(ErrorKind::PermissionDenied, "permission denied"),
        ),
    ] {
        let error = typed(code, error);
        assert_eq!(
            direct_save_failure_code(&error),
            code,
            "classifier drifted for: {error}"
        );
    }
}

/// Every data-preservation refusal is one marker type, so one classifier arm
/// types all its producers; the marker must win even over an ErrorKind that
/// looks transient, because the verdict is on the draft's content and a resend
/// of the same draft is refused again (GH #535, GH #546).
#[test]
fn a_data_preservation_refusal_has_its_own_no_retry_code() {
    use std::io::ErrorKind;
    for kind in [
        ErrorKind::InvalidData,
        ErrorKind::Interrupted,
        ErrorKind::AlreadyExists,
    ] {
        let refusal =
            projection_semantic_refusal(kind, "refusing to drop an existing page preamble");
        assert_eq!(
            direct_save_failure_code(&refusal),
            "refused.data_preservation"
        );
    }
}

/// The site-to-code binding for the whole conflict vocabulary, driven through
/// the REAL producers rather than through a stamped fixture.
///
/// This is the half that can discard a user's work. `conflict.*` is the
/// banner class, and the banner's "Use disk version" throws away the unsaved
/// edit, so a site that mints the wrong `conflict.*` code -- or mints one at
/// all where the failure is not a conflict -- is a data-loss defect. Before
/// the classifier was typed, the prose test caught that by construction;
/// stamping the expected code onto a fixture and reading it back would not,
/// so every case here goes through `EditorConflictSite`'s own accessors and
/// through `Graph::tokenless_conflict_error`.
///
/// `EditorConflictSite::ALL` has a pinned length, so a new site cannot be
/// added without appearing here.
#[test]
fn direct_save_conflict_sites_produce_their_own_codes() {
    for (site, suffix) in EditorConflictSite::ALL.into_iter().zip([
        "save_baseline_present",
        "save_baseline_absent",
        "commit_recheck",
        "replace_pre_retirement",
        "replace_retired_mismatch",
        "replace_publication_collision",
        "create_publication_collision",
        "final_reread_absent",
        "final_reread_present",
        "replace_post_publication",
    ]) {
        // The banner class, as `conflict_error_from_snapshot` reads it. That
        // producer needs a graph to mint an authority epoch; the branch under
        // test is its code selection, which is this accessor.
        assert_eq!(
            site.conflict_code().as_str(),
            format!("conflict.{suffix}"),
            "conflict site drifted from its banner code: {}",
            site.message()
        );

        // The retry class, through the real producer end to end.
        let tokenless = Graph::tokenless_conflict_error(
            site,
            std::io::Error::new(std::io::ErrorKind::WouldBlock, "continued churn"),
        );
        assert_eq!(
            direct_save_failure_code(&tokenless),
            format!("conflict_retry.{suffix}"),
            "tokenless conflict site drifted from its retry code: {}",
            site.tokenless_message()
        );
        assert_eq!(
            direct_save_conflict_epoch(&tokenless),
            None,
            "a tokenless conflict has no authority epoch to present"
        );
    }
}

/// The same binding for the precheck helpers, which are free functions and so
/// can be driven directly. `graph_text_capture_limit_error` and
/// `graph_text_inventory_limit_error` are the two the save path calls when a
/// bound is exceeded; both are `precheck.limit`, and neither may become a
/// conflict.
#[test]
fn direct_save_precheck_helpers_produce_their_own_codes() {
    for error in [
        graph_text_capture_limit_error("entries"),
        graph_text_inventory_limit_error("bytes"),
    ] {
        assert_eq!(direct_save_failure_code(&error), "precheck.limit");
        assert_eq!(direct_save_conflict_epoch(&error), None);
    }
}

#[test]
fn direct_save_failure_code_does_not_inherit_conflict_from_page_text() {
    let error = std::io::Error::new(
        std::io::ErrorKind::Other,
        "exact-identity restore failed for pages/path-pinned page does not match its captured exact owner.md",
    );

    assert_eq!(direct_save_failure_code(&error), "unknown");
    assert_eq!(direct_save_conflict_epoch(&error), None);
}

/// Existing saves inspect only their exact retained parent, and skip
/// unrelated symlinks there just as graph-text discovery does. Symlinks in
/// other parents are outside the local validation boundary entirely.
#[cfg(unix)]
#[test]
fn unrelated_symlinks_do_not_expand_an_existing_save() {
    use std::os::unix::fs::symlink;

    // A symlink outside the target parent is unrelated.
    let dir = scratch("symlink-scope-assets");
    fs::create_dir_all(dir.join("assets")).unwrap();
    fs::write(dir.join("pages/Target.md"), b"- before\n").unwrap();
    symlink(dir.join("pages/Target.md"), dir.join("assets/link.md")).unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let page = graph.load_by_path("pages/Target.md").unwrap().unwrap();
    let base = content_rev("- before\n");
    assert!(
        graph.save_page(&page, Some(&base)).is_ok(),
        "a symlink under assets/ must not block saves -- assets is fixed-excluded"
    );
    let _ = fs::remove_dir_all(&dir);

    // A different-name symlink inside the target parent is not an admitted
    // graph-text sibling and cannot redirect the exact target.
    for (tag, link, target) in [
        (
            "symlink-scope-pages-file",
            "pages/Alias.md",
            "pages/Target.md",
        ),
        ("symlink-scope-pages-dir", "pages/Linked", "pages"),
        ("symlink-scope-root-dir", "Linked", "pages"),
    ] {
        let dir = scratch(tag);
        fs::write(dir.join("pages/Target.md"), b"- before\n").unwrap();
        symlink(dir.join(target), dir.join(link)).unwrap();
        let graph = Graph::open(&dir);
        graph.warm_cache();
        let page = graph.load_by_path("pages/Target.md").unwrap().unwrap();
        let base = content_rev("- before\n");
        graph.save_page(&page, Some(&base)).unwrap_or_else(|error| {
            panic!("a symlink at {link} must not block an unrelated save: {error}")
        });
        let _ = fs::remove_dir_all(&dir);
    }
}

/// Build a Direct-mode graph of `pages` ordinary pages plus one target.
fn direct_save_bench_graph(tag: &str, pages: usize) -> (PathBuf, Graph) {
    let dir = scratch(tag);
    for index in 0..pages {
        let body = (0..24)
            .map(|line| format!("- block {line} of page {index} with some ordinary text\n"))
            .collect::<String>();
        fs::write(
            dir.join(format!("pages/Page {index:05}.md")),
            format!("title:: Page {index:05}\n\n{body}"),
        )
        .unwrap();
    }
    fs::write(dir.join("pages/Target.md"), b"- before\n").unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    (dir, graph)
}

fn direct_save_bench_once(graph: &Graph, marker: &str) -> std::time::Duration {
    let mut page = graph.load_by_path("pages/Target.md").unwrap().unwrap();
    page.blocks[0].raw = marker.to_owned();
    let started = std::time::Instant::now();
    graph
        .save_page(&page, page.rev.as_deref())
        .expect("direct save");
    started.elapsed()
}

/// GH #267. Losing complete-index certainty is unrelated to the authority
/// for an already loaded exact target. Existing Direct Files saves must
/// therefore remain target-local for both supported text formats.
#[test]
fn invalidated_graph_index_does_not_expand_an_existing_save() {
    for (tag, extension, before, after) in [
        (
            "existing-save-cut-markdown",
            "md",
            "- before\n",
            "saved markdown",
        ),
        ("existing-save-cut-org", "org", "* before\n", "saved org"),
    ] {
        let dir = scratch(tag);
        for index in 0..24 {
            fs::write(
                dir.join("pages").join(format!("Unrelated {index}.md")),
                format!("title:: Unrelated {index}\n\n- body {index}\n"),
            )
            .unwrap();
        }
        let relative = format!("pages/Target.{extension}");
        fs::write(dir.join(&relative), before).unwrap();
        let graph = Graph::open(&dir);
        graph.warm_cache();
        let mut page = graph.load_by_path(&relative).unwrap().unwrap();
        page.blocks[0].raw = after.to_owned();

        graph
            .observe_graph_text_external_paths(std::iter::empty::<&Path>(), true)
            .unwrap();
        let before_report = graph.guarded_graph_text_identity_report();
        assert!(before_report.invalidated, "test must start invalidated");
        GRAPH_TEXT_PARSE_ATTEMPTS.with(|attempts| attempts.set(0));

        graph.save_page(&page, page.rev.as_deref()).unwrap();

        let after_report = graph.guarded_graph_text_identity_report();
        assert_eq!(
            after_report.complete_builds, before_report.complete_builds,
            "an existing {extension} save must not construct the complete graph index"
        );
        assert_eq!(
            GRAPH_TEXT_PARSE_ATTEMPTS.with(Cell::get),
            1,
            "an existing {extension} save must parse only its exact target"
        );
        assert!(
            fs::read_to_string(dir.join(relative))
                .unwrap()
                .contains(after),
            "the target-local {extension} save must reach disk"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}

/// A content-only existing save must carry the already-warm semantic
/// evidence forward. The immediately following creation is target-local: it
/// may not census, rebuild, retain, or parse the graph.
#[test]
fn identity_preserving_existing_save_keeps_creation_evidence_warm() {
    let dir = scratch("existing-save-then-create-warm-evidence");
    fs::write(dir.join("pages/Existing.md"), b"- before\n").unwrap();
    fs::write(
        dir.join("pages/Explicit Owner.md"),
        b"title:: Claimed Identity\n\n- owner\n",
    )
    .unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    graph.list_pages();

    let mut existing = graph.load_by_path("pages/Existing.md").unwrap().unwrap();
    existing.blocks[0].raw = "after".into();
    graph
        .save_page(&existing, existing.rev.as_deref())
        .expect("identity-preserving existing save");
    let (inventory_generation, inventory_entries) = graph
        .page_list_cache
        .read()
        .unwrap()
        .as_ref()
        .cloned()
        .expect("content-only save must retain the warm page inventory");
    assert_eq!(inventory_generation, graph.cache_generation());
    assert!(inventory_entries
        .iter()
        .any(|entry| entry.rel_path == "pages/Existing.md"));
    let installed = graph
        .effective_identity_index
        .read()
        .unwrap()
        .as_ref()
        .cloned()
        .expect("content-only save must retain warm semantic evidence");
    assert_eq!(installed.generation(), graph.cache_generation());

    let before = graph.guarded_graph_text_identity_report();
    reset_graph_text_admission_test_counters();
    GRAPH_TEXT_PARSE_ATTEMPTS.with(|attempts| attempts.set(0));
    GRAPH_TEXT_FIRST_CAPTURE_CHARGE_OVERRIDE
        .with(|charge| charge.set(Some(GRAPH_TEXT_CAPTURE_LIMITS.peak_build_bytes)));
    graph
        .save_page(
            &direct_save_bench_new_page("Fresh After Existing Save"),
            None,
        )
        .expect("warm evidence must authorize the noncolliding creation");
    let after = graph.guarded_graph_text_identity_report();
    let counters = graph_text_admission_test_counters();
    assert_eq!(counters.direct_creation_censuses, 0);
    assert_eq!(counters.direct_creation_files_hashed, 0);
    assert_eq!(counters.builder_enumerations, 0);
    assert_eq!(counters.parser_invocations, 0);
    assert_eq!(GRAPH_TEXT_PARSE_ATTEMPTS.with(Cell::get), 0);
    assert_eq!(after.complete_builds, before.complete_builds);
    assert_eq!(
        GRAPH_TEXT_FIRST_CAPTURE_CHARGE_OVERRIDE.with(Cell::take),
        Some(GRAPH_TEXT_CAPTURE_LIMITS.peak_build_bytes),
        "creation must not consume the retained capture hook"
    );
    assert_eq!(
        fs::read(dir.join("pages/Existing.md")).unwrap(),
        b"- after\n"
    );
    assert!(dir.join("pages/Fresh After Existing Save.md").is_file());
    let _ = fs::remove_dir_all(&dir);
}

/// The O(1) content-only path is not authority to retain stale semantic
/// ownership when an ordinary existing save changes `title::`.
#[test]
fn identity_changing_existing_save_refreshes_creation_evidence() {
    let dir = scratch("existing-save-retitles-identity-evidence");
    fs::write(
        dir.join("pages/Physical Owner.md"),
        b"title:: Alpha Identity\n\n- owner\n",
    )
    .unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();

    let mut owner = graph
        .load_by_path("pages/Physical Owner.md")
        .unwrap()
        .unwrap();
    owner.pre_block = Some("title:: Omega Identity\n".into());
    graph
        .save_page(&owner, owner.rev.as_deref())
        .expect("identity-changing existing save");

    let installed = graph
        .effective_identity_index
        .read()
        .unwrap()
        .as_ref()
        .cloned()
        .expect("identity-changing save must publish replacement evidence");
    assert_eq!(installed.generation(), graph.cache_generation());
    assert!(!installed
        .owners
        .contains_key(&page_cache_key(PageKind::Page, "Alpha Identity")));
    assert!(installed
        .owners
        .contains_key(&page_cache_key(PageKind::Page, "Omega Identity")));
    let target = dir.join("pages/Omega Identity.md");
    let error = graph
        .save_page(&direct_save_bench_new_page("Omega Identity"), None)
        .expect_err("the new effective owner must refuse a duplicate creation");
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{error}");
    assert!(!target.exists());
    let _ = fs::remove_dir_all(&dir);
}

/// Existing exact-target authority is not invalidated by a noncolliding
/// sibling under another spelling of the same portable ancestor. A leaf
/// collision under that ancestor remains a hard refusal.
#[test]
fn existing_save_allows_portable_ancestor_neighbor_but_refuses_colliding_leaf() {
    for (tag, alias_leaf, should_save) in [
        ("noncolliding", "Other.md", true),
        ("colliding", "Target.md", false),
    ] {
        let dir = scratch(&format!("existing-save-portable-ancestor-{tag}"));
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq/config.edn"),
            "{:pages-directory \"Pages\"}\n",
        )
        .unwrap();
        fs::create_dir_all(dir.join("Pages")).unwrap();
        let target = dir.join("Pages/Target.md");
        let neighbor = dir.join("pages").join(alias_leaf);
        fs::write(&target, b"- before\n").unwrap();
        fs::write(&neighbor, b"- neighbor\n").unwrap();
        let graph = Graph::open(&dir);
        graph.warm_cache();
        let mut page = graph.load_by_path("Pages/Target.md").unwrap().unwrap();
        page.blocks[0].raw = "after".into();

        let result = graph.save_page(&page, page.rev.as_deref());
        if should_save {
            result.expect("noncolliding portable ancestor neighbor must not block save");
            assert_eq!(fs::read(&target).unwrap(), b"- after\n");
        } else {
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::AlreadyExists);
            assert_eq!(fs::read(&target).unwrap(), b"- before\n");
        }
        assert_eq!(fs::read(&neighbor).unwrap(), b"- neighbor\n");
        let _ = fs::remove_dir_all(&dir);
    }
}

/// A portable-equivalent symlink branch is not traversal authority over an
/// already loaded exact target. The save stays on the retained exact path.
#[cfg(unix)]
#[test]
fn existing_save_allows_portable_equivalent_symlink_neighbor() {
    let dir = scratch("existing-save-portable-symlink-neighbor");
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq/config.edn"),
        "{:pages-directory \"Pages\"}\n",
    )
    .unwrap();
    fs::remove_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("Pages")).unwrap();
    let target = dir.join("Pages/Target.md");
    fs::write(&target, b"- before\n").unwrap();
    let outside = dir.with_extension("portable-symlink-neighbor");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("Other.md"), b"- outside neighbor\n").unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("pages")).unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let mut page = graph.load_by_path("Pages/Target.md").unwrap().unwrap();
    page.blocks[0].raw = "after".into();

    graph
        .save_page(&page, page.rev.as_deref())
        .expect("portable-equivalent symlink neighbor must not block exact save");
    assert_eq!(fs::read(&target).unwrap(), b"- after\n");
    assert_eq!(
        fs::read(outside.join("Other.md")).unwrap(),
        b"- outside neighbor\n"
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&outside);
}

/// Repeated existing saves never need to construct the complete admission
/// index, even when their own exact publications leave that optional index
/// invalidated. State this as counters rather than a CI stopwatch.
#[test]
fn steady_state_direct_saves_never_build_the_graph_index() {
    let (dir, graph) = direct_save_bench_graph("direct-save-steady", 40);

    let before = graph.guarded_graph_text_identity_report();
    direct_save_bench_once(&graph, "- warm");
    let warm = graph.guarded_graph_text_identity_report();
    assert_eq!(
        warm.complete_builds, before.complete_builds,
        "the first existing save must not build the complete index: {warm:?}"
    );

    for round in 0..8 {
        direct_save_bench_once(&graph, &format!("- round {round}"));
    }

    let after = graph.guarded_graph_text_identity_report();
    assert_eq!(
        after.complete_builds, warm.complete_builds,
        "a steady-state Direct save must not build the whole-graph admission index"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// REG-DIRECT-CREATE-RETAINED-SHADOW-LIMIT-249-266 causal witness. On the
/// parent behavior, ordinary missing-target creation entered the retained
/// shadow-import builder and surfaced the exact v0.6.92 reporter suffix.
#[test]
fn missing_target_creation_ignores_the_retained_capture_peak_limit() {
    let dir = scratch("missing-target-retained-shadow-limit");
    fs::write(dir.join("pages/Existing.md"), b"- existing\n").unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();

    GRAPH_TEXT_FIRST_CAPTURE_CHARGE_OVERRIDE
        .with(|charge| charge.set(Some(GRAPH_TEXT_CAPTURE_LIMITS.peak_build_bytes)));
    let target = dir.join("pages/Noncolliding Missing Target.md");
    graph
        .save_page(
            &direct_save_bench_new_page("Noncolliding Missing Target"),
            None,
        )
        .expect("ordinary creation must not consult the retained capture peak bound");
    assert!(target.is_file(), "the admitted creation must publish bytes");
    assert_eq!(
        GRAPH_TEXT_FIRST_CAPTURE_CHARGE_OVERRIDE.with(Cell::take),
        Some(GRAPH_TEXT_CAPTURE_LIMITS.peak_build_bytes),
        "ordinary creation consumed the retained capture hook"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// The whole validation/publication path is target-local: no graph census,
/// retained capture, complete-index build, or parse work.
#[test]
fn missing_target_creation_has_zero_graph_census_capture_or_parse_work() {
    let dir = scratch("missing-target-one-streaming-census");
    for index in 0..24 {
        fs::write(
            dir.join("pages").join(format!("Unrelated {index}.md")),
            format!("title:: Unrelated {index}\n\n- body {index}\n"),
        )
        .unwrap();
    }
    fs::write(
        dir.join("pages/Physical Owner.md"),
        b"title:: Claimed Name\n\n- owner\n",
    )
    .unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let before = graph.guarded_graph_text_identity_report();
    reset_graph_text_admission_test_counters();
    GRAPH_TEXT_PARSE_ATTEMPTS.with(|attempts| attempts.set(0));
    graph
        .save_page(&direct_save_bench_new_page("Fresh Claimed Name"), None)
        .unwrap();
    let after = graph.guarded_graph_text_identity_report();
    let counters = graph_text_admission_test_counters();
    assert_eq!(counters.direct_creation_censuses, 0);
    assert_eq!(counters.direct_creation_files_hashed, 0);
    assert_eq!(counters.builder_enumerations, 0);
    assert_eq!(counters.parser_invocations, 0);
    assert_eq!(
        after.complete_builds, before.complete_builds,
        "missing-target creation entered the complete semantic builder"
    );
    assert_eq!(
        GRAPH_TEXT_PARSE_ATTEMPTS.with(Cell::get),
        0,
        "missing-target creation parsed a graph document"
    );
    assert!(dir.join("pages/Fresh Claimed Name.md").is_file());
    let _ = fs::remove_dir_all(&dir);
}

/// A normally reconciled external retitle updates semantic ownership even
/// when path, inode, and byte length are unchanged.
#[test]
fn same_path_same_length_retitle_after_reconciliation_refuses_creation() {
    let dir = scratch("same-length-retitle-creation-proof");
    let owner = dir.join("pages/Owner.md");
    let before = b"title:: Alpha Name\n\n- owner\n";
    let after = b"title:: Omega Name\n\n- owner\n";
    assert_eq!(before.len(), after.len());
    fs::write(&owner, before).unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    fs::write(&owner, after).unwrap();
    graph.sync_file_checked(&owner).unwrap();
    reset_graph_text_admission_test_counters();
    GRAPH_TEXT_PARSE_ATTEMPTS.with(|attempts| attempts.set(0));
    let target = dir.join("pages/Omega Name.md");
    let error = graph
        .save_page(&direct_save_bench_new_page("Omega Name"), None)
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{error}");
    assert_eq!(fs::read(&owner).unwrap(), after);
    assert!(!target.exists());
    assert_eq!(
        graph_text_admission_test_counters().direct_creation_censuses,
        0
    );
    assert_eq!(graph_text_admission_test_counters().builder_enumerations, 0);
    assert_eq!(GRAPH_TEXT_PARSE_ATTEMPTS.with(Cell::get), 0);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn creation_refuses_historical_arbitrary_and_explicit_semantic_owners() {
    for (extension, content) in [
        ("md", "- incumbent md\n"),
        ("markdown", "- incumbent markdown\n"),
        ("org", "* incumbent org\n"),
    ] {
        let dir = scratch(&format!("creation-semantic-owner-{extension}"));
        fs::create_dir_all(dir.join("arbitrary/deep")).unwrap();
        let incumbent = dir.join(format!("arbitrary/deep/Claimed Owner.{extension}"));
        fs::write(&incumbent, content).unwrap();
        let graph = Graph::open(&dir);
        graph.warm_cache();
        let target = dir.join("pages/Claimed Owner.md");
        let error = graph
            .save_page(&direct_save_bench_new_page("Claimed Owner"), None)
            .unwrap_err();
        assert_eq!(
            error.kind(),
            io::ErrorKind::AlreadyExists,
            "{extension}: {error}"
        );
        assert_eq!(fs::read_to_string(&incumbent).unwrap(), content);
        assert!(!target.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    let dir = scratch("creation-explicit-title-owner");
    fs::create_dir_all(dir.join("arbitrary/deep")).unwrap();
    let incumbent = dir.join("arbitrary/deep/Different Physical Name.md");
    let content = "title:: Claimed Explicit Owner\n\n- incumbent\n";
    fs::write(&incumbent, content).unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let target = dir.join("pages/Claimed Explicit Owner.md");
    let error = graph
        .save_page(&direct_save_bench_new_page("Claimed Explicit Owner"), None)
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{error}");
    assert_eq!(fs::read_to_string(&incumbent).unwrap(), content);
    assert!(!target.exists());
    let _ = fs::remove_dir_all(&dir);
}

#[path = "model_alias_admission_tests.rs"]
mod alias_admission;

#[path = "model_live_conflict_authority_tests.rs"]
mod live_conflict_authority;

#[path = "model_rename_cost_tests.rs"]
mod rename_cost;
#[path = "model_rename_refresh_tests.rs"]
mod rename_refresh;

/// GH #366's literal reporter page name. Unicode itself must not make an
/// otherwise ordinary Direct Files creation ambiguous; the neighboring test
/// retains the fail-closed NFC/NFD collision boundary.
#[test]
fn direct_creation_round_trips_a_chinese_page_name() {
    let dir = scratch("creation-chinese-page-name");
    fs::write(dir.join("pages/Anchor.md"), b"- anchor\n").unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let page = direct_save_bench_new_page("TINE版本更新提示词");

    graph.save_page(&page, None).unwrap();

    let path = dir.join("pages/TINE版本更新提示词.md");
    assert!(path.exists());
    assert_eq!(
        graph
            .load_named("TINE版本更新提示词", PageKind::Page)
            .unwrap()
            .unwrap()
            .blocks[0]
            .raw,
        "created"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn creation_refuses_symlink_parent_and_leaf_without_touching_outside_bytes() {
    let dir = scratch("creation-symlink-leaf-refusal");
    let outside = dir.with_extension("leaf-outside");
    fs::write(&outside, b"outside leaf\n").unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    std::os::unix::fs::symlink(&outside, dir.join("pages/Leaf.md")).unwrap();
    let error = graph
        .save_page(&direct_save_bench_new_page("Leaf"), None)
        .unwrap_err();
    assert!(
        matches!(
            error.kind(),
            io::ErrorKind::InvalidInput | io::ErrorKind::AlreadyExists
        ),
        "{error}"
    );
    assert_eq!(fs::read(&outside).unwrap(), b"outside leaf\n");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_file(&outside);

    let dir = scratch("creation-symlink-parent-refusal");
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq/config.edn"),
        "{:pages-directory \"linked/pages\"}\n",
    )
    .unwrap();
    let outside = dir.with_extension("parent-outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("incumbent"), b"outside parent\n").unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    std::os::unix::fs::symlink(&outside, dir.join("linked")).unwrap();
    let error = graph
        .save_page(&direct_save_bench_new_page("Fresh"), None)
        .unwrap_err();
    assert!(
        matches!(
            error.kind(),
            io::ErrorKind::InvalidInput | io::ErrorKind::AlreadyExists
        ),
        "{error}"
    );
    assert_eq!(
        fs::read(outside.join("incumbent")).unwrap(),
        b"outside parent\n"
    );
    assert!(!outside.join("pages/Fresh.md").exists());
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&outside);
}

#[test]
fn external_exact_target_creator_wins_without_byte_change() {
    let dir = scratch("creation-external-target-race");
    fs::write(dir.join("pages/Anchor.md"), b"- anchor\n").unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let target = dir.join("pages/Raced.md");
    GRAPH_TEXT_WRITE_BEFORE_MUTATION.with(|hook| {
        let target = target.clone();
        *hook.borrow_mut() = Some(Box::new(move || fs::write(target, b"external winner\n")));
    });
    let error = graph
        .save_page(&direct_save_bench_new_page("Raced"), None)
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{error}");
    assert_eq!(fs::read(&target).unwrap(), b"external winner\n");
    assert_eq!(
        fs::read(dir.join("pages/Anchor.md")).unwrap(),
        b"- anchor\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn external_portable_alias_creator_wins_before_creation_publication() {
    let dir = scratch("creation-external-portable-alias-race");
    fs::write(dir.join("pages/Anchor.md"), b"- anchor\n").unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let target = dir.join("pages/Raced.md");
    let alias = dir.join("pages/raced.md");
    GRAPH_TEXT_WRITE_BEFORE_MUTATION.with(|hook| {
        let alias = alias.clone();
        *hook.borrow_mut() = Some(Box::new(move || fs::write(alias, b"external winner\n")));
    });

    let error = graph
        .save_page(&direct_save_bench_new_page("Raced"), None)
        .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{error}");
    assert_eq!(fs::read(&alias).unwrap(), b"external winner\n");
    assert!(!target.exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn external_semantic_owner_creator_wins_before_creation_publication() {
    let dir = scratch("creation-external-semantic-owner-race");
    fs::write(dir.join("pages/Anchor.md"), b"- anchor\n").unwrap();
    let graph = Arc::new(Graph::open(&dir));
    graph.warm_cache();
    let target = dir.join("pages/Raced Semantic.md");
    let owner = dir.join("pages/External.md");
    GRAPH_TEXT_WRITE_BEFORE_MUTATION.with(|hook| {
        let owner = owner.clone();
        let graph = Arc::clone(&graph);
        *hook.borrow_mut() = Some(Box::new(move || {
            fs::write(owner, b"title:: Raced Semantic\n\n- external winner\n")?;
            graph.note_graph_text_external_observation();
            Ok(())
        }));
    });

    let error = graph
        .save_page(&direct_save_bench_new_page("Raced Semantic"), None)
        .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::WouldBlock, "{error}");
    assert_eq!(
        fs::read(&owner).unwrap(),
        b"title:: Raced Semantic\n\n- external winner\n"
    );
    assert!(!target.exists());
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn external_portable_symlink_alias_refuses_creation_publication() {
    let dir = scratch("creation-external-portable-symlink-alias-race");
    fs::write(dir.join("pages/Anchor.md"), b"- anchor\n").unwrap();
    let outside = dir.with_extension("external-symlink-owner");
    fs::write(&outside, b"external winner\n").unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let target = dir.join("pages/Raced.md");
    let alias = dir.join("pages/raced.md");
    GRAPH_TEXT_WRITE_BEFORE_MUTATION.with(|hook| {
        let alias = alias.clone();
        let outside = outside.clone();
        *hook.borrow_mut() = Some(Box::new(move || std::os::unix::fs::symlink(outside, alias)));
    });

    let error = graph
        .save_page(&direct_save_bench_new_page("Raced"), None)
        .unwrap_err();

    assert!(
        matches!(
            error.kind(),
            io::ErrorKind::InvalidInput | io::ErrorKind::AlreadyExists
        ),
        "{error}"
    );
    assert_eq!(fs::read(&outside).unwrap(), b"external winner\n");
    assert!(alias.is_symlink());
    assert!(!target.exists());
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_file(&outside);
}

/// GH #267 / F3. Leave a deterministic whole-graph capture race armed and
/// prove an existing save never reaches it.
#[test]
fn an_existing_save_never_enters_the_graph_capture_race() {
    let dir = scratch("existing-save-skips-capture-retry");
    fs::write(dir.join("pages/Target.md"), b"- before\n").unwrap();
    fs::write(dir.join("pages/Other.md"), b"- other\n").unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();

    // This hook runs only between the old capture's two graph-wide passes.
    GRAPH_TEXT_CAPTURE_REVALIDATION_RACE.with(|hook| {
        let other = dir.join("pages/Other.md");
        *hook.borrow_mut() = Some(Box::new(move || fs::write(&other, b"- other, pulled in\n")));
    });

    let mut page = graph.load_by_path("pages/Target.md").unwrap().unwrap();
    let base = page.rev.clone().expect("loaded page carries its revision");
    page.blocks[0].raw = "saved during sync activity".into();
    graph
        .save_page(&page, Some(&base))
        .expect("a sync client touching an unrelated file must not fail this save");
    GRAPH_TEXT_CAPTURE_REVALIDATION_RACE.with(|hook| {
        assert!(
            hook.borrow_mut().take().is_some(),
            "existing save must not enter whole-graph capture"
        );
    });
    assert_eq!(
        fs::read_to_string(dir.join("pages/Target.md")).unwrap(),
        "- saved during sync activity\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Replace `relative` atomically with `content`, giving the path a NEW inode
/// while its bytes may be unchanged. This is what OneDrive rehydration, a
/// Syncthing pull and a plain `cp` into place all look like from Tine.
fn replace_file_with_a_new_inode(dir: &Path, relative: &str, content: &[u8]) {
    let target = dir.join(relative);
    let staged = dir.join(format!("{relative}.replacement"));
    fs::write(&staged, content).unwrap();
    fs::rename(&staged, &target).unwrap();
}

/// GH #267 / F4. An external tool replacing a file with byte-identical
/// content used to strand the page: the save refused with "existing page
/// identity changed since load", "Keep mine (overwrite)" hit the same check,
/// and the only working button discarded the user's edit.
#[test]
fn a_same_bytes_external_replace_does_not_strand_the_editor() {
    let dir = scratch("same-bytes-replace");
    fs::write(dir.join("pages/Foo.md"), b"- before\n").unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();

    let mut page = graph.load_by_path("pages/Foo.md").unwrap().unwrap();
    let base_rev = page.rev.clone().expect("loaded page carries its revision");

    // Same bytes, new inode.
    replace_file_with_a_new_inode(&dir, "pages/Foo.md", b"- before\n");
    graph.sync_file_checked(&dir.join("pages/Foo.md")).unwrap();

    page.blocks[0].raw = "edited after the replace".into();
    graph
        .save_page(&page, Some(&base_rev))
        .expect("the path holds exactly the bytes the editor loaded");
    assert_eq!(
        fs::read_to_string(dir.join("pages/Foo.md")).unwrap(),
        "- edited after the replace\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// The other direction, which must NOT change: a replace that also changes
/// the bytes is a real external edit and still conflicts.
#[test]
fn a_changed_bytes_external_replace_still_conflicts() {
    let dir = scratch("changed-bytes-replace");
    fs::write(dir.join("pages/Foo.md"), b"- before\n").unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();

    let mut page = graph.load_by_path("pages/Foo.md").unwrap().unwrap();
    let base_rev = page.rev.clone().expect("loaded page carries its revision");

    replace_file_with_a_new_inode(&dir, "pages/Foo.md", b"- changed elsewhere\n");
    graph.sync_file_checked(&dir.join("pages/Foo.md")).unwrap();

    page.blocks[0].raw = "edited after the replace".into();
    let error = graph
        .save_page(&page, Some(&base_rev))
        .expect_err("an external edit must still raise a conflict");
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{error}");
    assert_eq!(
        fs::read_to_string(dir.join("pages/Foo.md")).unwrap(),
        "- changed elsewhere\n",
        "the refused save must not have written anything"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// A reconciled external retitle refuses duplicate semantic creation without
/// rebuilding or hashing the graph.
#[test]
fn missing_target_creation_refuses_an_externally_retitled_owner() {
    let dir = scratch("creation-proof-follows-external-retitle");
    fs::write(dir.join("pages/Owner.md"), b"title:: Alpha Name\n\n- o\n").unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    fs::write(dir.join("pages/Owner.md"), b"title:: Omega Name\n\n- o\n").unwrap();
    graph
        .sync_file_checked(&dir.join("pages/Owner.md"))
        .unwrap();
    let before = graph.guarded_graph_text_identity_report();
    reset_graph_text_admission_test_counters();
    let error = graph
        .save_page(&direct_save_bench_new_page("Omega Name"), None)
        .expect_err("the retitled document owns this effective page identity");
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{error}");
    assert_eq!(
        graph.guarded_graph_text_identity_report().complete_builds,
        before.complete_builds
    );
    assert_eq!(
        graph_text_admission_test_counters().direct_creation_censuses,
        0
    );
    assert_eq!(graph_text_admission_test_counters().builder_enumerations, 0);
    assert!(!dir.join("pages/Omega Name.md").exists());
    let _ = fs::remove_dir_all(&dir);
}

/// Cached semantic evidence is keyed by content, not inode identity. A
/// same-byte republication remains admissible; changed bytes fail closed.
#[test]
fn reused_semantics_follow_the_bytes_not_the_file() {
    let dir = scratch("rebuild-reuse-follows-bytes");
    let alpha = b"title:: Alpha Name\n\n- o\n";
    fs::write(dir.join("pages/Owner.md"), alpha).unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    guarded_test_prime_identity(&graph);

    graph
        .observe_graph_text_external_paths(std::iter::empty::<&Path>(), true)
        .unwrap();
    let owner = dir.join("pages/Owner.md");
    let observed = graph.graph_text_external_observation_ticket();
    graph.sync_file_checked(&owner).unwrap();
    graph.acknowledge_graph_text_external_observations(observed);
    graph
        .save_page(&direct_save_bench_new_page("Semantic Prime"), None)
        .unwrap();

    replace_file_with_a_new_inode(&dir, "pages/Owner.md", alpha);
    graph
        .observe_graph_text_external_paths(std::iter::empty::<&Path>(), true)
        .unwrap();
    let observed = graph.graph_text_external_observation_ticket();
    graph.sync_file_checked(&owner).unwrap();
    graph.acknowledge_graph_text_external_observations(observed);
    GRAPH_TEXT_PARSE_ATTEMPTS.with(|attempts| attempts.set(0));
    graph
        .save_page(&direct_save_bench_new_page("Same Bytes Proof"), None)
        .unwrap();
    let same_bytes = GRAPH_TEXT_PARSE_ATTEMPTS.with(Cell::get);

    replace_file_with_a_new_inode(&dir, "pages/Owner.md", b"title:: Omega Name\n\n- o\n");
    graph
        .observe_graph_text_external_paths(std::iter::empty::<&Path>(), true)
        .unwrap();
    let observed = graph.graph_text_external_observation_ticket();
    graph.sync_file_checked(&owner).unwrap();
    graph.acknowledge_graph_text_external_observations(observed);
    GRAPH_TEXT_PARSE_ATTEMPTS.with(|attempts| attempts.set(0));
    let error = graph
        .save_page(&direct_save_bench_new_page("Omega Name"), None)
        .expect_err("the retitled document owns its new semantic identity");
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{error}");
    assert_eq!(
        GRAPH_TEXT_PARSE_ATTEMPTS.with(Cell::get),
        same_bytes,
        "changed census bytes must fail without parsing"
    );
    assert!(!dir.join("pages/Omega Name.md").exists());
    let _ = fs::remove_dir_all(&dir);
}

fn direct_save_bench_new_page(name: &str) -> PageDto {
    PageDto {
        activation: None,
        name: name.to_owned(),
        kind: PageKind::Page,
        title: name.to_owned(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: "created".into(),
            raw: "created".into(),
            ..Default::default()
        }],
        rev: None,
        format: Format::Md,
        read_only: false,
        path: String::new(),
        guide: false,
    }
}

/// The measured receipt behind that gate. Release-only and `--ignored`: it
/// compares an existing Direct save with a valid complete index against the
/// same save after forced invalidation. Point it at a real graph copy with
/// TINE_DIRECT_SAVE_BENCH_GRAPH_COPY, or let it synthesise one.
#[test]
#[ignore = "manual benchmark: Direct-mode save latency, warm vs invalidated"]
fn direct_save_latency_manual_benchmark() {
    assert!(
            !cfg!(debug_assertions),
            "release-only; run cargo test -p tine-core --release direct_save_latency_manual_benchmark -- --ignored --nocapture"
        );
    let rounds: usize = std::env::var("TINE_DIRECT_SAVE_BENCH_ROUNDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(16);
    let (dir, graph) = match std::env::var("TINE_DIRECT_SAVE_BENCH_GRAPH_COPY") {
        Ok(source) => {
            let dir = scratch("direct-save-bench-copy");
            copy_directory_tree(Path::new(&source), &dir);
            fs::write(dir.join("pages/Target.md"), b"- before\n").unwrap();
            let graph = Graph::open(&dir);
            graph.warm_cache();
            (dir, graph)
        }
        Err(_) => {
            let pages: usize = std::env::var("TINE_DIRECT_SAVE_BENCH_PAGES")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1_000);
            direct_save_bench_graph("direct-save-bench", pages)
        }
    };

    let describe = |label: &str, samples: &mut Vec<std::time::Duration>, graph: &Graph| {
        samples.sort();
        let report = graph.guarded_graph_text_identity_report();
        println!(
            "{label}: median {:?} p95 {:?} max {:?} over {} rounds; builds {} exact {} last {:?}",
            samples[samples.len() / 2],
            samples[samples.len() * 95 / 100],
            samples[samples.len() - 1],
            samples.len(),
            report.complete_builds,
            report.exact_updates,
            report.last_build,
        );
    };

    guarded_test_prime_identity(&graph);
    let builds_before = graph.guarded_graph_text_identity_report().complete_builds;
    direct_save_bench_once(&graph, "- prime");
    let mut warm = Vec::new();
    for round in 0..rounds {
        warm.push(direct_save_bench_once(&graph, &format!("- warm {round}")));
    }
    describe("warm index", &mut warm, &graph);

    let mut cold = Vec::new();
    for round in 0..rounds {
        graph
            .observe_graph_text_external_paths(std::iter::empty::<&Path>(), true)
            .unwrap();
        cold.push(direct_save_bench_once(&graph, &format!("- cold {round}")));
    }
    describe("invalidated index", &mut cold, &graph);
    assert_eq!(
        graph.guarded_graph_text_identity_report().complete_builds,
        builds_before,
        "existing-page benchmark must not construct another complete index"
    );

    let _ = fs::remove_dir_all(&dir);
}

fn direct_query_bench_fixture_bytes() -> &'static [u8] {
    b"title:: B4 Measurement Target\ncategory:: work\ntags:: work\n\n- TODO needle [[B4 Measurement Target]] #work\n  status:: active\n"
}

fn direct_query_bench_open() -> (PathBuf, Graph, usize, usize) {
    let dir = match std::env::var("TINE_DIRECT_QUERY_BENCH_GRAPH_COPY") {
        Ok(source) => {
            let dir = scratch("direct-query-bench-copy");
            copy_directory_tree(Path::new(&source), &dir);
            dir
        }
        Err(_) => {
            let pages = std::env::var("TINE_DIRECT_QUERY_BENCH_PAGES")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1_000);
            let (dir, graph) = direct_save_bench_graph("direct-query-bench", pages);
            drop(graph);
            dir
        }
    };
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::write(
        dir.join("pages/B4 Measurement Target.md"),
        direct_query_bench_fixture_bytes(),
    )
    .unwrap();
    fs::write(
        dir.join("pages/B4 Measurement Unrelated.md"),
        b"- b4-unrelated-before\n",
    )
    .unwrap();
    fs::write(
        dir.join("pages/B4___Measurement Namespace.md"),
        b"- b4 namespace probe\n",
    )
    .unwrap();
    fs::create_dir_all(dir.join("journals")).unwrap();
    fs::write(dir.join("journals/2026_09_03.md"), b"- b4 journal probe\n").unwrap();
    let graph = Graph::open(&dir);
    graph
        .attach_direct_projection(dir.join(".b4-measurement/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    wait_for_direct_query_projection(&graph);
    let (pages, blocks) = graph.with_pages(|pages| {
        fn count_blocks(blocks: &[DocBlock]) -> usize {
            blocks
                .iter()
                .map(|block| 1 + count_blocks(&block.children))
                .sum()
        }
        (
            pages.len(),
            pages
                .iter()
                .map(|(_, document)| count_blocks(&document.roots))
                .sum(),
        )
    });
    (dir, graph, pages, blocks)
}

fn wait_for_direct_query_projection(graph: &Graph) {
    let started = Instant::now();
    while !graph.direct_projection_ready_test() {
        // A budget that reports only "did not converge" costs a whole rerun to
        // learn anything. This fixture takes 0.04 s alone and has blown the
        // 60 s budget under contention -- a 1500x spread -- so the one thing
        // the failure must say is WHICH state it was stuck in: `Working` is a
        // slow machine and the budget is wrong, `Stale` or `Stopped` is a
        // product defect that no amount of waiting would have fixed.
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "Direct query test projection did not converge: progress={:?}, cache generation {}",
            graph.direct_projection_progress(),
            graph.cache_generation(),
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn direct_query_bench_edit(graph: &Graph, serial: usize) {
    let mut page = graph
        .load_by_path("pages/B4 Measurement Target.md")
        .unwrap()
        .unwrap();
    page.blocks[0].raw =
        format!("TODO needle [[B4 Measurement Target]] #work variant-{serial}\nstatus:: active");
    graph
        .save_page(&page, page.rev.as_deref())
        .expect("Direct query benchmark content-only save");
}

fn direct_query_bench_sample(graph: &Graph, query: &str) -> Duration {
    let started = Instant::now();
    // RET2: an unready projection refuses with a typed error instead of
    // walking, so an immediate-after-edit sample measures the refusal — which
    // is exactly what the app measures at that instant too.
    let result = graph.run_query_bounded(query, 20_000, 32 * 1024 * 1024);
    std::hint::black_box(result.map(|answer| (answer.total, answer.exceeded)).ok());
    started.elapsed()
}

fn direct_query_bench_ready_sample(graph: &Graph, query: &str) -> Duration {
    let started = Instant::now();
    let answer = graph
        .run_query_bounded(query, 20_000, 32 * 1024 * 1024)
        .expect("a ready Direct projection must answer the benchmark query");
    std::hint::black_box((answer.total, answer.exceeded));
    started.elapsed()
}

fn direct_query_bench_report(
    class: &str,
    phase: &str,
    samples: &mut [Duration],
    pages: usize,
    blocks: usize,
    statement_reads: u64,
) {
    samples.sort();
    let ms = |duration: Duration| duration.as_secs_f64() * 1_000.0;
    println!(
        "b4_query class={class} phase={phase} median_ms={:.6} p95_ms={:.6} max_ms={:.6} rounds={} pages={pages} blocks={blocks} statement_reads={statement_reads}",
        ms(samples[samples.len() / 2]),
        ms(samples[samples.len() * 95 / 100]),
        ms(samples[samples.len() - 1]),
        samples.len(),
    );
}

/// B4 step 0 measurement. This release-only ignored benchmark compares
/// repeated evaluation before and after edits for each sequencing class,
/// measures projection readiness immediately after a Direct delta, and
/// records acquired-image execution. Point it at a copied graph with
/// TINE_DIRECT_QUERY_BENCH_GRAPH_COPY; graph content is never printed.
#[test]
#[ignore = "manual benchmark: Direct query classes, facets, and invalidation"]
fn direct_query_latency_manual_benchmark() {
    assert!(
        !cfg!(debug_assertions),
        "release-only; run cargo test -p tine-core --release --lib direct_query_latency_manual_benchmark -- --ignored --nocapture --test-threads=1"
    );
    let rounds = std::env::var("TINE_DIRECT_QUERY_BENCH_ROUNDS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(9)
        .max(3);
    let (dir, graph, pages, blocks) = direct_query_bench_open();
    let classes = [
        ("sparse_task", "(task TODO)"),
        ("page_ref", "(page-ref \"B4 Measurement Target\")"),
        (
            "task_non_sparse",
            "(and (task TODO) (page \"B4 Measurement Target\"))",
        ),
        ("block_property", "(property \"status\" \"active\")"),
        ("page_property", "(page-property \"category\" \"work\")"),
        ("page_tags", "(page-tags \"work\")"),
        ("page", "(page \"B4 Measurement Target\")"),
        ("namespace", "(namespace B4)"),
        ("journal", "(journal)"),
        (
            "mixed_and",
            "(and (property \"status\" \"active\") (page \"B4 Measurement Target\"))",
        ),
        (
            "complete_or",
            "(or (page \"B4 Measurement Target\") (namespace B4))",
        ),
        ("plain_text", "\"needle\""),
        ("friendly_search", "(search \"needle\")"),
        (
            "boolean_composition",
            "(and \"needle\" (page-ref \"B4 Measurement Target\"))",
        ),
    ];
    let mut serial = 0;
    for (class, query) in classes {
        direct_query_bench_ready_sample(&graph, query);
        let repeated_statements_before = graph.direct_projection_statement_reads_test();
        let mut repeated = (0..rounds)
            .map(|_| direct_query_bench_ready_sample(&graph, query))
            .collect::<Vec<_>>();
        let repeated_statement_reads = graph
            .direct_projection_statement_reads_test()
            .saturating_sub(repeated_statements_before);
        direct_query_bench_report(
            class,
            "repeat",
            &mut repeated,
            pages,
            blocks,
            repeated_statement_reads,
        );

        let statements_before = graph.direct_projection_statement_reads_test();
        let mut invalidated = Vec::with_capacity(rounds);
        for sample in 0..rounds {
            serial += 1;
            direct_query_bench_edit(&graph, serial);
            wait_for_direct_query_projection(&graph);
            graph.reset_direct_projection_candidate_probe_test();
            let fallback_before = graph.direct_projection_fallback_reads_test();
            let statement_before = graph.direct_projection_statement_reads_test();
            let elapsed = direct_query_bench_ready_sample(&graph, query);
            let statement_queries_completed = graph
                .direct_projection_statement_reads_test()
                .saturating_sub(statement_before);
            let fallback_reads = graph
                .direct_projection_fallback_reads_test()
                .saturating_sub(fallback_before);
            let full_graph_evaluations = crate::query::full_graph_query_evaluations();
            // RET2 retired the candidate route, so there is no candidate page
            // set left to report; the hydration census is what the dispatched
            // query still materializes.
            let evaluated_pages = graph.direct_projection_hydrated_pages_test().len();
            println!(
                "b4_query_sample class={class} run={} sample={} statementQueriesCompleted={statement_queries_completed} fallbackReads={fallback_reads} fullGraphEvaluations={full_graph_evaluations} evaluatedPages={evaluated_pages} medianMs={:.6}",
                std::env::var("TINE_B4_QUERY_BENCH_RUN").unwrap_or_else(|_| "1".into()),
                sample + 1,
                elapsed.as_secs_f64() * 1_000.0,
            );
            invalidated.push(elapsed);
        }
        let statement_reads = graph
            .direct_projection_statement_reads_test()
            .saturating_sub(statements_before);
        direct_query_bench_report(
            class,
            "invalidated_ready",
            &mut invalidated,
            pages,
            blocks,
            statement_reads,
        );
    }

    let mut ready_hits = 0_usize;
    let mut ready_misses = 0_usize;
    let statements_before = graph.direct_projection_statement_reads_test();
    let mut immediate = Vec::with_capacity(rounds);
    for save in 0..rounds {
        serial += 1;
        direct_query_bench_edit(&graph, serial);
        let generation = graph.cache_generation();
        let immediate_ready = graph.direct_projection_ready_test();
        if immediate_ready {
            ready_hits += 1;
        } else {
            ready_misses += 1;
        }
        immediate.push(direct_query_bench_sample(&graph, "(task TODO)"));
        let readiness_started = Instant::now();
        wait_for_direct_query_projection(&graph);
        let ready_latency_ms = readiness_started.elapsed().as_secs_f64() * 1_000.0;
        let oracle =
            crate::query::run_query_bounded(&graph, "(task TODO)", 20_000, 32 * 1024 * 1024);
        let statement_before = graph.direct_projection_statement_reads_test();
        let fallback_before = graph.direct_projection_fallback_reads_test();
        let actual = graph
            .run_query_bounded("(task TODO)", 20_000, 32 * 1024 * 1024)
            .expect("the converged projection answers the public bounded route");
        let oracle_equal = (actual.total, actual.exceeded) == (oracle.total, oracle.exceeded)
            && serde_json::to_vec(actual.groups.as_ref()).unwrap()
                == serde_json::to_vec(&oracle.groups).unwrap();
        println!(
            "b4_readiness save={}-{} generation={generation} immediate_ready={immediate_ready} ready_latency_ms={ready_latency_ms:.6} terminal_event=worker_apply_complete statement_reads={} fallback_reads={} oracle_equal={oracle_equal}",
            std::env::var("TINE_B4_QUERY_BENCH_RUN").unwrap_or_else(|_| "1".into()),
            save + 1,
            graph.direct_projection_statement_reads_test().saturating_sub(statement_before),
            graph.direct_projection_fallback_reads_test().saturating_sub(fallback_before),
        );
    }
    let statement_reads = graph
        .direct_projection_statement_reads_test()
        .saturating_sub(statements_before);
    direct_query_bench_report(
        "sparse_task",
        "data_rev_immediate",
        &mut immediate,
        pages,
        blocks,
        statement_reads,
    );
    println!(
        "b4_projection_hit_rate samples={} ready_hits={ready_hits} ready_misses={ready_misses} statement_reads={statement_reads}",
        ready_hits + ready_misses,
    );

    let today = crate::date::JournalDate::today();
    let queries = [
        "(task TODO)",
        "\"needle\"",
        "\"b4-never-present\"",
        "(page-ref \"B4 Measurement Target\")",
    ];
    let acquire_rows = |day| {
        queries
            .iter()
            .map(|query| {
                simple_query_pre_view_rows_at(&graph, query, day, 20_000, 32 * 1024 * 1024)
            })
            .collect::<Vec<_>>()
    };
    let report_new_image = |edit: &str,
                            before: &[Vec<RefGroup>],
                            after: &[Vec<RefGroup>],
                            generation_before: u64| {
        let outputs_equal = before.iter().zip(after).all(|(before, after)| {
            serde_json::to_vec(before).unwrap() == serde_json::to_vec(after).unwrap()
        });
        println!(
            "b4_acquired_image edit={edit} queries={} executions={} outputs_equal={outputs_equal} cache_gen_before={generation_before} cache_gen_after={}",
            before.len(),
            after.len(),
            graph.cache_generation(),
        );
        assert!(outputs_equal);
    };

    let before = acquire_rows(today);
    let same_image = acquire_rows(today);
    assert_eq!(
        serde_json::to_vec(&before).unwrap(),
        serde_json::to_vec(&same_image).unwrap(),
        "same-image executions must agree"
    );
    let generation_before = graph.cache_generation();
    let mut unrelated = graph
        .load_by_path("pages/B4 Measurement Unrelated.md")
        .unwrap()
        .unwrap();
    unrelated.blocks[0].raw = "b4-unrelated-after".into();
    graph
        .save_page(&unrelated, unrelated.rev.as_deref())
        .unwrap();
    wait_for_direct_query_projection(&graph);
    let after = acquire_rows(today);
    report_new_image("content_only", &before, &after, generation_before);

    let before = acquire_rows(today);
    let generation_before = graph.cache_generation();
    let mut unrelated = graph
        .load_by_path("pages/B4 Measurement Unrelated.md")
        .unwrap()
        .unwrap();
    unrelated.pre_block = Some("alias:: B4 Measurement Alias\n".into());
    graph
        .save_page(&unrelated, unrelated.rev.as_deref())
        .unwrap();
    wait_for_direct_query_projection(&graph);
    let after = acquire_rows(today);
    report_new_image("alias_change", &before, &after, generation_before);

    let before = acquire_rows(today);
    let generation_before = graph.cache_generation();
    let mut new_page = direct_save_bench_new_page("B4 Measurement New Page");
    new_page.blocks[0].id = Uuid::from_u128(0xb400_0000_0000_0000_0000_0000_0000_0001).to_string();
    graph.save_page(&new_page, None).unwrap();
    wait_for_direct_query_projection(&graph);
    let after = acquire_rows(today);
    report_new_image("page_set_change", &before, &after, generation_before);

    let before = acquire_rows(today);
    let same_day = acquire_rows(today);
    assert_eq!(
        serde_json::to_vec(&before).unwrap(),
        serde_json::to_vec(&same_day).unwrap()
    );
    let generation_before = graph.cache_generation();
    let after = acquire_rows(today.add_days(1));
    report_new_image("explicit_day_change", &before, &after, generation_before);

    let facet_sizes = std::env::var("TINE_DIRECT_QUERY_BENCH_FACET_SIZES")
        .unwrap_or_else(|_| "1000,4000".into())
        .split(',')
        .map(|value| value.trim().parse::<usize>().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(facet_sizes.len(), 2, "facet benchmark requires two sizes");
    for size in facet_sizes {
        let (facet_dir, facet_graph) = direct_save_bench_graph("direct-query-facets", size);
        facet_graph
            .attach_direct_projection(facet_dir.join(".b4-facets/projection.sqlite"))
            .unwrap();
        wait_for_direct_query_projection(&facet_graph);
        let facet_blocks = size.saturating_mul(24).saturating_add(1);
        let mut query_facets = Vec::with_capacity(rounds);
        let mut autocomplete = Vec::with_capacity(rounds);
        for _ in 0..rounds {
            let started = Instant::now();
            std::hint::black_box(facet_graph.property_facets());
            query_facets.push(started.elapsed());
            let started = Instant::now();
            std::hint::black_box(
                facet_graph.autocomplete_property_facets_bounded(usize::MAX, usize::MAX),
            );
            autocomplete.push(started.elapsed());
        }
        for (family, samples) in [
            ("query_facets", &mut query_facets),
            ("autocomplete_property_facets", &mut autocomplete),
        ] {
            samples.sort();
            println!(
                "b4_facet family={family} pages={} blocks={facet_blocks} median_ms={:.6} p95_ms={:.6} max_ms={:.6} rounds={}",
                size + 1,
                samples[samples.len() / 2].as_secs_f64() * 1_000.0,
                samples[samples.len() * 95 / 100].as_secs_f64() * 1_000.0,
                samples[samples.len() - 1].as_secs_f64() * 1_000.0,
                samples.len(),
            );
        }
        let _ = fs::remove_dir_all(&facet_dir);
    }

    let _ = fs::remove_dir_all(&dir);
}

fn copy_directory_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_directory_tree(&entry.path(), &target);
        } else if entry.file_type().unwrap().is_file() {
            let _ = fs::copy(entry.path(), target);
        }
    }
}

/// The watcher's routing predicates must admit exactly what discovery
/// admits, plus conflict copies (which are never cached as pages but must
/// still refresh the conflicts panel). GH #268 was the gap between the two.
#[test]
fn watch_predicates_track_the_same_scope_discovery_walks() {
    let dir = scratch("watch-predicate-scope");
    fs::create_dir_all(dir.join("Archive/Deep")).unwrap();
    fs::create_dir_all(dir.join("assets")).unwrap();
    fs::create_dir_all(dir.join(".hidden")).unwrap();
    fs::write(dir.join("pages/Page.md"), b"- p\n").unwrap();
    fs::write(dir.join("top.md"), b"- t\n").unwrap();
    fs::write(dir.join("Archive/Deep/deep.org"), b"* d\n").unwrap();
    let graph = Graph::open(&dir);

    for relative in [
        "pages/Page.md",
        "journals/2026_08_06.md",
        "top.md",
        "Archive/Deep/deep.org",
        // Not present on disk: the predicate is lexical on purpose, so a
        // deletion routes through exactly the same test as a creation.
        "Archive/Gone.md",
        // A Syncthing conflict copy is not eligible text, but its arrival
        // still has to reach the conflicts panel.
        "pages/Page.sync-conflict-20260806-101500-ABCDEFG.md",
    ] {
        assert!(
            graph.graph_text_watch_relevant(&dir.join(relative)),
            "{relative} must be routed to its graph"
        );
    }

    for relative in [
        "assets/image.md",
        ".hidden/skip.md",
        "logseq/bak/old.md",
        "pages/notes.txt",
        "pages",
    ] {
        assert!(
            !graph.graph_text_watch_relevant(&dir.join(relative)),
            "{relative} must not be routed as graph text"
        );
    }
    assert!(
        !graph.graph_text_watch_relevant(Path::new("/elsewhere/pages/Other.md")),
        "a path outside the graph root belongs to another graph, or none"
    );

    // Unclassified paths (directory moves) force a full scan, but only where
    // eligible text could live.
    for relative in ["pages/Moved", "Archive", "top.md"] {
        assert!(
            graph.graph_text_watch_could_contain(&dir.join(relative)),
            "{relative} could contain graph text"
        );
    }
    for relative in ["assets", "assets/pictures", ".git/objects", "logseq/bak"] {
        assert!(
            !graph.graph_text_watch_could_contain(&dir.join(relative)),
            "{relative} is excluded -- a move there must not rescan the graph"
        );
    }

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn graph_text_byte_verification_uses_real_nested_sources_without_mutation() {
    let dir = scratch("graph-text-byte-verification");
    fs::create_dir_all(dir.join("pages/nested")).unwrap();
    fs::create_dir_all(dir.join("archive/deep")).unwrap();
    fs::write(
        dir.join("pages/nested/Unicode 題.md"),
        b"- exact\r\nbytes\n",
    )
    .unwrap();
    fs::write(dir.join("archive/deep/Elsewhere.org"), b"* elsewhere\n").unwrap();
    fs::write(dir.join("pages/.hidden.md"), b"- private\n").unwrap();
    fs::write(dir.join("archive/deep/not-graph.txt"), b"ignored\n").unwrap();
    let graph = Graph::open(&dir);

    let paths = graph.graph_text_source_paths().unwrap();
    assert!(paths.contains(&"pages/nested/Unicode 題.md".to_owned()));
    assert!(paths.contains(&"archive/deep/Elsewhere.org".to_owned()));
    assert!(!paths.iter().any(|path| path.contains(".hidden.md")));
    assert!(!paths.iter().any(|path| path.ends_with("not-graph.txt")));

    let before = fs::read(dir.join("pages/nested/Unicode 題.md")).unwrap();
    let result = graph
        .digest_graph_text_source("pages/nested/Unicode 題.md", &AtomicBool::new(false))
        .unwrap();
    assert_eq!(result.length, before.len() as u64);
    assert_eq!(result.digest, format!("{:x}", Sha256::digest(&before)));
    assert_eq!(fs::read(dir.join(&result.path)).unwrap(), before);
    assert_eq!(
        graph
            .digest_graph_text_source("pages/nested/Unicode 題.md", &AtomicBool::new(true),)
            .unwrap_err()
            .kind(),
        io::ErrorKind::Interrupted
    );
    assert_eq!(fs::read(dir.join(&result.path)).unwrap(), before);

    let _ = fs::remove_dir_all(&dir);
}

/// A cross-directory move syncs BOTH the source and the destination chain
/// (`graph_text_move_noreplace_validated`). Each of those is one barrier on
/// the directory whose entry list actually changed — the source loses a
/// name, the destination gains one — so the depth of either chain is free.
///
/// This drives the Direct Files path deliberately: `rename_file_to_page`
/// is a real user operation on a Direct graph.
#[test]
fn a_cross_directory_move_flushes_one_directory_per_side() {
    fn move_barriers(tag: &str, source_rel: &str, new_name: &str) -> u64 {
        let dir = scratch(tag);
        fs::create_dir_all(dir.join(source_rel).parent().unwrap()).unwrap();
        fs::write(dir.join(source_rel), "- loose\n").unwrap();
        let graph = Graph::open(&dir);

        let session = crate::durability_counters::BarrierSession::begin();
        graph
            .rename_file_to_page(source_rel, new_name)
            .expect("the rescue rename under measurement must succeed");
        let counted = session
            .counts()
            .get(crate::durability_counters::Barrier::Directory);
        crate::durability_counters::BarrierSession::detach_current_thread();

        assert!(dir.join("pages").join(format!("{new_name}.md")).is_file());
        assert!(!dir.join(source_rel).exists());
        let _ = fs::remove_dir_all(&dir);
        counted
    }

    let shallow = move_barriers("projection-move-shallow", "journals/Loose.md", "Rescued");
    let deep = move_barriers(
        "projection-move-deep",
        "pages/one/two/three/Loose.md",
        "Rescued",
    );

    assert!(shallow > 0, "a move must still take its directory barriers");
    assert_eq!(
            deep, shallow,
            "the depth of the source chain must not cost directory barriers: a three-deep              source took {deep} and a one-deep source took {shallow}"
        );
}

#[cfg(windows)]
#[test]
fn projection_missing_capture_rejects_reparse_intermediate_without_escape() {
    use std::os::windows::fs::symlink_dir;

    let dir = scratch("projection-reparse-intermediate");
    let outside = scratch("projection-reparse-intermediate-outside");
    symlink_dir(&outside, dir.join("pages/linked")).unwrap();
    let relative = "pages/linked/deep/Projection.md";
    let graph = Graph::open(&dir);

    assert!(graph
        .read_projection_input(&GraphTextPath::parse(relative).unwrap())
        .is_err());
    // The write side refuses at the same admission boundary. Its exact-byte
    // writer went with Managed Storage (ADR 0066), so the assertion names what
    // that writer called first: resolving a target through the reparse
    // intermediate must fail before any byte is published.
    let refused = match graph.admit_retained_graph_text_writer() {
        Ok(permit) => graph
            .graph_text_target(&permit, &dir.join(relative), false)
            .is_err(),
        Err(_) => true,
    };
    assert!(
        refused,
        "a reparse intermediate must not resolve to a writable graph-text target"
    );
    assert!(!outside.join("deep/Projection.md").exists());
    assert!(dir.join("pages/linked").symlink_metadata().is_ok());

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&outside);
}

#[test]
fn configured_root_helper_stays_inert_while_private_present_decoder_uses_bytes() {
    let root = scratch("admission-inert-helper-regression");
    fs::create_dir_all(root.join("logseq")).unwrap();
    fs::write(
        root.join("logseq/config.edn"),
        "{:pages-directory \"content/pages\"\n\
              :journals-directory \"content/journals\"\n\
              :journal/file-name-format \"dd-MM-yyyy\"\n\
              :journal/page-title-format \"yyyy-MM-dd\"}\n",
    )
    .unwrap();
    let graph = Graph::open(&root);
    let path = GraphTextPath::parse("content/pages/25-07-2026.md").unwrap();

    let configured = graph.graph_text_entry_for_graph_text_path(&path).unwrap();
    assert_eq!(configured.kind, PageKind::Journal);
    assert_eq!(configured.name, "2026-07-25");
    assert!(configured.date_key.is_some());

    let bytes = b"title:: 26-07-2026\n\n- parser-owned title\n";
    let content = std::str::from_utf8(bytes).unwrap();
    let permit = graph_text_parse_budget_permit(&graph, &path, content).unwrap();
    let (present, format) = graph
        .decode_present_graph_text(&path, bytes, permit)
        .unwrap();
    assert_eq!(present.kind, PageKind::Journal);
    assert_eq!(present.name, "2026-07-26");
    assert!(present.date_key.is_some());
    assert_eq!(format, Format::Md);

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn graph_text_entry_decoder_uses_og_filename_semantics_outside_configured_roots() {
    let root = scratch("graph-entry-nonstandard-layout");
    fs::create_dir_all(root.join("logseq")).unwrap();
    fs::write(
        root.join("logseq/config.edn"),
        "{:journal/file-name-format \"dd-MM-yyyy\"\n\
              :journal/page-title-format \"yyyy-MM-dd\"}\n",
    )
    .unwrap();
    let graph = Graph::open(&root);

    // OG walks the whole graph directory and derives the page title from the
    // last path component only, so a nested page outside the configured
    // roots keeps its exact nested spelling and its file-name title.
    let nested = GraphTextPath::parse("archive/2024/client notes/Ünicode Page.md").unwrap();
    let entry = graph.graph_text_entry_for_graph_text_path(&nested).unwrap();
    assert_eq!(entry.kind, PageKind::Page);
    assert_eq!(entry.name, "Ünicode Page");
    assert_eq!(entry.date_key, None);
    assert_eq!(entry.rel_path, "archive/2024/client notes/Ünicode Page.md");
    assert_eq!(
        entry.path,
        root.join("archive/2024/client notes/Ünicode Page.md")
    );

    // OG decides journal-ness by parsing that title as a date, never by the
    // containing directory.
    let journal = GraphTextPath::parse("archive/2024/25-07-2026.org").unwrap();
    let entry = graph
        .graph_text_entry_for_graph_text_path(&journal)
        .unwrap();
    assert_eq!(entry.kind, PageKind::Journal);
    assert_eq!(entry.name, "2026-07-25");
    assert!(entry.date_key.is_some());
    assert_eq!(entry.rel_path, "archive/2024/25-07-2026.org");

    // A graph-root file is equally ordinary graph text for OG.
    let top = GraphTextPath::parse("Top Level.md").unwrap();
    let entry = graph.graph_text_entry_for_graph_text_path(&top).unwrap();
    assert_eq!(entry.kind, PageKind::Page);
    assert_eq!(entry.name, "Top Level");

    // All supported graph-text extensions, including case variants, keep
    // the same OG filename semantics outside configured roots.
    for (relative, expected_name) in [
        ("archive/Lower Md.md", "Lower Md"),
        ("archive/Lower Markdown.markdown", "Lower Markdown"),
        ("archive/Lower Org.org", "Lower Org"),
        ("archive/Upper Md.MD", "Upper Md"),
        ("archive/Upper Markdown.MARKDOWN", "Upper Markdown"),
        ("archive/Upper Org.ORG", "Upper Org"),
    ] {
        let path = GraphTextPath::parse(relative).unwrap();
        let entry = graph.graph_text_entry_for_graph_text_path(&path).unwrap();
        assert_eq!(entry.kind, PageKind::Page, "{relative}");
        assert_eq!(entry.name, expected_name, "{relative}");
        assert_eq!(entry.rel_path, relative, "{relative}");
    }

    // Containers OG itself ignores, hidden paths, provider conflict copies,
    // and spellings the guarded sparse writer cannot project stay refused.
    for refused in [
        "assets/note.md",
        "publish/note.md",
        "published-queries/open-tasks/pages/note.md",
        ".tine-sync/note.md",
        "logseq/bak/pages/note.md",
        "logseq/version-files/note.md",
        "node_modules/pkg/readme.md",
        ".hidden/note.md",
        "archive/.hidden/note.md",
        "archive/note.sync-conflict-20260726-120000-ABCDEFG.md",
    ] {
        assert!(
            graph
                .graph_text_entry_for_graph_text_path(&GraphTextPath::parse(refused).unwrap())
                .is_err(),
            "accepted {refused}"
        );
    }
    for invalid in ["archive/note.txt", "archive/../escape.md"] {
        assert!(GraphTextPath::parse(invalid).is_err(), "accepted {invalid}");
    }

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn graph_text_exact_path_authority_preserves_root_nested_and_markdown_spelling() {
    let root = scratch("admission-target");
    let graph = Graph::open(&root);
    let root_target = graph.graph_text_exact_path("Root.MarkDown", true).unwrap();
    assert!(root_target.parent_components.is_empty());
    assert_eq!(
        root_target.graph_text_path.as_ref().unwrap().as_str(),
        "Root.MarkDown"
    );
    assert_eq!(root_target.filename, "Root.MarkDown");
    assert_eq!(
        Format::from_path(Path::new(&root_target.filename)),
        Format::Md
    );
    assert_eq!(
        graph
            .graph_text_event_parent(&root_target)
            .unwrap()
            .chain
            .len(),
        1
    );

    fs::create_dir_all(root.join("archive/client")).unwrap();
    let nested = graph
        .graph_text_exact_path("archive/client/Plan.Markdown", true)
        .unwrap();
    assert_eq!(nested.parent_components, ["archive", "client"]);
    assert_eq!(
        nested.graph_text_path.as_ref().unwrap().as_str(),
        "archive/client/Plan.Markdown"
    );
    assert_eq!(nested.filename, "Plan.Markdown");
    assert_eq!(Format::from_path(Path::new(&nested.filename)), Format::Md);
    assert_eq!(
        graph.graph_text_event_parent(&nested).unwrap().chain.len(),
        3
    );
    assert!(graph
        .graph_text_exact_path("archive/client/alias.bin", false)
        .is_ok());
    assert!(graph
        .graph_text_exact_path("archive/client/alias.bin", true)
        .is_err());
    let root_projection = graph.projection_page_target("Root.markdown").unwrap();
    assert!(root_projection.parent_components.is_empty());
    assert_eq!(root_projection.filename, "Root.markdown");
    assert_eq!(root_projection.absolute_path, root.join("Root.markdown"));
    let nested_projection = graph
        .projection_page_target("archive/client/Plan.markdown")
        .unwrap();
    assert_eq!(nested_projection.parent_components, ["archive", "client"]);
    assert_eq!(nested_projection.filename, "Plan.markdown");
    assert_eq!(
        nested_projection.absolute_path,
        root.join("archive/client/Plan.markdown")
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn projection_target_accepts_supported_graph_text_outside_configured_roots() {
    let root = scratch("projection-target-nonstandard-layout");
    let graph = Graph::open(&root);

    // OG reads and rewrites ordinary graph text wherever it already lives,
    // so the guarded writer must address the exact nested spelling instead
    // of refusing it or relocating it into a configured root.
    let nested = graph
        .projection_page_target("archive/2024/client notes/Ünicode Page.md")
        .unwrap();
    assert_eq!(
        nested.parent_components,
        ["archive", "2024", "client notes"]
    );
    assert_eq!(nested.filename, "Ünicode Page.md");
    assert_eq!(
        nested.absolute_path,
        root.join("archive/2024/client notes/Ünicode Page.md")
    );

    let top = graph.projection_page_target("Top Level.org").unwrap();
    assert!(top.parent_components.is_empty());
    assert_eq!(top.filename, "Top Level.org");

    // Configured roots keep working exactly as before.
    assert!(graph.projection_page_target("pages/Plain.md").is_ok());
    assert!(graph
        .projection_page_target("journals/2026_07_25.md")
        .is_ok());

    // Containers outside the graph-text scope, traversals and unsupported
    // spellings stay refused.
    for refused in [
        "assets/note.md",
        "publish/note.md",
        "published-queries/open-tasks/pages/note.md",
        ".tine-sync/note.md",
        "logseq/.recycle/note.md",
        "logseq/bak/pages/note.md",
        "logseq/.tine-trash/note.md",
        "node_modules/pkg/readme.md",
        ".hidden/note.md",
        "archive/.hidden/note.md",
        "archive/note.sync-conflict-20260726-120000-ABCDEFG.md",
        "archive/../escape.md",
        "archive/.md",
    ] {
        assert!(
            graph.projection_page_target(refused).is_err(),
            "accepted {refused}"
        );
    }

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn projection_twin_check_uses_only_bounded_direct_metadata_lookups() {
    let source = crate::test_support::model_module_source();
    let shape = source
        .split_once("fn ensure_projection_target_shape(")
        .expect("projection target-shape function")
        .1
        .split_once("fn ensure_projection_parent_binding(")
        .expect("next projection function")
        .0;

    assert!(!shape.contains(".entries("));
    assert!(!shape.contains("read_dir("));
    assert!(shape.contains("for extension in LOGSEQ_TEXT_EXTENSIONS"));
    assert!(shape.contains("projection_optional_regular_metadata(parent.final_dir(), &sibling)"));
}

#[test]
fn projection_twin_check_covers_all_supported_extensions_and_preserves_files() {
    let root = scratch("projection-lowercase-twins");
    let graph = Graph::open(&root);

    for (stem, target_extension, twin_extension) in [
        ("MdMarkdown", "md", "markdown"),
        ("MarkdownOrg", "markdown", "org"),
        ("OrgMd", "org", "md"),
    ] {
        let target_relative = format!("pages/{stem}.{target_extension}");
        let target_path = root.join(&target_relative);
        let twin_path = root.join(format!("pages/{stem}.{twin_extension}"));
        let target_bytes = format!("- {target_extension} target\n").into_bytes();
        let twin_bytes = format!("- {twin_extension} twin\n").into_bytes();
        fs::write(&target_path, &target_bytes).unwrap();
        fs::write(&twin_path, &twin_bytes).unwrap();

        let target = graph.projection_page_target(&target_relative).unwrap();
        let parent = graph.projection_parent(&target).unwrap();
        graph
            .ensure_projection_target_shape(&parent, &target)
            .unwrap();

        assert_eq!(fs::read(&target_path).unwrap(), target_bytes);
        assert_eq!(fs::read(&twin_path).unwrap(), twin_bytes);
    }

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn admission_persistent_avl_ordered_keys_stay_logarithmic_and_share_large_values() {
    fn ordered_work(entries: usize) -> (GraphTextAdmissionTestCounters, u8) {
        reset_graph_text_admission_test_counters();
        let mut map = PersistentMap::default();
        for key in 0..entries {
            map.insert(key, key);
        }
        (
            graph_text_admission_test_counters(),
            PersistentMap::<usize, usize>::height(&map.root),
        )
    }

    let (small, small_height) = ordered_work(1024);
    let (large, large_height) = ordered_work(4096);
    assert!(small_height <= 2 * 11);
    assert!(large_height <= 2 * 13);
    assert!(
        large.persistent_node_allocations < small.persistent_node_allocations * 6,
        "ordered AVL insertion must remain O(N log N): small={small:?}, large={large:?}"
    );
    assert!(small.persistent_rotations < 2 * 1024);
    assert!(large.persistent_rotations < 2 * 4096);

    let large_group = (0..4096)
        .map(|member| format!("member-{member:04}"))
        .collect::<std::collections::BTreeSet<_>>();
    let mut map = PersistentMap::default();
    map.insert(2048_usize, large_group);
    let snapshot = map.clone();
    let before = map.shared_value(&2048).unwrap();
    reset_graph_text_admission_test_counters();
    map.insert(4096, std::collections::BTreeSet::new());
    let after = map.shared_value(&2048).unwrap();
    let snapshot_value = snapshot.shared_value(&2048).unwrap();
    assert!(Arc::ptr_eq(&before, &after));
    assert!(Arc::ptr_eq(&before, &snapshot_value));
    assert_eq!(
        graph_text_admission_test_counters().persistent_payload_members,
        0,
        "path copying must not walk or clone an untouched owned payload"
    );
}

#[test]
fn admission_semantic_accounting_admits_large_ordinary_text_and_rejects_overlong_title() {
    let root = scratch("admission-realistic-semantic-accounting");
    let graph = Graph::open(&root);
    let path = GraphTextPath::parse("Ordinary.md").unwrap();
    let observed =
        graph_text_observed_semantic_name_upper_bound(&graph, &path, "- ordinary body\n")
            .unwrap()
            .semantic_name_bytes;
    let one_record =
        graph_text_file_record_worst_case_upper_bound(&graph, path.as_str().len() as u64, observed)
            .unwrap();
    let realistic_raw_corpus = 480_u64 * 1024 * 1024;
    assert!(realistic_raw_corpus < GRAPH_TEXT_CAPTURE_LIMITS.raw_bytes);
    assert!(
        checked_mul_bytes(one_record, 4).unwrap() < GRAPH_TEXT_CAPTURE_LIMITS.permanent_index_bytes,
        "ordinary titles must not be charged as four 120 MiB document bodies"
    );

    let overlong = "T".repeat(MAX_GRAPH_TEXT_SEMANTIC_NAME_BYTES as usize + 1);
    let content = format!("title:: {overlong}\n\n- body\n");
    assert!(graph_text_observed_semantic_name_upper_bound(&graph, &path, &content).is_err());

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn bounded_read_stops_file_growth_after_metadata_at_the_ceiling() {
    let root = scratch("bounded-read-growth");
    let path = root.join("pages/growing.md");
    fs::write(&path, b"tiny").unwrap();
    let graph = Graph::open(&root);
    BOUNDED_READ_AFTER_METADATA.with(|hook| {
        let path = path.clone();
        *hook.borrow_mut() = Some(Box::new(move || {
            let file = fs::OpenOptions::new().write(true).open(path)?;
            file.set_len(32)
        }));
    });
    assert!(open_and_read_projection_regular_with_limit(
        graph.projection_root.as_ref().unwrap(),
        "pages/growing.md",
        8,
    )
    .is_err());
    assert_eq!(fs::metadata(path).unwrap().len(), 32);

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn projection_semantic_refusal_marker_excludes_transient_io() {
    let policy = projection_semantic_refusal(
        io::ErrorKind::InvalidData,
        "deterministic serialization policy refusal",
    );
    assert!(is_projection_semantic_refusal(&policy));
    for kind in [
        io::ErrorKind::Interrupted,
        io::ErrorKind::WouldBlock,
        io::ErrorKind::TimedOut,
        io::ErrorKind::PermissionDenied,
    ] {
        assert!(
            !is_projection_semantic_refusal(&io::Error::new(kind, "filesystem failure")),
            "{kind:?} I/O must remain retryable"
        );
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[test]
fn native_case_alias_requires_retirement_before_new_spelling() {
    let dir = scratch("projection-native-case-alias");
    let old = dir.join("pages/foo.md");
    let new = dir.join("pages/Foo.md");
    let base = b"- before\n";
    let target = b"- after\n";
    fs::write(&old, base).unwrap();
    if fs::symlink_metadata(&new).is_err() {
        let _ = fs::remove_dir_all(&dir);
        return;
    }
    let pages = Dir::open_ambient_dir(dir.join("pages"), ambient_authority()).unwrap();
    fs::write(dir.join("pages/staged"), target).unwrap();

    // A new spelling only becomes live through the graph tree's no-clobber
    // publication. On a case-insensitive filesystem the existing `foo.md` IS
    // the destination, so publication must refuse rather than replace it, and
    // the old spelling must be retired first. (Managed Storage's exact-byte
    // writer performed this rename; ADR 0066 removed the writer, not the rule.)
    let conflict = rename_projection_noreplace(&pages, "staged", "Foo.md").unwrap_err();
    assert_eq!(conflict.kind(), io::ErrorKind::AlreadyExists, "{conflict}");
    assert_eq!(fs::read(&old).unwrap(), base);

    fs::remove_file(&old).unwrap();
    rename_projection_noreplace(&pages, "staged", "Foo.md").unwrap();
    assert_eq!(fs::read(&new).unwrap(), target);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn production_projection_has_no_alternate_graph_writer_entrypoint() {
    let source = crate::test_support::model_module_source();
    let forbidden = ["pub(crate) fn write_projection", "_exact"].concat();
    assert!(!source.contains(&forbidden));
    assert!(source.contains("self.serialize_page_document("));
}

/// GH #466. The rule this guard states on failure: every Direct Files graph-text
/// name transition (create, live-name retirement, staged publication, recovery
/// restore, recovery set-aside) goes through `move_graph_text_exact_no_replace`
/// — the exact-byte protocol over the graph tree's own no-clobber rename family
/// (I-16) — and never through `tine_storage::DurableDirectoryPublication`,
/// whose Android arm is hard-link-then-unlink and fails with `EACCES` on the
/// shared storage a Direct Files graph lives in. v0.6.981 shipped exactly that
/// and every Android save failed. Imitate `move_graph_text_exact_no_replace`
/// in `model/projection_rename.rs`; the storage boundary stays for app-private authorities only.
#[test]
fn direct_files_graph_text_publication_uses_the_graph_tree_noreplace_rename() {
    const RULE: &str = "GH #466 / I-16: Direct Files graph-text name transitions use \
        move_graph_text_exact_no_replace (the graph tree's renameat2(RENAME_NOREPLACE) \
        family), never tine-storage's DurableDirectoryPublication, whose Android arm is a \
        hard link that shared storage refuses; imitate move_graph_text_exact_no_replace";
    let source = crate::test_support::model_module_source();
    let create = source
        .split_once("fn graph_text_atomic_create_with_proof(")
        .expect("Direct Files create path")
        .1
        .split_once("fn graph_text_atomic_write_with_conflict(")
        .expect("next Direct Files write function")
        .0;
    assert!(
        create.contains(
            "move_graph_text_exact_no_replace(target.parent(), &temp, &target.filename, bytes)"
        ),
        "{RULE}"
    );
    assert!(!create.contains("DurableDirectoryPublication"), "{RULE}");
    assert!(!create.contains(".move_exact_no_replace("), "{RULE}");

    let write = source
        .split_once("fn graph_text_atomic_write_validated(")
        .expect("Direct Files validated write path")
        .1
        .split_once("\n    /// Replace an existing editor target")
        .expect("Direct Files bounded replacement")
        .0;
    assert!(write.contains("self.graph_text_atomic_replace_bound("));
    assert!(
        write.contains(
            "move_graph_text_exact_no_replace(target.parent(), &temp, &target.filename, bytes)"
        ),
        "{RULE}"
    );
    assert!(!write.contains("DurableDirectoryPublication"), "{RULE}");
    assert!(!write.contains(".move_exact_no_replace("), "{RULE}");
    assert!(!write.contains("target.parent().rename("), "{RULE}");

    let replace = source
        .split_once("fn graph_text_atomic_replace_bound(")
        .expect("Direct Files bounded replacement")
        .1
        .split_once("fn graph_text_move_noreplace(")
        .expect("next projection method")
        .0;
    // The retire/publish closure, the recovery set-aside, and the restore.
    assert!(
        replace.matches("move_graph_text_exact_no_replace(").count() >= 3,
        "{RULE}"
    );
    assert!(!replace.contains("DurableDirectoryPublication"), "{RULE}");
    assert!(!replace.contains(".move_exact_no_replace("), "{RULE}");
}

/// GH #466. The exact-byte protocol the Direct Files name transition carries:
/// a matching source is published under a name nothing else holds, an
/// occupied destination is never replaced, and a source whose bytes are not
/// the expected ones (an external writer got there first) is never published.
#[test]
fn graph_text_exact_move_publishes_expected_bytes_and_refuses_a_replaced_source() {
    let root = scratch("gh466-graph-text-exact-move");
    fs::create_dir_all(&root).unwrap();
    let dir = Dir::open_ambient_dir(&root, ambient_authority()).unwrap();

    dir.write("staged.md", b"- staged\n").unwrap();
    move_graph_text_exact_no_replace(&dir, "staged.md", "Page.md", b"- staged\n").unwrap();
    assert_eq!(fs::read(root.join("Page.md")).unwrap(), b"- staged\n");
    assert!(!root.join("staged.md").exists());

    dir.write("other.md", b"- other\n").unwrap();
    let occupied =
        move_graph_text_exact_no_replace(&dir, "other.md", "Page.md", b"- other\n").unwrap_err();
    assert_eq!(occupied.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(root.join("Page.md")).unwrap(), b"- staged\n");
    assert_eq!(fs::read(root.join("other.md")).unwrap(), b"- other\n");

    let replaced = move_graph_text_exact_no_replace(&dir, "other.md", "Fresh.md", b"- expected\n")
        .unwrap_err();
    assert_eq!(replaced.kind(), io::ErrorKind::AlreadyExists);
    assert!(replaced.to_string().contains("source name"), "{replaced}");
    assert!(!root.join("Fresh.md").exists());
    assert_eq!(fs::read(root.join("other.md")).unwrap(), b"- other\n");

    let _ = fs::remove_dir_all(&root);
}

// ---- #21: path-pinned pages + duplicate-day reconcile ----

/// A graph with a canonical day file AND a title-named stray for the same day,
/// in the user's `EEEE, dd-MM-yyyy` title format. Both resolve to the journal
/// name "Friday, 26-06-2026" — the collision #21 makes addressable by path.
fn dup_day_graph(tag: &str) -> PathBuf {
    let dir = scratch(tag);
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq").join("config.edn"),
        "{:preferred-format \"Org\"\n :journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
    )
    .unwrap();
    fs::write(
        dir.join("journals").join("2026_06_26.org"),
        "* canonical body\n",
    )
    .unwrap();
    fs::write(
        dir.join("journals").join("Friday, 26-06-2026.org"),
        "* stray body\n",
    )
    .unwrap();
    dir
}

#[test]
fn resolve_rel_accepts_graph_files_and_rejects_escapes() {
    let dir = scratch("resolve-rel");
    let g = Graph::open(&dir);
    // Root-level eligible text files are ordinary graph documents.
    assert_eq!(g.resolve_rel("Note.md"), Some(dir.join("Note.md")));
    // Valid: one segment under journals/ or pages/, md/org extension.
    assert_eq!(
        g.resolve_rel("journals/2026_06_26.org"),
        Some(dir.join("journals").join("2026_06_26.org"))
    );
    assert_eq!(
        g.resolve_rel("pages/Note.md"),
        Some(dir.join("pages").join("Note.md"))
    );
    // Valid: nested sub-directories under pages/ (#21) — any depth.
    assert_eq!(
        g.resolve_rel("pages/client-a/foo.md"),
        Some(dir.join("pages").join("client-a").join("foo.md"))
    );
    assert_eq!(
        g.resolve_rel("pages/a/b/c/deep.org"),
        Some(
            dir.join("pages")
                .join("a")
                .join("b")
                .join("c")
                .join("deep.org")
        )
    );
    // Rejections: traversal (incl. FROM a subdir), absolute, empty/`.` segment,
    // reserved/non-text paths, wrong/no extension, and bare directories.
    // Nesting itself is NOT rejected.
    for bad in [
        "../secrets.md",
        "journals/../../etc/passwd.md",
        "pages/../../etc/passwd.md",
        "pages/sub/../../../etc/passwd.md",
        "pages/client-a/../../escape.md",
        "pages/./foo.md",
        "pages/a//b.md",
        "pages/sub/.md",
        "/etc/passwd.md",
        "assets/pic.png",
        "journals/note.txt",
        "journals/",
        "pages/sub/",
        "",
    ] {
        assert_eq!(g.resolve_rel(bad), None, "should reject {bad:?}");
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn page_source_file_prefers_the_recorded_nested_identity() {
    let dir = scratch("page-source-file");
    fs::create_dir_all(dir.join("pages/client-a")).unwrap();
    let canonical = dir.join("pages/Note.md");
    let nested = dir.join("pages/client-a/Note.md");
    fs::write(&canonical, "- canonical\n").unwrap();
    fs::write(&nested, "- nested\n").unwrap();
    let g = Graph::open(&dir);

    assert_eq!(
        g.page_source_file("Note", PageKind::Page, Some("pages/client-a/Note.md"))
            .unwrap(),
        nested.canonicalize().unwrap()
    );
    assert_eq!(
        g.page_source_file("Note", PageKind::Page, None).unwrap(),
        canonical.canonicalize().unwrap()
    );
    assert!(g
        .page_source_file("Note", PageKind::Page, Some("assets/Note.md"))
        .is_err());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn checked_open_rejects_configured_directories_outside_graph() {
    let dir = scratch("checked-open-layout");
    fs::create_dir_all(dir.join("logseq")).unwrap();
    for config in [
        "{:pages-directory \"../outside\"}\n",
        "{:journals-directory \"/tmp/tine-outside\"}\n",
        "{:pages-directory \"pages\\\\escape\"}\n",
    ] {
        fs::write(dir.join("logseq/config.edn"), config).unwrap();
        assert!(Graph::open_checked(&dir).is_err(), "accepted {config:?}");
    }
    fs::write(
        dir.join("logseq/config.edn"),
        "{:pages-directory \"archive/pages\" :journals-directory \"diary\"}\n",
    )
    .unwrap();
    assert!(Graph::open_checked(&dir).is_ok());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn atomic_update_retries_on_external_change_without_losing_it() {
    let dir = scratch("atomic-update-external");
    let path = dir.join("config.edn");
    fs::write(&path, "{:base 1}\n").unwrap();
    let lock = std::sync::Mutex::new(());
    let injected = std::sync::atomic::AtomicBool::new(false);
    atomic_update_with_hooks(
        &path,
        &lock,
        |content| Ok(content.replace('}', " :mine 3}")),
        |_| {
            if !injected.swap(true, std::sync::atomic::Ordering::SeqCst) {
                fs::write(&path, "{:base 1 :external 2}\n").unwrap();
            }
        },
        |_| {},
    )
    .unwrap();
    let final_content = fs::read_to_string(&path).unwrap();
    assert!(final_content.contains(":external 2"));
    assert!(final_content.contains(":mine 3"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn atomic_update_absent_publish_preserves_a_concurrent_creator() {
    let dir = scratch("atomic-update-absent-race");
    let path = dir.join("config.edn");
    let lock = std::sync::Mutex::new(());
    let injected = std::sync::atomic::AtomicBool::new(false);
    atomic_update_with_hooks(
        &path,
        &lock,
        |content| Ok(content.replace('}', " :mine 3}")),
        |_| {},
        |_| {
            if !injected.swap(true, std::sync::atomic::Ordering::SeqCst) {
                fs::write(&path, "{:external 2}\n").unwrap();
            }
        },
    )
    .unwrap();
    let final_content = fs::read_to_string(&path).unwrap();
    assert!(final_content.contains(":external 2"));
    assert!(final_content.contains(":mine 3"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn atomic_update_existing_publish_preserves_a_concurrent_external_write() {
    // A2: the recheck narrows the clobber window but cannot close it. With a
    // baseline already on disk, an external writer (Syncthing delivering a
    // peer's config.edn, Logseq, an editor) landing AFTER the recheck and
    // before the rename used to be overwritten silently - no conflict copy,
    // no refusal, no trace. The publish must be conditional.
    let dir = scratch("atomic-update-existing-race");
    let path = dir.join("config.edn");
    fs::write(&path, "{:base 1}\n").unwrap();
    let lock = std::sync::Mutex::new(());
    let injected = std::sync::atomic::AtomicBool::new(false);
    atomic_update_with_hooks(
        &path,
        &lock,
        |content| Ok(content.replace('}', " :mine 3}")),
        |_| {},
        |_| {
            if !injected.swap(true, std::sync::atomic::Ordering::SeqCst) {
                fs::write(&path, "{:base 1 :external 2}\n").unwrap();
            }
        },
    )
    .unwrap();
    let final_content = fs::read_to_string(&path).unwrap();
    assert!(
        final_content.contains(":external 2"),
        "external bytes were clobbered: {final_content:?}"
    );
    assert!(
        final_content.contains(":mine 3"),
        "our edit was dropped: {final_content:?}"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn conditional_publish_refuses_when_the_file_changed_underneath() {
    let dir = scratch("conditional-publish-refuses");
    let path = dir.join("sidecar.edn");
    fs::write(&path, b"expected").unwrap();
    fs::write(&path, b"external").unwrap();
    let outcome = atomic_replace_expected(&path, b"expected", b"ours").unwrap();
    match outcome {
        AtomicReplaceOutcome::ExternalChanged => {}
        AtomicReplaceOutcome::Published => panic!("published over an external write"),
    }
    assert_eq!(fs::read(&path).unwrap(), b"external", "their bytes stand");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn conditional_publish_leaves_no_retired_file_behind_on_success() {
    let dir = scratch("conditional-publish-clean");
    let path = dir.join("sidecar.edn");
    fs::write(&path, b"expected").unwrap();
    let outcome = atomic_replace_expected(&path, b"expected", b"ours").unwrap();
    assert!(matches!(outcome, AtomicReplaceOutcome::Published));
    assert_eq!(fs::read(&path).unwrap(), b"ours");
    let leftovers: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".retired") || name.contains(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "left {leftovers:?} behind");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_crash_between_retire_and_publish_is_recovered_byte_identical() {
    // The window the protocol opens deliberately: `path` does not exist and
    // its content lives under a `.retired` sibling. A crash there must not
    // look like a deleted file.
    let dir = scratch("retired-crash-recovery");
    let path = dir.join("config.edn");
    fs::write(&path, b"{:base 1}\n").unwrap();
    let crashed = atomic_replace_expected_with_hooks(&path, b"{:base 1}\n", b"{:next 2}\n", || {
        Err(io::Error::other("simulated crash after retire"))
    });
    assert!(crashed.is_err());
    // The unwind restores it; simulate the harder case - a real crash, where
    // nothing runs - by retiring again and abandoning it.
    let retired = dir.join(".config.edn.999.0.retired");
    fs::rename(&path, &retired).unwrap();
    assert!(!path.exists(), "the window is real");

    let recovered = restore_retired_files(&dir, &[dir.clone()]).unwrap();

    assert_eq!(recovered, 1);
    assert_eq!(
        fs::read(&path).unwrap(),
        b"{:base 1}\n",
        "content must come back byte-identical"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recovery_keeps_a_superseded_retired_copy_instead_of_deleting_it() {
    // The publish completed (or an external writer recreated the file), so
    // the retired copy is superseded - but it is still the only copy of
    // those bytes, so it goes to recoverable trash, never to /dev/null.
    let dir = scratch("retired-superseded");
    let path = dir.join("config.edn");
    fs::write(&path, b"current").unwrap();
    fs::write(dir.join(".config.edn.999.0.retired"), b"older").unwrap();

    let recovered = restore_retired_files(&dir, &[dir.clone()]).unwrap();

    assert_eq!(recovered, 0);
    assert_eq!(
        fs::read(&path).unwrap(),
        b"current",
        "current file untouched"
    );
    let trashed = typed_trash_dir(&dir, TrashEntryKind::Conflict).join(".config.edn.999.0.retired");
    assert_eq!(
        fs::read(&trashed).unwrap(),
        b"older",
        "the superseded copy must be recoverable"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn checked_open_restores_one_stranded_editor_recovery_without_guessing() {
    let dir = scratch("editor-recovery-single-restore");
    let recovery = dir.join("pages").join(".Note.md.4242.1.editor-recovery");
    fs::write(&recovery, b"- exact pre-crash bytes\n").unwrap();

    let graph = Graph::open_checked(&dir).unwrap();

    assert_eq!(
        fs::read(dir.join("pages/Note.md")).unwrap(),
        b"- exact pre-crash bytes\n"
    );
    assert!(!recovery.exists());
    assert!(graph.list_pages().iter().any(|entry| entry.name == "Note"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn editor_recovery_names_accept_legacy_and_turn_derived_shapes() {
    assert_eq!(
        editor_recovery_target_name(".Note.md.4242.1.editor-recovery"),
        Some("Note.md")
    );
    assert_eq!(
        editor_recovery_target_name(".Note.md.4242.1.1234abcd.editor-staged-recovery"),
        Some("Note.md")
    );
    assert_eq!(
        editor_recovery_target_name(".Note.md.4242.12345678.editor-recovery"),
        Some("Note.md"),
        "an eight-digit legacy sequence must not be consumed as a turn id"
    );
    for lookalike in [
        ".Note.md.4242.1.short.editor-recovery",
        ".Note.md.4242.1.1234xyz8.editor-recovery",
        ".Note.md.pid.1.1234abcd.editor-recovery",
        ".Note.md.4242.seq.1234abcd.editor-recovery",
    ] {
        assert_eq!(editor_recovery_target_name(lookalike), None, "{lookalike}");
    }
    assert_eq!(
        editor_retired_target_name(".Note.md.4242.1.editor-retired"),
        Some("Note.md")
    );
    for lookalike in [
        ".Note.md.4242.editor-retired",
        ".Note.md.pid.1.editor-retired",
        ".Note.md.4242.seq.editor-retired",
        ".Note.txt.4242.1.editor-retired",
        "Note.md.4242.1.editor-retired",
    ] {
        assert_eq!(editor_retired_target_name(lookalike), None, "{lookalike}");
    }
}

#[test]
fn checked_open_retries_editor_retired_cleanup_without_restoring_it() {
    let dir = scratch("editor-retired-cleanup-retry");
    let live = dir.join("pages/Note.md");
    let retired = dir.join("pages").join(".Note.md.4242.1.editor-retired");
    fs::write(&live, b"- published bytes\n").unwrap();
    fs::write(&retired, b"- displaced bytes\n").unwrap();
    EDITOR_RETIRED_CLEANUP.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(|| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected retired-cleanup unlink failure",
            ))
        }));
    });

    let refused = match Graph::open_checked(&dir) {
        Ok(_) => panic!("checked open ignored a retired-cleanup failure"),
        Err(error) => error,
    };
    assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied, "{refused}");
    assert_eq!(fs::read(&live).unwrap(), b"- published bytes\n");
    assert_eq!(fs::read(&retired).unwrap(), b"- displaced bytes\n");

    let _graph = Graph::open_checked(&dir).unwrap();
    assert_eq!(fs::read(&live).unwrap(), b"- published bytes\n");
    assert!(!retired.exists(), "the next checked open owns the retry");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn editor_retired_cleanup_is_never_document_authority() {
    let dir = scratch("editor-retired-never-authority");
    let retired = dir.join("pages").join(".Note.md.4242.1.editor-retired");
    fs::write(&retired, b"- displaced bytes\n").unwrap();

    let _graph = Graph::open_checked(&dir).unwrap();

    assert!(!dir.join("pages/Note.md").exists());
    assert!(!retired.exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn checked_open_fails_closed_when_the_recovery_name_walk_exceeds_its_bound() {
    struct LimitsReset;
    impl Drop for LimitsReset {
        fn drop(&mut self) {
            GRAPH_TEXT_INVENTORY_LIMITS_OVERRIDE.with(|limits| {
                *limits.borrow_mut() = None;
            });
        }
    }

    let dir = scratch("editor-recovery-walk-bound");
    let _reset = LimitsReset;
    GRAPH_TEXT_INVENTORY_LIMITS_OVERRIDE.with(|limits| {
        *limits.borrow_mut() = Some(GraphTextInventoryLimits {
            all_entries: 0,
            ..GRAPH_TEXT_INVENTORY_LIMITS
        });
    });

    let refused = match Graph::open_checked(&dir) {
        Ok(_) => panic!("bounded recovery walk unexpectedly succeeded"),
        Err(error) => error,
    };
    assert_eq!(refused.kind(), io::ErrorKind::InvalidData);
    assert!(
        refused.to_string().contains("all directory entries"),
        "unexpected bounded-walk error: {refused}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn editor_recovery_sweep_preserves_ambiguous_and_superseded_bytes() {
    // Two distinct claims and no live target are ambiguous. Neither is
    // selected, renamed, or deleted.
    let ambiguous = scratch("editor-recovery-ambiguous");
    let old = ambiguous
        .join("pages")
        .join(".Note.md.4242.1.editor-recovery");
    let edited = ambiguous
        .join("pages")
        .join(".Note.md.4242.2.editor-staged-recovery");
    fs::write(&old, b"- old bytes\n").unwrap();
    fs::write(&edited, b"- edited bytes\n").unwrap();
    let _graph = Graph::open_checked(&ambiguous).unwrap();
    assert!(!ambiguous.join("pages/Note.md").exists());
    assert_eq!(fs::read(&old).unwrap(), b"- old bytes\n");
    assert_eq!(fs::read(&edited).unwrap(), b"- edited bytes\n");

    // If a live winner exists, it stays byte-identical and every stranded
    // artifact moves intact to recoverable conflict trash.
    let superseded = scratch("editor-recovery-superseded");
    let live = superseded.join("pages/Note.md");
    let old = superseded
        .join("pages")
        .join(".Note.md.4242.1.editor-recovery");
    let edited = superseded
        .join("pages")
        .join(".Note.md.4242.2.editor-staged-recovery");
    fs::write(&live, b"- external winner\n").unwrap();
    fs::write(&old, b"- old bytes\n").unwrap();
    fs::write(&edited, b"- edited bytes\n").unwrap();
    let _graph = Graph::open_checked(&superseded).unwrap();
    assert_eq!(fs::read(&live).unwrap(), b"- external winner\n");
    assert!(!old.exists());
    assert!(!edited.exists());
    let recovered = fs::read_dir(typed_trash_dir(&superseded, TrashEntryKind::Conflict))
        .unwrap()
        .flatten()
        .map(|entry| fs::read(entry.path()).unwrap())
        .collect::<Vec<_>>();
    assert!(recovered.contains(&b"- old bytes\n".to_vec()));
    assert!(recovered.contains(&b"- edited bytes\n".to_vec()));

    let _ = fs::remove_dir_all(&ambiguous);
    let _ = fs::remove_dir_all(&superseded);
}

#[test]
fn a_foreground_displacement_crash_is_restored_before_journal_replay() {
    let dir = scratch("editor-recovery-exact-name");
    let staged = dir
        .join("pages")
        .join(".Note.md.4242.1.editor-staged-recovery");
    let lookalike = dir
        .join("pages")
        .join(".Other.md.not-a-pid.1.editor-recovery");
    fs::write(&staged, b"- sole staged bytes\n").unwrap();
    fs::write(&lookalike, b"- ordinary lookalike\n").unwrap();

    let _graph = Graph::open_checked(&dir).unwrap();

    assert_eq!(
        fs::read(dir.join("pages/Note.md")).unwrap(),
        b"- sole staged bytes\n"
    );
    assert_eq!(fs::read(&lookalike).unwrap(), b"- ordinary lookalike\n");
    let _ = fs::remove_dir_all(&dir);
}

/// A recovery artifact with a second link used to make the checked open refuse
/// the whole graph. git-annex's assistant hard-links new files under
/// `.git/annex/watchtmp`, so a crash on an annexed graph could leave exactly
/// that (GH #555). Restoring is a MOVE: the inode keeps its bytes, and so does
/// every other name linked to it, so the refusal defended no in-scope scenario
/// and cost the user their graph.
#[cfg(unix)]
#[test]
fn editor_recovery_sweep_restores_a_multi_link_artifact() {
    let dir = scratch("editor-recovery-hardlink");
    let artifact = dir.join("pages").join(".Note.md.4242.1.editor-recovery");
    fs::write(&artifact, b"- linked bytes\n").unwrap();
    fs::hard_link(&artifact, dir.join("linked-copy")).unwrap();

    let _graph = Graph::open_checked(&dir).expect("a multi-link artifact is restored");

    assert_eq!(
        fs::read(dir.join("pages/Note.md")).unwrap(),
        b"- linked bytes\n"
    );
    assert!(!artifact.exists());
    assert_eq!(
        fs::read(dir.join("linked-copy")).unwrap(),
        b"- linked bytes\n"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_dir_fsync_that_is_merely_unsupported_is_not_a_durability_failure() {
    // The best-effort discard existed because dir fsync genuinely does not
    // exist everywhere. Keep tolerating that, and only that.
    for kind in [
        io::ErrorKind::Unsupported,
        io::ErrorKind::InvalidInput,
        io::ErrorKind::PermissionDenied,
        io::ErrorKind::NotFound,
    ] {
        assert!(
            dir_fsync_is_unsupported(&io::Error::new(kind, "x")),
            "{kind:?}"
        );
    }
    for errno in [9, 13, 21, 22] {
        assert!(dir_fsync_is_unsupported(&io::Error::from_raw_os_error(
            errno
        )));
    }
    // A real durability failure must surface.
    for errno in [5 /* EIO */, 28 /* ENOSPC */] {
        assert!(
            !dir_fsync_is_unsupported(&io::Error::from_raw_os_error(errno)),
            "errno {errno} must not be swallowed"
        );
    }
}

#[test]
fn retired_names_round_trip_to_their_target() {
    assert_eq!(
        retired_target_name(".config.edn.123.7.retired"),
        Some("config.edn")
    );
    assert_eq!(
        retired_target_name(".a.b.c.md.1.2.retired"),
        Some("a.b.c.md")
    );
    assert_eq!(retired_target_name("config.edn"), None);
    assert_eq!(retired_target_name(".config.edn.tmp"), None);
}

#[test]
fn guide_twin_withdrawal_preserves_a_concurrent_markdown_replacement() {
    let dir = scratch("guide-twin-withdrawal-race");
    let graph = Graph::open(&dir);
    GUIDE_TWIN_RACE_CONTENT.with(|content| {
        *content.borrow_mut() = Some(b"* external org twin\n".to_vec());
    });
    WITHDRAW_RACE_REPLACEMENT.with(|replacement| {
        *replacement.borrow_mut() = Some(b"- external markdown replacement\n".to_vec());
    });

    assert!(!graph
        .create_markdown_page_if_absent("Guide", "- bundled guide\n")
        .unwrap());
    assert_eq!(
        fs::read_to_string(dir.join("pages/Guide.md")).unwrap(),
        "- external markdown replacement\n"
    );
    assert_eq!(
        fs::read_to_string(dir.join("pages/Guide.org")).unwrap(),
        "* external org twin\n"
    );
    let _ = fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn checked_open_and_resolve_reject_symlink_escape() {
    use std::os::unix::fs::symlink;
    let dir = scratch("checked-open-symlink");
    let outside =
        std::env::temp_dir().join(format!("tine-checked-open-outside-{}", std::process::id()));
    let _ = fs::remove_dir_all(&outside);
    fs::create_dir_all(&outside).unwrap();
    fs::create_dir_all(dir.join("logseq")).unwrap();
    symlink(&outside, dir.join("pages-link")).unwrap();
    fs::write(
        dir.join("logseq/config.edn"),
        "{:pages-directory \"pages-link\"}\n",
    )
    .unwrap();
    assert!(Graph::open_checked(&dir).is_err());

    fs::create_dir_all(dir.join("pages")).unwrap();
    symlink(&outside, dir.join("pages/escape")).unwrap();
    let g = Graph::open(&dir);
    assert!(g.resolve_rel("pages/escape/foreign.md").is_none());
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&outside);
}

/// A query export is a copy of graph pages living under the graph root, so the
/// scanner must never read it back as pages: an exported `Secret.md` would
/// otherwise reappear as a twin of its own source (and leak into a later
/// export). The fixed exclusion lives in `graph_text_scope::fixed_excluded`.
#[test]
fn published_queries_output_never_becomes_pages() {
    let dir = scratch("published-queries-excluded");
    fs::write(dir.join("pages/Secret.md"), "- real-page\n").unwrap();
    fs::create_dir_all(dir.join("published-queries/open-tasks/pages")).unwrap();
    fs::write(
        dir.join("published-queries/open-tasks/pages/Secret.md"),
        "- exported-copy\n",
    )
    .unwrap();
    fs::write(
        dir.join("published-queries/open-tasks/Loose Page.md"),
        "- exported-loose\n",
    )
    .unwrap();
    let g = Graph::open(&dir);
    let pages = g.list_pages();
    assert_eq!(
        pages.iter().filter(|p| p.name == "Secret").count(),
        1,
        "{pages:?}"
    );
    assert!(
        pages
            .iter()
            .all(|p| !p.rel_path.starts_with("published-queries/")),
        "{pages:?}"
    );
    assert!(g
        .resolve_rel("published-queries/open-tasks/pages/Secret.md")
        .is_none());
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn checked_open_rejects_output_symlink_escapes() {
    use std::os::unix::fs::symlink;
    for output in ["assets", "logseq", "publish"] {
        let dir = scratch(&format!("checked-open-{output}-symlink"));
        let outside = std::env::temp_dir().join(format!(
            "tine-checked-open-{output}-outside-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&outside);
        fs::create_dir_all(&outside).unwrap();
        let output_path = dir.join(output);
        let _ = fs::remove_dir_all(&output_path);
        symlink(&outside, &output_path).unwrap();

        assert!(
            Graph::open_checked(&dir).is_err(),
            "accepted escaped {output} directory"
        );

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&outside);
    }
}

#[cfg(unix)]
#[test]
fn checked_open_accepts_only_the_approved_external_assets_target() {
    use std::os::unix::fs::symlink;
    let dir = scratch("checked-open-approved-assets");
    let outside = std::env::temp_dir().join(format!(
        "tine-checked-open-approved-assets-outside-{}",
        std::process::id()
    ));
    let other = std::env::temp_dir().join(format!(
        "tine-checked-open-approved-assets-other-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&outside);
    let _ = fs::remove_dir_all(&other);
    fs::create_dir_all(&outside).unwrap();
    fs::create_dir_all(&other).unwrap();
    let _ = fs::remove_dir_all(dir.join("assets"));
    symlink(&outside, dir.join("assets")).unwrap();

    assert!(Graph::open_checked(&dir).is_err());
    assert!(Graph::open_checked_with_assets(&dir, Some(&other)).is_err());
    let graph = Graph::open_checked_with_assets(&dir, Some(&outside)).unwrap();
    assert_eq!(graph.assets_path(), outside.canonicalize().unwrap());
    assert_eq!(
        graph.save_asset("approved.txt", b"safe").unwrap(),
        "approved.txt"
    );
    assert_eq!(fs::read(outside.join("approved.txt")).unwrap(), b"safe");

    // Retargeting the graph link cannot redirect an already-open graph: the
    // Graph holds the originally approved canonical capability. A fresh open
    // also fails because the stored approval no longer matches.
    fs::remove_file(dir.join("assets")).unwrap();
    symlink(&other, dir.join("assets")).unwrap();
    assert_eq!(
        graph
            .save_asset("after-retarget.txt", b"still safe")
            .unwrap(),
        "after-retarget.txt"
    );
    assert!(outside.join("after-retarget.txt").exists());
    assert!(!other.join("after-retarget.txt").exists());
    assert!(Graph::open_checked_with_assets(&dir, Some(&outside)).is_err());

    // A nested link inside the approved root remains confined: neither read
    // nor write may follow it into another directory.
    symlink(other.join("secret.txt"), outside.join("escape.txt")).unwrap();
    fs::write(other.join("secret.txt"), b"private").unwrap();
    assert!(graph.read_asset("escape.txt").is_err());
    let area_key = crate::pdf::asset_key("Escaping area.pdf");
    symlink(&other, outside.join(&area_key)).unwrap();
    assert!(graph
        .write_pdf_area_image("Escaping area.pdf", 1, "id", 1, b"png")
        .is_err());

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&outside);
    let _ = fs::remove_dir_all(&other);
}

#[cfg(windows)]
#[test]
fn checked_open_accepts_an_approved_windows_assets_junction() {
    let dir = scratch("checked-open-approved-assets-junction");
    let outside = std::env::temp_dir().join(format!(
        "tine-approved-assets-junction-outside-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&outside);
    fs::create_dir_all(&outside).unwrap();
    let _ = fs::remove_dir_all(dir.join("assets"));
    let status = std::process::Command::new("cmd")
        .args([
            "/C",
            "mklink",
            "/J",
            &dir.join("assets").display().to_string(),
            &outside.display().to_string(),
        ])
        .status()
        .unwrap();
    assert!(status.success(), "mklink /J must create the test junction");

    assert!(Graph::open_checked(&dir).is_err());
    let graph = Graph::open_checked_with_assets(&dir, Some(&outside)).unwrap();
    assert_eq!(graph.assets_path(), outside.canonicalize().unwrap());

    let _ = fs::remove_dir(dir.join("assets"));
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&outside);
}

#[cfg(unix)]
#[test]
fn checked_open_rejects_graph_text_directories_aliased_inside_graph() {
    use std::os::unix::fs::symlink;
    let dir = scratch("checked-open-graph-alias");
    symlink(dir.join("assets"), dir.join("publish")).unwrap();
    assert!(Graph::open_checked(&dir).is_err());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn journal_filename_format_cannot_escape_graph_on_save() {
    let dir = scratch("journal-format-escape");
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq/config.edn"),
        "{:journal/file-name-format \"../../yyyy_MM_dd\"}\n",
    )
    .unwrap();
    let g = Graph::open_checked(&dir).unwrap();
    let page = PageDto {
        activation: None,
        name: "Jul 10th, 2026".into(),
        kind: PageKind::Journal,
        title: "Jul 10th, 2026".into(),
        pre_block: None,
        blocks: vec![],
        format: Format::Md,
        rev: None,
        read_only: false,
        path: String::new(),
        guide: false,
    };
    assert!(g.save_page(&page, None).is_err());
    assert!(!dir.parent().unwrap().join("2026_07_10.md").exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn load_by_path_serves_the_stray_not_the_canonical() {
    let dir = dup_day_graph("loadbypath");
    let g = Graph::open(&dir);
    g.warm_cache(); // canonical is what name-resolution caches

    // By name → canonical.
    let by_name = g
        .load_named("Friday, 26-06-2026", PageKind::Journal)
        .unwrap()
        .unwrap();
    assert_eq!(by_name.blocks[0].raw, "canonical body");
    assert_eq!(by_name.path, "journals/2026_06_26.org");

    // By path → the STRAY's own content, even though it shares the (kind,name).
    let stray = g
        .load_by_path("journals/Friday, 26-06-2026.org")
        .unwrap()
        .unwrap();
    assert_eq!(stray.blocks[0].raw, "stray body");
    assert_eq!(stray.path, "journals/Friday, 26-06-2026.org");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn save_with_path_writes_the_pinned_file_and_leaves_canonical_intact() {
    // The core regression for #21: editing a path-pinned stray must save to the
    // stray file, NOT be re-resolved by name onto the canonical one.
    let dir = dup_day_graph("savepinned");
    let g = Graph::open(&dir);
    g.warm_cache();

    let mut stray = g
        .load_by_path("journals/Friday, 26-06-2026.org")
        .unwrap()
        .unwrap();
    stray.blocks[0].raw = "stray body edited".into();
    let rev = g.save_page(&stray, stray.rev.as_deref()).unwrap();
    assert_eq!(
        rev,
        content_rev(
            &fs::read_to_string(dir.join("journals").join("Friday, 26-06-2026.org")).unwrap()
        )
    );

    // The stray file got the edit; the canonical file is byte-for-byte untouched.
    assert_eq!(
        fs::read_to_string(dir.join("journals").join("Friday, 26-06-2026.org")).unwrap(),
        "* stray body edited\n"
    );
    assert_eq!(
        fs::read_to_string(dir.join("journals").join("2026_06_26.org")).unwrap(),
        "* canonical body\n"
    );
    // And name-resolution still serves the canonical (the stray didn't poison
    // the (kind,name) cache slot).
    let by_name = g
        .load_named("Friday, 26-06-2026", PageKind::Journal)
        .unwrap()
        .unwrap();
    assert_eq!(by_name.blocks[0].raw, "canonical body");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn save_rejects_pinned_path_that_escapes_the_graph() {
    let dir = dup_day_graph("savebadpath");
    let g = Graph::open(&dir);
    let mut p = g
        .load_by_path("journals/Friday, 26-06-2026.org")
        .unwrap()
        .unwrap();
    p.path = "../escape.md".into();
    assert!(
        g.save_page(&p, p.rev.as_deref()).is_err(),
        "save must refuse an out-of-graph path"
    );
    let _ = fs::remove_dir_all(&dir);
}

// ---- #21: recursive sub-directory scanning under pages/ ----

#[test]
fn nested_page_is_listed_openable_by_name_and_searchable() {
    // A page archived in a real sub-folder (`pages/client-a/foo.md`) must show
    // up as a page — by its BASENAME `foo` (the directory is discarded, OG
    // parity) — and be openable by name and findable by search.
    let dir = scratch("nested-visible");
    fs::create_dir_all(dir.join("pages").join("client-a")).unwrap();
    fs::write(
        dir.join("pages").join("client-a").join("foo.md"),
        "- nestedsentinel body\n",
    )
    .unwrap();
    let g = ready_graph(&dir);
    g.warm_cache();

    // Listed by basename, carrying its nested path.
    let entry = g
        .list_pages()
        .into_iter()
        .find(|e| e.kind == PageKind::Page && e.name == "foo")
        .expect("nested page listed by basename");
    assert_eq!(g.rel_path(&entry.path), "pages/client-a/foo.md");
    assert_eq!(entry.rel_path, "pages/client-a/foo.md");

    // Openable by name (find_entry resolves via the recursive scan), and the
    // DTO carries the nested path so a later save round-trips in place.
    let dto = g
        .load_named("foo", PageKind::Page)
        .unwrap()
        .expect("open nested page by name");
    assert_eq!(dto.blocks[0].raw, "nestedsentinel body");
    assert_eq!(dto.path, "pages/client-a/foo.md");

    // Indexed for full-text search (the cache folded it in via list_pages).
    assert!(
        !g.search("nestedsentinel", 10).unwrap().is_empty(),
        "nested page is searchable"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn nested_page_edit_saves_in_place_with_no_flat_twin() {
    // The data-safety invariant: editing a nested page must write back to its
    // own file — never re-resolve by name and create a flat `pages/foo.md` twin.
    let dir = scratch("nested-roundtrip");
    fs::create_dir_all(dir.join("pages").join("client-a")).unwrap();
    fs::write(
        dir.join("pages").join("client-a").join("foo.md"),
        "- before\n",
    )
    .unwrap();
    let g = Graph::open(&dir);
    g.warm_cache();

    let mut dto = g.load_named("foo", PageKind::Page).unwrap().unwrap();
    assert_eq!(dto.path, "pages/client-a/foo.md");
    dto.blocks[0].raw = "after".into();
    g.save_page(&dto, dto.rev.as_deref()).unwrap();

    // The nested file got the edit…
    assert_eq!(
        fs::read_to_string(dir.join("pages").join("client-a").join("foo.md")).unwrap(),
        "- after\n"
    );
    // …and NO flat twin was created.
    assert!(
        !dir.join("pages").join("foo.md").exists(),
        "save must not create a flat pages/foo.md twin"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn colliding_nested_pages_round_trip_by_path_without_flat_twin() {
    let dir = scratch("nested-collision-roundtrip");
    fs::create_dir_all(dir.join("pages").join("client-a")).unwrap();
    fs::create_dir_all(dir.join("pages").join("client-b")).unwrap();
    fs::write(
        dir.join("pages").join("client-a").join("foo.md"),
        "- before a\n",
    )
    .unwrap();
    fs::write(
        dir.join("pages").join("client-b").join("foo.md"),
        "- before b\n",
    )
    .unwrap();
    let g = Graph::open(&dir);
    g.warm_cache();

    let mut a = g.load_by_path("pages/client-a/foo.md").unwrap().unwrap();
    let mut b = g.load_by_path("pages/client-b/foo.md").unwrap().unwrap();
    assert_eq!(a.name, "foo");
    assert_eq!(b.name, "foo");
    assert_eq!(a.path, "pages/client-a/foo.md");
    assert_eq!(b.path, "pages/client-b/foo.md");

    a.blocks[0].raw = "after a".into();
    b.blocks[0].raw = "after b".into();
    g.save_page(&a, a.rev.as_deref()).unwrap();
    g.save_page(&b, b.rev.as_deref()).unwrap();

    assert_eq!(
        fs::read_to_string(dir.join("pages").join("client-a").join("foo.md")).unwrap(),
        "- after a\n"
    );
    assert_eq!(
        fs::read_to_string(dir.join("pages").join("client-b").join("foo.md")).unwrap(),
        "- after b\n"
    );
    assert!(
        !dir.join("pages").join("foo.md").exists(),
        "path-pinned saves must not create a flat pages/foo.md twin"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn warmed_duplicate_name_cache_keeps_physical_owners_distinct() {
    let dir = scratch("warmed-duplicate-name-owners");
    fs::create_dir_all(dir.join("pages/duplicates")).unwrap();
    let flat = dir.join("pages/Exact Storage Twin.md");
    let nested = dir.join("pages/duplicates/Exact Storage Twin.md");
    fs::write(&flat, "- flat original sentinel\n").unwrap();
    fs::write(&nested, "- nested original sentinel\n").unwrap();

    let g = ready_graph(&dir);
    g.warm_cache();
    let logical_winner = g
        .find_entry("Exact Storage Twin", PageKind::Page)
        .expect("one duplicate is the stable name winner");
    let non_winner_path = if logical_winner.path == flat {
        &nested
    } else {
        &flat
    };
    let non_winner_entry = g
        .entry_for_path(non_winner_path)
        .expect("non-winning duplicate is addressable by path");
    let winner_original = fs::read_to_string(&logical_winner.path).unwrap();

    // Save through the duplicate's captured physical path after both entries
    // have been warmed. The name winner must remain stable while the other
    // physical owner receives its own cached document and revision.
    let mut non_winner = g
        .load_by_path(&non_winner_entry.rel_path)
        .unwrap()
        .expect("non-winning duplicate loads by path");
    non_winner.blocks[0].raw = "nested saved sentinel".into();
    g.save_page(&non_winner, non_winner.rev.as_deref()).unwrap();

    assert_eq!(
        g.find_entry("Exact Storage Twin", PageKind::Page)
            .expect("name winner remains present")
            .path,
        logical_winner.path,
        "path-addressed save must not repoint the logical first winner"
    );

    let winner_loaded = g.load_page(&logical_winner).unwrap();
    assert_eq!(
        winner_loaded.blocks[0].raw,
        winner_original.trim_start_matches("- ").trim_end(),
        "the name winner retains its own warmed bytes"
    );
    let non_winner_loaded = g.load_page(&non_winner_entry).unwrap();
    assert_eq!(non_winner_loaded.blocks[0].raw, "nested saved sentinel");
    assert_eq!(non_winner_loaded.path, non_winner_entry.rel_path);

    let cached = g.with_pages(|pages| {
        pages
            .iter()
            .filter(|(entry, _)| entry.name == "Exact Storage Twin")
            .map(|(entry, doc)| (entry.path.clone(), doc.roots[0].raw.clone()))
            .collect::<Vec<_>>()
    });
    assert!(cached.iter().any(|(path, raw)| {
        *path == logical_winner.path && raw == winner_original.trim_start_matches("- ").trim_end()
    }));
    assert!(cached
        .iter()
        .any(|(path, raw)| *path == *non_winner_path && raw == "nested saved sentinel"));

    for (needle, path) in [
        (
            winner_original.trim_start_matches("- ").trim_end(),
            logical_winner.rel_path.as_str(),
        ),
        ("nested saved sentinel", non_winner_entry.rel_path.as_str()),
    ] {
        assert!(
            g.run_graph_search(needle, 0, 8, false)
                .unwrap()
                .hits
                .iter()
                .any(|hit| matches!(
                    hit,
                    crate::query_plan::QueryHit::Block { path: hit_path, .. } if hit_path == path
                )),
            "search hit for {needle:?} must retain its physical owner {path:?}"
        );
    }

    // Give the winner the non-winner's current bytes. A name-keyed revision
    // map incorrectly treats that as already fresh and suppresses its reload.
    fs::write(&logical_winner.path, "- nested saved sentinel\n").unwrap();
    assert!(
        g.sync_file(&logical_winner.path)
            .is_some_and(|entry| entry.path == logical_winner.path),
        "one duplicate's revision must not mark the other duplicate fresh"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn forget_file_evicts_only_the_deleted_duplicate_path() {
    let dir = scratch("forget-duplicate-path");
    fs::create_dir_all(dir.join("pages/duplicates")).unwrap();
    let flat = dir.join("pages/Exact Storage Twin.md");
    let nested = dir.join("pages/duplicates/Exact Storage Twin.md");
    fs::write(&flat, "- flat survives if not removed\n").unwrap();
    fs::write(&nested, "- nested survives if not removed\n").unwrap();

    let g = Graph::open(&dir);
    g.warm_cache();
    let removed = g
        .find_entry("Exact Storage Twin", PageKind::Page)
        .expect("one duplicate is the initial logical winner");
    let survivor_path = if removed.path == flat { &nested } else { &flat };
    let survivor = g
        .entry_for_path(survivor_path)
        .expect("the other duplicate is a physical cache owner");

    fs::remove_file(&removed.path).unwrap();
    assert_eq!(
        g.forget_file(&removed.path)
            .expect("the deleted path had a cache entry")
            .path,
        removed.path
    );
    assert_eq!(
        g.find_entry("Exact Storage Twin", PageKind::Page)
            .expect("surviving duplicate is the new name winner")
            .path,
        survivor.path
    );
    assert_eq!(
        g.with_pages(|pages| {
            pages
                .iter()
                .filter(|(entry, _)| entry.name == "Exact Storage Twin")
                .map(|(entry, _)| entry.path.clone())
                .collect::<Vec<_>>()
        }),
        vec![survivor.path.clone()],
        "forgetting one physical duplicate leaves the other cached"
    );
    assert_eq!(
        g.load_page(&survivor).unwrap().blocks[0].raw,
        fs::read_to_string(&survivor.path)
            .unwrap()
            .trim_start_matches("- ")
            .trim_end()
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn rename_refuses_colliding_nested_page_identities_without_losing_either() {
    let dir = scratch("nested-collision-rename");
    fs::create_dir_all(dir.join("pages/client-a")).unwrap();
    fs::create_dir_all(dir.join("pages/client-b")).unwrap();
    let a = dir.join("pages/client-a/foo.md");
    let b = dir.join("pages/client-b/foo.md");
    fs::write(&a, "- body a\n").unwrap();
    fs::write(&b, "- body b\n").unwrap();
    let g = Graph::open(&dir);

    let err = g.rename_page("foo", "bar").unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read_to_string(&a).unwrap(), "- body a\n");
    assert_eq!(fs::read_to_string(&b).unwrap(), "- body b\n");
    assert!(!dir.join("pages/bar.md").exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn delete_refuses_ambiguous_nested_page_identity() {
    let dir = scratch("nested-collision-delete");
    fs::create_dir_all(dir.join("pages/client-a")).unwrap();
    fs::create_dir_all(dir.join("pages/client-b")).unwrap();
    let a = dir.join("pages/client-a/foo.md");
    let b = dir.join("pages/client-b/foo.md");
    fs::write(&a, "- body a\n").unwrap();
    fs::write(&b, "- body b\n").unwrap();
    let g = Graph::open(&dir);

    let err = g.delete_page("foo", PageKind::Page).unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read_to_string(&a).unwrap(), "- body a\n");
    assert_eq!(fs::read_to_string(&b).unwrap(), "- body b\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn page_mutations_require_the_captured_exact_owner_and_still_refuse_duplicates() {
    let dir = scratch("expected-page-owner");
    fs::create_dir_all(dir.join("pages/client-a")).unwrap();
    fs::create_dir_all(dir.join("pages/client-b")).unwrap();
    let a = dir.join("pages/client-a/Twin.md");
    let b = dir.join("pages/client-b/Twin.md");
    fs::write(&a, "- client a\n").unwrap();
    let g = Graph::open(&dir);

    let stale = g
        .delete_page_expected("Twin", PageKind::Page, Some("pages/client-b/Twin.md"))
        .unwrap_err();
    assert_eq!(stale.kind(), io::ErrorKind::NotFound);
    assert_eq!(fs::read_to_string(&a).unwrap(), "- client a\n");

    fs::write(&b, "- client b\n").unwrap();
    let g = Graph::open(&dir);
    let ambiguous = g
        .rename_page_expected("Twin", "Renamed", Some("pages/client-b/Twin.md"))
        .unwrap_err();
    assert_eq!(ambiguous.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read_to_string(&a).unwrap(), "- client a\n");
    assert_eq!(fs::read_to_string(&b).unwrap(), "- client b\n");
    assert!(!dir.join("pages/Renamed.md").exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn rename_refuses_target_that_exists_in_other_format() {
    let dir = scratch("rename-cross-format-target");
    let old = dir.join("pages/Old.org");
    let target = dir.join("pages/New.md");
    fs::write(&old, "* old body\n").unwrap();
    fs::write(&target, "- existing target\n").unwrap();
    let g = Graph::open(&dir);

    let err = g.rename_page("Old", "New").unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read_to_string(&old).unwrap(), "* old body\n");
    assert_eq!(fs::read_to_string(&target).unwrap(), "- existing target\n");
    assert!(!dir.join("pages/New.org").exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn graph_text_twin_refusal_includes_markdown_extension_variant() {
    let dir = scratch("graph-markdown-twin");
    fs::write(dir.join("pages/Twin.md"), "- md body\n").unwrap();
    fs::write(dir.join("pages/Twin.markdown"), "- markdown body\n").unwrap();
    let graph = Graph::open(&dir);
    let write = graph.admit_graph_text_writer().unwrap();

    assert!(graph
        .graph_text_has_twin(&write, "Twin", PageKind::Page)
        .unwrap());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn rename_preserves_configured_markdown_extension() {
    let dir = scratch("rename-preserve-markdown-extension");
    let old = dir.join("pages/Old.markdown");
    fs::write(&old, "- old body\n").unwrap();
    let graph = Graph::open(&dir);

    graph.rename_page("Old", "Renamed").unwrap();

    let renamed = dir.join("pages/Renamed.markdown");
    assert_eq!(fs::read_to_string(&renamed).unwrap(), "- old body\n");
    assert!(!old.exists());
    assert!(!dir.join("pages/Renamed.md").exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn rename_refuses_logical_target_in_nested_directory() {
    let dir = scratch("rename-nested-target");
    fs::create_dir_all(dir.join("pages/client")).unwrap();
    let old = dir.join("pages/Old.org");
    let target = dir.join("pages/client/New.md");
    fs::write(&old, "* old body\n").unwrap();
    fs::write(&target, "- nested target\n").unwrap();
    let g = Graph::open(&dir);

    let err = g.rename_page("Old", "New").unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read_to_string(&old).unwrap(), "* old body\n");
    assert_eq!(fs::read_to_string(&target).unwrap(), "- nested target\n");
    assert!(!dir.join("pages/New.org").exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn merge_pages_appends_stray_into_canonical_and_trashes_stray() {
    let dir = dup_day_graph("merge");
    let g = Graph::open(&dir);
    g.warm_cache();
    g.merge_pages("journals/Friday, 26-06-2026.org", "journals/2026_06_26.org")
        .unwrap();

    // Canonical now holds both bodies; the stray is gone (moved to trash).
    let merged = fs::read_to_string(dir.join("journals").join("2026_06_26.org")).unwrap();
    assert!(
        merged.contains("canonical body"),
        "canonical kept: {merged:?}"
    );
    assert!(merged.contains("stray body"), "stray appended: {merged:?}");
    assert!(
        !dir.join("journals").join("Friday, 26-06-2026.org").exists(),
        "stray trashed"
    );
    // Recoverable, not hard-deleted.
    let trash = dir.join("logseq").join(".tine-trash");
    let kept = fs::read_dir(&trash).unwrap().flatten().count();
    assert_eq!(kept, 1, "stray sits in the recoverable trash");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn rename_collision_merges_content_and_rewrites_graph_refs() {
    let dir = scratch("rename-collision-merge");
    fs::write(
        dir.join("pages/Old.md"),
        "- old body links [[Old]] and [[Old/Child]]\n",
    )
    .unwrap();
    fs::write(dir.join("pages/New.md"), "- new body links [[Old]]\n").unwrap();
    // The default page filename format is Legacy, so a namespace slash is
    // percent-encoded. Using the TripleLowbar spelling here made the file a
    // literal `Old___Child` page and left the test's descendant assertion
    // outside the behavior it claimed to exercise.
    fs::write(dir.join("pages/Old%2FChild.md"), "- child of [[Old]]\n").unwrap();
    fs::write(dir.join("pages/Referrer.md"), "- see [[Old]] and #Old\n").unwrap();
    let graph = Graph::open(&dir);

    graph
        .merge_pages_after_rename("pages/Old.md", "pages/New.md", "Old", "New")
        .unwrap();

    let merged = fs::read_to_string(dir.join("pages/New.md")).unwrap();
    assert!(merged.contains("new body links [[New]]"), "{merged}");
    assert!(
        merged.contains("old body links [[New]] and [[New/Child]]"),
        "{merged}"
    );
    assert_eq!(
        fs::read_to_string(dir.join("pages/Referrer.md")).unwrap(),
        "- see [[New]] and #New\n"
    );
    assert_eq!(
        fs::read_to_string(dir.join("pages/New%2FChild.md")).unwrap(),
        "- child of [[New]]\n"
    );
    assert!(!dir.join("pages/Old.md").exists());
    assert!(!dir.join("pages/Old%2FChild.md").exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn rename_file_to_page_rescues_stray_and_refuses_collision() {
    let dir = dup_day_graph("renamefile");
    let g = Graph::open(&dir);
    g.warm_cache();
    g.rename_file_to_page("journals/Friday, 26-06-2026.org", "Old Friday")
        .unwrap();

    // The stray became a normal page, reachable by its new unique name.
    assert!(!dir.join("journals").join("Friday, 26-06-2026.org").exists());
    GRAPH_TEXT_CONTENT_READS.with(|reads| reads.set(0));
    GRAPH_TEXT_PARSE_ATTEMPTS.with(|attempts| attempts.set(0));
    let inventory = g.list_pages();
    assert!(inventory
        .iter()
        .any(|entry| { entry.name == "Old Friday" && entry.rel_path == "pages/Old Friday.org" }));
    assert_eq!(GRAPH_TEXT_CONTENT_READS.with(Cell::get), 0);
    assert_eq!(GRAPH_TEXT_PARSE_ATTEMPTS.with(Cell::get), 0);
    let page = g.load_named("Old Friday", PageKind::Page).unwrap().unwrap();
    assert_eq!(page.blocks[0].raw, "stray body");
    assert_eq!(page.kind, PageKind::Page);

    // A second rescue onto an existing page name is refused (never clobbers).
    fs::write(
        dir.join("journals").join("Saturday, 27-06-2026.org"),
        "* s\n",
    )
    .unwrap();
    assert!(
        g.rename_file_to_page("journals/Saturday, 27-06-2026.org", "Old Friday")
            .is_err(),
        "collision refused"
    );
    assert!(
        dir.join("journals")
            .join("Saturday, 27-06-2026.org")
            .exists(),
        "source left intact on refusal"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn rename_file_to_page_refuses_legacy_page_identity_collision() {
    let dir = scratch("renamefile-legacy-identity-collision");
    let incumbent = dir.join("pages/A:B.md");
    let stray = dir.join("journals/Loose.md");
    let incumbent_bytes = b"- authoritative historical page\n";
    let stray_bytes = b"- loose journal stray\n";
    fs::write(&incumbent, incumbent_bytes).unwrap();
    fs::write(&stray, stray_bytes).unwrap();
    let graph = Graph::open(&dir);

    let error = graph
        .rename_file_to_page("journals/Loose.md", "A:B")
        .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(&incumbent).unwrap(), incumbent_bytes);
    assert_eq!(fs::read(&stray).unwrap(), stray_bytes);
    assert!(!dir.join("pages/A%3AB.md").exists());
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn rename_file_to_page_rejects_symlinked_pages_before_legacy_collision_scan() {
    use std::os::unix::fs::symlink;

    let dir = scratch("renamefile-retained-pages-symlink");
    let outside = scratch("renamefile-retained-pages-symlink-outside");
    let incumbent = outside.join("pages/A:B.md");
    let stray = dir.join("journals/Loose.md");
    let incumbent_bytes = b"- external legacy page\n";
    let stray_bytes = b"- admitted loose stray\n";
    fs::write(&incumbent, incumbent_bytes).unwrap();
    fs::write(&stray, stray_bytes).unwrap();
    fs::remove_dir_all(dir.join("pages")).unwrap();
    symlink(outside.join("pages"), dir.join("pages")).unwrap();
    let graph = Graph::open(&dir);

    let error = graph
        .rename_file_to_page("journals/Loose.md", "A:B")
        .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(fs::read(&incumbent).unwrap(), incumbent_bytes);
    assert_eq!(fs::read(&stray).unwrap(), stray_bytes);
    assert!(!outside.join("pages/A%3AB.md").exists());
    fs::remove_file(dir.join("pages")).unwrap();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&outside);
}

#[test]
fn rename_file_to_page_refuses_legacy_identity_in_other_text_extension() {
    let dir = scratch("renamefile-legacy-identity-extension");
    let incumbent = dir.join("pages/B:C.markdown");
    let stray = dir.join("journals/Loose.org");
    let incumbent_bytes = b"- authoritative legacy markdown page\n";
    let stray_bytes = b"* loose org journal stray\n";
    fs::write(&incumbent, incumbent_bytes).unwrap();
    fs::write(&stray, stray_bytes).unwrap();
    let graph = Graph::open(&dir);

    let error = graph
        .rename_file_to_page("journals/Loose.org", "B:C")
        .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(&incumbent).unwrap(), incumbent_bytes);
    assert_eq!(fs::read(&stray).unwrap(), stray_bytes);
    assert!(!dir.join("pages/B%3AC.org").exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn rename_file_to_page_refuses_effective_markdown_title_collision() {
    let dir = scratch("renamefile-effective-markdown-title-collision");
    let incumbent = dir.join("pages/Other.md");
    let stray = dir.join("journals/Loose.md");
    let incumbent_bytes = b"title:: A:B\n\n- authoritative page\n";
    let stray_bytes = b"- loose journal stray\n";
    fs::write(&incumbent, incumbent_bytes).unwrap();
    fs::write(&stray, stray_bytes).unwrap();
    let graph = Graph::open(&dir);

    let error = graph
        .rename_file_to_page("journals/Loose.md", "A:B")
        .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(&incumbent).unwrap(), incumbent_bytes);
    assert_eq!(fs::read(&stray).unwrap(), stray_bytes);
    assert!(!dir.join("pages/A%3AB.md").exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn journal_conflicts_expose_a_routable_path_per_file() {
    let dir = dup_day_graph("conflictpath");
    let g = Graph::open(&dir);
    let conflicts = g.journal_conflicts();
    assert_eq!(conflicts.len(), 1, "one duplicated day");
    let files = &conflicts[0].files;
    assert_eq!(files.len(), 2);
    // Canonical first; both carry a graph-root-relative, resolvable path.
    assert!(files[0].canonical);
    assert_eq!(files[0].path, "journals/2026_06_26.org");
    assert_eq!(files[1].path, "journals/Friday, 26-06-2026.org");
    for f in files {
        assert!(
            g.resolve_rel(&f.path).is_some(),
            "conflict path resolves: {}",
            f.path
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn root_replacement_after_admission_writes_retained_resource() {
    let dir = scratch("admission-root-race");
    let moved = dir.with_file_name("tine-admission-root-race-moved");
    let _ = fs::remove_dir_all(&moved);
    let graph = Graph::open(&dir);
    GRAPH_TEXT_WRITE_AFTER_ADMISSION.with(|hook| {
        let dir = dir.clone();
        let moved = moved.clone();
        *hook.borrow_mut() = Some(Box::new(move || {
            fs::rename(&dir, &moved)?;
            fs::create_dir_all(dir.join("pages"))?;
            fs::create_dir_all(dir.join("journals"))
        }));
    });

    assert!(graph
        .create_markdown_page_if_absent("admission retained", "- retained\n")
        .unwrap());
    assert!(!dir.join("pages").join("admission retained.md").exists());
    assert_eq!(
        fs::read(moved.join("pages").join("admission retained.md")).unwrap(),
        b"- retained\n"
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&moved);
}

#[cfg(unix)]
#[test]
fn root_replacement_while_writer_waits_for_page_lock_writes_retained_resource() {
    let dir = scratch("root-replacement-page-lock");
    let moved = dir.with_file_name("tine-root-replacement-page-lock-moved");
    let _ = fs::remove_dir_all(&moved);
    let graph = Arc::new(Graph::open(&dir));
    let target = dir.join("pages").join("page lock retained.md");
    let lock = graph.page_lock(&target);
    let guard = lock.lock().unwrap();
    let (admitted_tx, admitted_rx) = std::sync::mpsc::channel();
    let writer = std::thread::spawn({
        let graph = Arc::clone(&graph);
        move || {
            GRAPH_TEXT_WRITE_AFTER_IDENTITY_CHECK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move || admitted_tx.send(()).unwrap()));
            });
            graph.create_markdown_page_if_absent("page lock retained", "- retained\n")
        }
    });

    admitted_rx.recv().unwrap();
    fs::rename(&dir, &moved).unwrap();
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("journals")).unwrap();
    drop(guard);

    assert!(writer.join().unwrap().unwrap());
    assert!(!target.exists());
    assert_eq!(
        fs::read(moved.join("pages").join("page lock retained.md")).unwrap(),
        b"- retained\n"
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&moved);
}

#[test]
fn retained_content_budget_failed_reservation_is_atomic_and_retryable() {
    let budget = RetainedContentBudget::new(GraphTextInventoryLimits {
        retained_content_bytes: 10,
        ..GRAPH_TEXT_INVENTORY_LIMITS
    });
    let first = budget.reserve(6, "first").unwrap();
    assert_eq!(budget.retained(), 6);
    assert_eq!(
        budget.reserve(5, "rejected").unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(
        budget.retained(),
        6,
        "failed admission poisoned the counter"
    );
    drop(first);
    assert_eq!(budget.retained(), 0);
    let retry = budget.reserve(10, "exact retry").unwrap();
    assert_eq!(budget.retained(), 10);
    drop(retry);
    assert_eq!(budget.retained(), 0);
}

#[test]
fn budgeted_reader_retains_metadata_capacity_across_repeated_shrink_races() {
    let root = scratch("budgeted-reader-shrink-capacity");
    let path = root.join("pages/shrinking.md");
    let graph = Graph::open(&root);
    let budget = RetainedContentBudget::new(GraphTextInventoryLimits {
        retained_content_bytes: 64,
        ..GRAPH_TEXT_INVENTORY_LIMITS
    });
    for _ in 0..3 {
        fs::write(&path, vec![b'x'; 64]).unwrap();
        BOUNDED_READ_AFTER_METADATA.with(|hook| {
            let path = path.clone();
            *hook.borrow_mut() = Some(Box::new(move || {
                let file = fs::OpenOptions::new().write(true).open(path)?;
                file.set_len(1)
            }));
        });
        let (_, bytes, reservation) = open_and_read_projection_regular_with_budget(
            graph.projection_root.as_ref().unwrap(),
            "pages/shrinking.md",
            64,
            &budget,
            "shrink race",
        )
        .unwrap();
        assert_eq!(bytes.len(), 1);
        assert_eq!(bytes.capacity(), 64);
        assert_eq!(budget.retained(), 64);
        drop(bytes);
        drop(reservation);
        assert_eq!(budget.retained(), 0);
    }
    let _ = fs::remove_dir_all(&root);
}

#[cfg(any(unix, windows))]
#[test]
fn namespace_rename_budget_has_exact_pass_fail_and_retry_boundary() {
    fn populate(dir: &Path) {
        fs::write(dir.join("pages/Project.md"), "- [[Project/Child]]\n").unwrap();
        fs::write(
            dir.join("pages/Project%2FChild.md"),
            "- child\n  tags:: Project\n",
        )
        .unwrap();
        fs::write(
            dir.join("pages/Refs.md"),
            "- [[Project]] and #Project/Child\n",
        )
        .unwrap();
    }

    let probe = scratch("budget-rename-a");
    populate(&probe);
    Graph::open(&probe)
        .rename_page("Project", "Archive")
        .unwrap();
    let peak = last_graph_text_content_budget_peak();

    let accepted = scratch("budget-rename-b");
    populate(&accepted);
    set_graph_text_content_budget_limit(peak);
    Graph::open(&accepted)
        .rename_page("Project", "Archive")
        .unwrap();
    clear_graph_text_content_budget_limit();
    assert!(accepted.join("pages/Archive.md").exists());
    assert!(fs::read_to_string(accepted.join("pages/Refs.md"))
        .unwrap()
        .contains("[[Archive]]"));

    let rejected = scratch("budget-rename-c");
    populate(&rejected);
    set_graph_text_content_budget_limit(peak - 1);
    let graph = Graph::open(&rejected);
    let error = graph.rename_page("Project", "Archive").unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        fs::read_to_string(rejected.join("pages/Project.md")).unwrap(),
        "- [[Project/Child]]\n"
    );
    assert!(!rejected.join("pages/Archive.md").exists());
    // The rejected rename must leave nothing of itself behind. The parsed
    // cache may legitimately be warm here — `list_pages` joins the shared
    // page-build flight (GH #550) — but every page in it must still be the
    // page that is on disk, which is the graph before the rename.
    if let Some(pages) = graph.cache.read().unwrap().as_ref() {
        assert!(pages.iter().any(|(entry, _)| entry.name == "Project"));
        assert!(!pages.iter().any(|(entry, _)| entry.name == "Archive"));
    }
    assert!(graph.recent_writes.lock().unwrap().is_empty());

    set_graph_text_content_budget_limit(peak);
    graph.rename_page("Project", "Archive").unwrap();
    clear_graph_text_content_budget_limit();
    assert!(rejected.join("pages/Archive.md").exists());
    assert!(fs::read_to_string(rejected.join("pages/Refs.md"))
        .unwrap()
        .contains("[[Archive]]"));

    let _ = fs::remove_dir_all(&probe);
    let _ = fs::remove_dir_all(&accepted);
    let _ = fs::remove_dir_all(&rejected);
}

#[cfg(any(unix, windows))]
#[test]
fn namespace_rename_many_small_entries_charges_container_state_before_mutation() {
    fn populate(dir: &Path, count: usize) {
        fs::write(dir.join("pages/Project.md"), "- root\n").unwrap();
        for index in 0..count {
            fs::write(
                dir.join("pages")
                    .join(format!("Project%2FTiny{index:03}.md")),
                "- x\n",
            )
            .unwrap();
        }
    }

    let small = scratch("rename-container-probe-small");
    populate(&small, 1);
    Graph::open(&small)
        .rename_page("Project", "Archive")
        .unwrap();
    let small_peak = last_graph_text_content_budget_peak();

    let many = scratch("rename-container-many-a");
    populate(&many, 32);
    Graph::open(&many)
        .rename_page("Project", "Archive")
        .unwrap();
    let many_peak = last_graph_text_content_budget_peak();
    assert!(many_peak > small_peak);

    let rejected = scratch("rename-container-many-b");
    populate(&rejected, 32);
    let before = regular_file_tree(&rejected.join("pages"));
    let graph = Graph::open(&rejected);
    set_graph_text_content_budget_limit(many_peak - 1);
    assert_eq!(
        graph.rename_page("Project", "Archive").unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(regular_file_tree(&rejected.join("pages")), before);
    assert!(!rejected.join("pages/Archive.md").exists());
    assert!(graph.recent_writes.lock().unwrap().is_empty());
    set_graph_text_content_budget_limit(many_peak);
    graph.rename_page("Project", "Archive").unwrap();
    clear_graph_text_content_budget_limit();
    assert!(rejected.join("pages/Archive.md").exists());
    assert!(rejected.join("pages/Archive%2FTiny031.md").exists());

    let _ = fs::remove_dir_all(&small);
    let _ = fs::remove_dir_all(&many);
    let _ = fs::remove_dir_all(&rejected);
}

#[test]
fn cached_reference_and_dto_depth_boundaries_are_iterative_and_contained() {
    const TARGET_ID: &str = "aaaaaaaa-0000-0000-0000-000000000001";

    fn nested_document(depth: usize, deepest_raw: Option<&str>) -> Document {
        let mut children = Vec::new();
        for level in (0..depth).rev() {
            let raw = if level + 1 == depth {
                deepest_raw
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("- [[Depth {level}]]"))
            } else {
                format!("- [[Depth {level}]]")
            };
            let mut block = DocBlock::new(&raw);
            block.uuid = format!("runtime-depth-{level}");
            block.children = children;
            children = vec![block];
        }
        Document {
            pre_block: None,
            roots: children,
        }
    }

    fn nested_markdown(depth: usize) -> String {
        let mut markdown = String::new();
        for level in 0..depth {
            markdown.push_str(&"\t".repeat(level));
            markdown.push_str(&format!("- level {level}\n"));
        }
        markdown
    }

    let dir = scratch("iterative-cache-dto-depth");
    let entry = PageEntry {
        name: "Source".to_owned(),
        kind: PageKind::Page,
        date_key: None,
        rel_path: "pages/Source.md".to_owned(),
        path: dir.join("pages/Source.md"),
    };
    let target_entry = PageEntry {
        name: "Deep target".to_owned(),
        kind: PageKind::Page,
        date_key: None,
        rel_path: "pages/Deep target.md".to_owned(),
        path: dir.join("pages/Deep target.md"),
    };
    let target_doc = nested_document(1, Some("- target"));
    let deepest_reference = format!("- [[Deep target]] and (({TARGET_ID}))");

    let accepted = nested_document(MAX_BLOCK_DEPTH, Some(&deepest_reference));
    let accepted_dto = page_dto_checked(&entry, &accepted).unwrap();
    let mut accepted_walk = BlockDtoWalk::new(&accepted_dto.blocks);
    let mut accepted_count = 0_usize;
    while accepted_walk.next().unwrap().is_some() {
        accepted_count += 1;
    }
    assert_eq!(accepted_count, MAX_BLOCK_DEPTH);

    let accepted_block = block_to_dto(&accepted.roots[0]).unwrap();
    let mut accepted_block_walk = BlockDtoWalk::new(std::slice::from_ref(&accepted_block));
    let mut accepted_block_count = 0_usize;
    while accepted_block_walk.next().unwrap().is_some() {
        accepted_block_count += 1;
    }
    assert_eq!(accepted_block_count, MAX_BLOCK_DEPTH);

    let accepted_snapshot = Graph::from_page_snapshot(
        &dir,
        vec![
            (entry.clone(), Arc::new(accepted.clone())),
            (target_entry.clone(), Arc::new(target_doc.clone())),
        ],
    );
    let target_names = vec![crate::refs::page_key("Deep target")];
    let accepted_candidates = accepted_snapshot.reference_candidate_pages(
        &target_names,
        "Deep target",
        ReferenceKind::Explicit,
    );
    assert!(!accepted_candidates.indexed);
    assert_eq!(
            candidate_paths(&accepted_candidates),
            vec![
                "pages/Deep target.md".to_owned(),
                "pages/Source.md".to_owned(),
            ],
            "without an attached current SQLite projection, exact parser fallback must retain the complete snapshot",
        );
    let accepted_counts = accepted_snapshot.block_ref_counts().unwrap();
    assert_eq!(accepted_counts.get(TARGET_ID).copied(), Some(1));

    let accepted_graph = Graph::open(&dir);
    *accepted_graph.cache.write().unwrap() =
        Some(Arc::new(vec![(entry.clone(), Arc::new(accepted))]));
    assert_eq!(
        accepted_graph.referenced_page_names().len(),
        MAX_BLOCK_DEPTH,
    );

    let rejected = nested_document(MAX_BLOCK_DEPTH + 1, Some(&deepest_reference));
    assert_eq!(
        page_dto_checked(&entry, &rejected).unwrap_err().kind(),
        io::ErrorKind::InvalidData,
    );
    assert_eq!(
        block_to_dto(&rejected.roots[0]).unwrap_err().kind(),
        io::ErrorKind::InvalidData,
    );
    assert_eq!(
        block_to_dto(&DocBlock::new("- missing runtime identity"))
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData,
    );

    let rejected_snapshot = Graph::from_page_snapshot(
        &dir,
        vec![
            (entry.clone(), Arc::new(rejected.clone())),
            (target_entry, Arc::new(target_doc)),
        ],
    );
    for _ in 0..2 {
        let candidates = rejected_snapshot.reference_candidate_pages(
            &target_names,
            "Deep target",
            ReferenceKind::Explicit,
        );
        assert!(!candidates.indexed);
        assert_eq!(candidates.pages.len(), candidates.full_page_count);
        assert!(
            candidate_paths(&candidates).contains(&"pages/Source.md".to_owned()),
            "the deepest possible referrer must remain in the fallback set",
        );
        assert_eq!(
            rejected_snapshot.block_ref_counts().unwrap_err().kind(),
            io::ErrorKind::InvalidData,
        );
    }

    let rejected_graph = Graph::open(&dir);
    *rejected_graph.cache.write().unwrap() = Some(Arc::new(vec![(entry, Arc::new(rejected))]));
    assert!(rejected_graph.referenced_page_names().is_empty());

    let accepted_markdown =
        markdown_page_dto("Depth 128", "Depth 128", &nested_markdown(128)).unwrap();
    let mut accepted_markdown_walk = BlockDtoWalk::new(&accepted_markdown.blocks);
    let mut accepted_markdown_count = 0_usize;
    while accepted_markdown_walk.next().unwrap().is_some() {
        accepted_markdown_count += 1;
    }
    assert_eq!(accepted_markdown_count, MAX_BLOCK_DEPTH);
    assert_eq!(
        markdown_page_dto("Depth 129", "Depth 129", &nested_markdown(129))
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData,
    );
    let ordinary = markdown_page_dto("Ordinary", "Ordinary", "- body\n").unwrap();
    assert_eq!(ordinary.blocks.len(), 1);
    assert_eq!(ordinary.blocks[0].raw, "body");

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn graph_text_scope_binding_tracks_policy_and_retained_resource_across_move() {
    let dir = scratch("graph-text-scope-binding");
    let moved = dir.with_file_name("tine-graph-text-scope-binding-moved");
    let copied = dir.with_file_name("tine-graph-text-scope-binding-copy");
    let _ = fs::remove_dir_all(&moved);
    let _ = fs::remove_dir_all(&copied);
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq/config.edn"),
        r#"{:hidden ["archive/" "scratch" "archive"]}"#,
    )
    .unwrap();
    let graph = Graph::open(&dir);
    let original = graph.graph_text_scope_binding().unwrap();

    fs::write(dir.join("logseq/config.edn"), r#"{:hidden ["different"]}"#).unwrap();
    let changed_policy = Graph::open(&dir);
    assert_eq!(
        graph.canonical_resource_id().unwrap(),
        changed_policy.canonical_resource_id().unwrap()
    );
    assert_ne!(original, changed_policy.graph_text_scope_binding().unwrap());

    fs::rename(&dir, &moved).unwrap();
    assert_eq!(original, graph.graph_text_scope_binding().unwrap());

    fs::create_dir_all(copied.join("pages")).unwrap();
    fs::create_dir_all(copied.join("journals")).unwrap();
    fs::create_dir_all(copied.join("logseq")).unwrap();
    fs::write(
        copied.join("logseq/config.edn"),
        r#"{:hidden ["archive" "scratch"]}"#,
    )
    .unwrap();
    let copied_graph = Graph::open(&copied);
    assert_ne!(original, copied_graph.graph_text_scope_binding().unwrap());

    let _ = fs::remove_dir_all(&moved);
    let _ = fs::remove_dir_all(&copied);
}

#[cfg(windows)]
#[test]
fn windows_live_graph_root_move_is_denied_without_rebinding() {
    let dir = scratch("windows-live-root-move-denied");
    let moved = dir.with_file_name("tine-windows-live-root-move-denied-moved");
    let _ = fs::remove_dir_all(&moved);
    let graph = Graph::open(&dir);
    let binding = graph.graph_text_scope_binding().unwrap();

    let error = fs::rename(&dir, &moved).unwrap_err();
    assert_eq!(
        error.raw_os_error(),
        Some(windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION as i32)
    );
    assert_eq!(graph.graph_text_scope_binding().unwrap(), binding);
    // The retained binding still writes. The exact-byte writer this used went
    // with Managed Storage (ADR 0066); the user path proves the same thing.
    graph.warm_cache();
    let page = markdown_page_dto("still-bound", "still-bound", "- retained\n").unwrap();
    graph.save_page(&page, None).unwrap();
    assert_eq!(
        fs::read(dir.join("pages/still-bound.md")).unwrap(),
        b"- retained\n"
    );

    drop(graph);
    crate::test_support::remove_dir_all(&dir);
}

#[test]
fn parsed_page_title_accepts_case_insensitive_org_directives_and_drawers() {
    for (source, expected) in [
        ("#+title: Lower\n\n* body\n", "Lower"),
        ("#+TITLE: Upper\n\n* body\n", "Upper"),
        ("#+TiTlE: Mixed\n\n* body\n", "Mixed"),
        (":PROPERTIES:\n:TiTlE: Drawer\n:END:\n\n* body\n", "Drawer"),
    ] {
        let document = crate::org::parse_org(source);
        assert_eq!(
            parsed_page_title(&document, Format::Org).as_deref(),
            Some(expected)
        );
    }
}

// GH #254 increment 2. These tests intentionally drive each accepted
// conflict site through its own deterministic boundary. The site-specific
// codes are part of the safety contract: only a site that captured a usable
// override token may enter the keep-mine/use-disk banner class.
fn gh254_loaded(tag: &str) -> (PathBuf, PathBuf, Graph, PageDto) {
    let root = scratch(&format!("gh254-increment2-{tag}"));
    let path = root.join("pages/Note.md");
    fs::write(&path, "- loaded\n").unwrap();
    let graph = Graph::open(&root);
    graph.warm_cache();
    let mut page = graph.load_by_path("pages/Note.md").unwrap().unwrap();
    // A loaded EDITOR, which since increment 3 means an activation as well as
    // a DTO: reading alone no longer implies one, precisely so that a read for
    // export/preview/hydration cannot inherit an editor's override authority.
    // The frontend does exactly this — activate, then stamp the DTO it saves.
    let handle = graph
        .activate_editor(
            "pages/Note.md",
            ActivationIntent::Replace,
            page.rev.as_deref(),
        )
        .unwrap();
    page.activation = Some(handle.activation.as_u64());
    page.blocks[0].raw = "mine".into();
    (root, path, graph, page)
}

#[test]
fn concord_live_save_conflict_uses_editor_base_and_guarded_resolution() {
    let root = scratch("concord-live-save-conflict");
    let path = root.join("pages/Note.md");
    fs::write(&path, "- one\n- two\n").unwrap();
    let graph = Graph::open(&root);
    graph.warm_cache();
    let mut page = graph.load_by_path("pages/Note.md").unwrap().unwrap();
    let activation = graph
        .activate_editor(
            "pages/Note.md",
            ActivationIntent::Replace,
            page.rev.as_deref(),
        )
        .unwrap();
    page.activation = Some(activation.activation.as_u64());
    page.blocks[0].raw = "mine one".into();
    fs::write(&path, "- one\n- disk two\n").unwrap();

    let refusal = graph.save_page(&page, page.rev.as_deref()).unwrap_err();
    let shown = gh254_shown(&refusal);
    let diff = graph
        .live_save_conflict_diff(&page, page.rev.as_deref(), shown)
        .unwrap();
    assert!(
        diff.three_way,
        "the live editor activation must retain a real base"
    );

    fn accept_suggestions(
        rows: &[crate::sync_diff::DiffRow],
        out: &mut std::collections::HashMap<String, String>,
    ) {
        for row in rows {
            if row.kind != crate::sync_diff::RowKind::Unchanged {
                out.insert(
                    row.id.clone(),
                    row.suggestion.clone().unwrap_or_else(|| "both".to_owned()),
                );
            }
            accept_suggestions(&row.children, out);
        }
    }
    let mut decisions = std::collections::HashMap::new();
    accept_suggestions(&diff.rows, &mut decisions);
    graph
        .resolve_live_save_conflict(&page, page.rev.as_deref(), shown, &decisions, "union")
        .unwrap();
    let resolved = fs::read_to_string(&path).unwrap();
    assert!(
        resolved.contains("mine one"),
        "mine-only edit must survive: {resolved}"
    );
    assert!(
        resolved.contains("disk two"),
        "theirs-only edit must survive: {resolved}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn concord_live_save_conflict_capsule_survives_restart_and_rechecks_disk() {
    let root = scratch("concord-live-save-restart");
    let path = root.join("pages/Note.md");
    fs::write(&path, "- one\n- two\n").unwrap();
    let graph = Graph::open(&root);
    graph.warm_cache();
    let mut page = graph.load_by_path("pages/Note.md").unwrap().unwrap();
    let activation = graph
        .activate_editor(
            "pages/Note.md",
            ActivationIntent::Replace,
            page.rev.as_deref(),
        )
        .unwrap();
    page.activation = Some(activation.activation.as_u64());
    page.blocks[0].raw = "mine one".into();
    fs::write(&path, "- one\n- disk two\n").unwrap();
    let shown = gh254_shown(&graph.save_page(&page, page.rev.as_deref()).unwrap_err());
    let capture = graph
        .capture_live_save_conflict(&page, page.rev.as_deref(), shown)
        .unwrap();
    assert!(capture.diff.three_way);
    drop(graph);

    // A later process has no activation registry or one-shot token. The
    // app-private capsule is sufficient to reconstruct the same review.
    let reopened = Graph::open(&root);
    reopened.warm_cache();
    let diff = reopened
        .durable_live_save_conflict_diff(&page, capture.base_text.as_deref())
        .unwrap();
    assert!(diff.three_way);
    assert_eq!(diff.conflict_rev, capture.disk_rev);

    let decisions = diff
        .rows
        .iter()
        .filter(|row| row.kind != crate::sync_diff::RowKind::Unchanged)
        .map(|row| {
            (
                row.id.clone(),
                row.suggestion.clone().unwrap_or_else(|| "both".to_owned()),
            )
        })
        .collect::<std::collections::HashMap<_, _>>();

    // An unseen external write invalidates the durable authority rather than
    // being overwritten by choices computed for the prior disk revision.
    fs::write(&path, "- one\n- newer disk two\n").unwrap();
    let rejected = reopened
        .resolve_durable_live_save_conflict(&page, &capture.disk_rev, &decisions, "union")
        .unwrap_err();
    assert_eq!(rejected.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(direct_save_failure_code(&rejected), "conflict.base_rev");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "- one\n- newer disk two\n"
    );

    let refreshed = reopened
        .durable_live_save_conflict_diff(&page, capture.base_text.as_deref())
        .unwrap();
    let refreshed_decisions = refreshed
        .rows
        .iter()
        .filter(|row| row.kind != crate::sync_diff::RowKind::Unchanged)
        .map(|row| {
            (
                row.id.clone(),
                row.suggestion.clone().unwrap_or_else(|| "both".to_owned()),
            )
        })
        .collect::<std::collections::HashMap<_, _>>();
    reopened
        .resolve_durable_live_save_conflict(
            &page,
            &refreshed.conflict_rev,
            &refreshed_decisions,
            "union",
        )
        .unwrap();
    let resolved = fs::read_to_string(&path).unwrap();
    assert!(resolved.contains("mine one"));
    assert!(resolved.contains("newer disk two"));
    let _ = fs::remove_dir_all(root);
}

/// Make `dto` an EDITOR's DTO, the way the frontend does.
///
/// Since increment 3 a loaded page and a live editor are different things: a
/// read alone mints no identity, precisely so a read for export, preview or
/// hydration cannot inherit an editor's override authority. A test that
/// force-saves is modelling a user answering a banner, so it has to activate
/// like one.
fn as_editor(graph: &Graph, dto: &mut PageDto) {
    let handle = graph
        .activate_editor(&dto.path, ActivationIntent::Replace, dto.rev.as_deref())
        .expect("a loaded page is path-pinned and inside the graph");
    dto.activation = Some(handle.activation.as_u64());
}

#[test]
fn durable_draft_deleted_file_review_is_read_only_and_apply_restores() {
    for extension in ["md", "org"] {
        let root = scratch("durable-draft-deleted");
        let relative = format!("pages/Note.{extension}");
        let path = root.join(&relative);
        let base = if extension == "md" {
            "- original\n"
        } else {
            "* original\n"
        };
        fs::write(&path, base).unwrap();
        let graph = Graph::open(&root);
        let mut page = graph.load_by_path(&relative).unwrap().unwrap();
        page.blocks[0].raw = "retained draft".into();
        drop(graph);
        fs::remove_file(&path).unwrap();
        let reopened = Graph::open(&root);
        let diff = reopened
            .durable_live_save_conflict_diff(&page, Some(base))
            .unwrap();
        assert!(!path.exists(), "review must not recreate the file");
        assert_eq!(diff.conflict_rev, "absent");
        reopened
            .resolve_durable_live_save_conflict(
                &page,
                &diff.conflict_rev,
                &std::collections::HashMap::new(),
                "mine",
            )
            .unwrap();
        assert!(fs::read_to_string(&path)
            .unwrap()
            .contains("retained draft"));
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn durable_draft_absence_review_refuses_even_an_empty_recreated_file() {
    let root = scratch("durable-draft-recreated");
    let path = root.join("pages/Note.md");
    fs::write(&path, "- original\n").unwrap();
    let graph = Graph::open(&root);
    let mut page = graph.load_by_path("pages/Note.md").unwrap().unwrap();
    page.blocks[0].raw = "retained draft".into();
    fs::remove_file(&path).unwrap();
    let diff = graph.durable_live_save_conflict_diff(&page, None).unwrap();
    fs::write(&path, "").unwrap();
    let error = graph
        .resolve_durable_live_save_conflict(
            &page,
            &diff.conflict_rev,
            &std::collections::HashMap::new(),
            "mine",
        )
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read_to_string(&path).unwrap(), "");
    assert_eq!(page.blocks[0].raw, "retained draft");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn durable_draft_absence_apply_never_clobbers_a_late_creator() {
    let root = scratch("durable-draft-late-create");
    let path = root.join("pages/Note.md");
    fs::write(&path, "- original\n").unwrap();
    let graph = Graph::open(&root);
    let mut page = graph.load_by_path("pages/Note.md").unwrap().unwrap();
    page.blocks[0].raw = "retained draft".into();
    fs::remove_file(&path).unwrap();
    let diff = graph.durable_live_save_conflict_diff(&page, None).unwrap();
    let raced = path.clone();
    GRAPH_TEXT_WRITE_BEFORE_MUTATION.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || fs::write(&raced, "- external winner\n")));
    });
    let result = graph.resolve_durable_live_save_conflict(
        &page,
        &diff.conflict_rev,
        &std::collections::HashMap::new(),
        "mine",
    );
    GRAPH_TEXT_WRITE_BEFORE_MUTATION.with(|hook| drop(hook.borrow_mut().take()));
    assert!(result.is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "- external winner\n");
    assert_eq!(page.blocks[0].raw, "retained draft");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn durable_draft_present_apply_checks_bytes_at_bound_publication() {
    let root = scratch("durable-draft-late-write");
    let path = root.join("pages/Note.md");
    fs::write(&path, "- original\n").unwrap();
    let graph = Graph::open(&root);
    let mut page = graph.load_by_path("pages/Note.md").unwrap().unwrap();
    page.blocks[0].raw = "retained draft".into();
    let diff = graph.durable_live_save_conflict_diff(&page, None).unwrap();
    let raced = path.clone();
    GRAPH_TEXT_WRITE_BEFORE_MUTATION.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || fs::write(&raced, "- external winner\n")));
    });
    let result = graph.resolve_durable_live_save_conflict(
        &page,
        &diff.conflict_rev,
        &std::collections::HashMap::new(),
        "mine",
    );
    GRAPH_TEXT_WRITE_BEFORE_MUTATION.with(|hook| drop(hook.borrow_mut().take()));
    assert!(result.is_err(), "a later write must refuse the stale merge");
    assert_eq!(fs::read_to_string(&path).unwrap(), "- external winner\n");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn durable_draft_present_apply_refuses_a_late_same_bytes_new_identity() {
    let root = scratch("durable-draft-late-identity");
    let path = root.join("pages/Note.md");
    fs::write(&path, "- original\n").unwrap();
    let graph = Graph::open(&root);
    let mut page = graph.load_by_path("pages/Note.md").unwrap().unwrap();
    page.blocks[0].raw = "retained draft".into();
    let diff = graph.durable_live_save_conflict_diff(&page, None).unwrap();
    let raced = path.clone();
    let replacement = root.join("external-replacement.tmp");
    fs::write(&replacement, "- original\n").unwrap();
    GRAPH_TEXT_WRITE_BEFORE_MUTATION.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            fs::remove_file(&raced)?;
            fs::rename(&replacement, &raced)
        }));
    });
    let result = graph.resolve_durable_live_save_conflict(
        &page,
        &diff.conflict_rev,
        &std::collections::HashMap::new(),
        "mine",
    );
    GRAPH_TEXT_WRITE_BEFORE_MUTATION.with(|hook| drop(hook.borrow_mut().take()));
    assert!(
        result.is_err(),
        "same bytes must not substitute a different physical file"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "- original\n");
    assert_eq!(page.blocks[0].raw, "retained draft");
    let _ = fs::remove_dir_all(root);
}

fn gh254_code(error: &io::Error) -> &'static str {
    direct_save_failure_code(error)
}

#[cfg(any(unix, windows))]
fn gh254_replace(path: &Path, replacement: &Path) -> io::Result<()> {
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(replacement, path)
}

#[test]
fn gh254_token_is_required_and_consumed_once_per_force_attempt() {
    let (root, path, graph, page) = gh254_loaded("one-shot");
    assert!(
        graph.force_save_page(&page).is_err(),
        "a load is not authority"
    );
    fs::write(&path, "- external winner\n").unwrap();
    let conflict = graph.save_page(&page, page.rev.as_deref()).unwrap_err();
    assert_eq!(gh254_code(&conflict), "conflict.save_baseline_present");
    let shown = gh254_shown(&conflict);
    graph
        .force_save_page_at_revision(&page, page.rev.as_deref(), shown)
        .unwrap();
    assert!(
        graph
            .force_save_page_at_revision(&page, page.rev.as_deref(), shown)
            .is_err(),
        "successful force must not replay its consumed token"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gh254_force_binds_the_shown_bytes_not_only_the_revision_or_path() {
    let (root, path, graph, page) = gh254_loaded("substituted-baseline");
    fs::write(&path, "- shown winner\n").unwrap();
    let conflict = graph.save_page(&page, page.rev.as_deref()).unwrap_err();
    // Preserve the inode while changing its bytes. Revision-only force used
    // to overwrite this unseen second winner.
    GRAPH_TEXT_WRITE_BEFORE_MUTATION.with(|hook| {
        let path = path.clone();
        *hook.borrow_mut() = Some(Box::new(move || {
            fs::write(path, "- unseen second winner\n")
        }));
    });
    let error = graph
        .force_save_page_at_revision(&page, page.rev.as_deref(), gh254_shown(&conflict))
        .unwrap_err();
    assert_eq!(gh254_code(&error), "conflict.replace_retired_mismatch");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "- unseen second winner\n"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gh254_absent_snapshot_can_create_once_without_a_present_owner() {
    let (root, path, graph, page) = gh254_loaded("absent");
    fs::remove_file(&path).unwrap();
    let conflict = graph.save_page(&page, page.rev.as_deref()).unwrap_err();
    assert_eq!(gh254_code(&conflict), "conflict.save_baseline_absent");
    graph
        .force_save_page_at_revision(&page, page.rev.as_deref(), gh254_shown(&conflict))
        .unwrap();
    assert!(fs::read_to_string(&path).unwrap().contains("mine"));
    assert!(graph.force_save_page(&page).is_err());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gh254_watcher_and_forget_each_revoke_authority() {
    // The disk-move boundaries. These keep their original assertions: the file
    // underneath the editor genuinely moved, so the snapshot the banner
    // described is gone and its authority must go with it.
    for action in ["watcher", "forget"] {
        let (root, path, graph, page) = gh254_loaded(action);
        fs::write(&path, "- external winner\n").unwrap();
        graph.save_page(&page, page.rev.as_deref()).unwrap_err();
        match action {
            "watcher" => {
                graph.sync_file_checked(&path).unwrap();
            }
            "forget" => {
                graph.forget_file(&path);
            }
            _ => unreachable!(),
        }
        assert!(
            graph.force_save_page(&page).is_err(),
            "{action} must revoke the shown-snapshot authority"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "- external winner\n");
        let _ = fs::remove_dir_all(root);
    }
}

/// The other half of the split (GH #254 increment 3).
///
/// A plain read is NOT an activation and must no longer revoke. Re-hydrating
/// an already-open page happens constantly — the sidebar, live references,
/// query hydration — and revoking there disarms a banner the user can still
/// see, leaving them a conflict whose only working button destroys their edit.
///
/// This asserts the read is inert, NOT that the force then succeeds: under
/// increment 3 a force also needs a live editor activation, which this
/// read-only path deliberately never mints. The two conditions are separate
/// and `gh254_override_requires_a_live_editor_activation` covers the other.
#[test]
fn gh254_a_plain_read_does_not_revoke_authority() {
    let (root, path, graph, page) = gh254_loaded("reload");
    fs::write(&path, "- external winner\n").unwrap();
    graph.save_page(&page, page.rev.as_deref()).unwrap_err();
    let before = graph.outstanding_conflict_override(&page).unwrap();
    assert!(
        before.is_some(),
        "the refused save must have minted authority for this test to mean anything"
    );

    graph.load_by_path("pages/Note.md").unwrap().unwrap();

    let after = graph.outstanding_conflict_override(&page).unwrap();
    assert_eq!(
        before, after,
        "a plain read must leave the live observation exactly as it found it"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "- external winner\n");
    let _ = fs::remove_dir_all(root);
}

/// Rule 1 of the increment-3 contract: an override may only be spent by the
/// exact editor activation that was shown the conflict.
///
/// The reproduced defect this closes is a *clone*. Two DTOs agreeing on path,
/// name and `base_rev` were indistinguishable to the old episode equality, so
/// a copy could spend the live editor's one-shot authority and overwrite the
/// external winner the real editor still had a banner for. Identity therefore
/// cannot be derived from content, revision or path — the copy matches on all
/// three — which is why the activation is minted, opaque, and compared exactly.
#[test]
fn gh254_override_requires_a_live_editor_activation() {
    let (root, path, graph, page) = gh254_loaded("activation");
    fs::write(&path, "- external winner\n").unwrap();
    graph.save_page(&page, page.rev.as_deref()).unwrap_err();
    let shown = graph
        .outstanding_conflict_override(&page)
        .unwrap()
        .expect("the refused save mints the authority the banner shows");

    // (a) No activation at all — an editor-less writer, or a pre-increment-3
    // caller. Legal on the ordinary path; never authority to overwrite.
    let mut tokenless = page.clone();
    tokenless.activation = None;
    let refused = graph
        .force_save_page_at_revision(&tokenless, page.rev.as_deref(), shown)
        .unwrap_err();
    assert!(
        refused.to_string().contains("conflict_authority."),
        "refusal must carry a bounded authority code, got: {refused}"
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "- external winner\n",
        "a refused override must not write"
    );

    // (b) The live editor that was shown this conflict may spend it. Without
    // this arm the test would pass on a build that refuses every override.
    let mut mine = page.clone();
    mine.blocks[0].raw = "mine wins".into();
    graph
        .force_save_page_at_revision(&mine, page.rev.as_deref(), shown)
        .expect("the live editor shown the conflict must be able to answer it");
    assert!(fs::read_to_string(&path).unwrap().contains("mine wins"));
    let _ = fs::remove_dir_all(root);
}

/// An absent editor's prospective target can go stale, and first save must
/// notice.
///
/// The resolver prefers the configured format only while no alternate exists,
/// so an external `.org` appearing after activation moves the answer off the
/// `.md` this editor was promised. Landing on the stale pin would create the
/// ambiguous twin that creation admission exists to refuse; both naive routes
/// were reproduced and are worse (keeping `base_rev = None` strands the draft
/// on `AlreadyExists` forever, adopting the existing revision silently
/// overwrites bytes the user never saw). So the drift becomes an ordinary
/// conflict, answerable with the two buttons the user already understands.
#[test]
fn gh254_absent_editor_first_save_re_resolves_its_drifted_target() {
    // (a) Drift onto a target nobody occupies: same editor, new target.
    let root = scratch("gh254-inc3-drift-free");
    let graph = Graph::open(&root);
    graph.warm_cache();
    let handle = graph
        .activate_absent_editor("Prospective", PageKind::Page)
        .unwrap();
    assert!(handle.target.ends_with(".md"), "got {}", handle.target);
    assert!(
        !root.join(&handle.target).exists(),
        "activation must reserve nothing on disk"
    );
    let _ = fs::remove_dir_all(&root);

    // (b) Drift onto a target that EXISTS is a conflict, not a re-target.
    let root = scratch("gh254-inc3-drift-occupied");
    fs::create_dir_all(root.join("pages")).unwrap();
    let graph = Graph::open(&root);
    graph.warm_cache();
    let handle = graph
        .activate_absent_editor("Prospective", PageKind::Page)
        .unwrap();
    let promised = handle.target.clone();

    // The external winner appears at the alternate extension AFTER activation.
    let occupied = root.join("pages/Prospective.org");
    fs::write(&occupied, "- external winner\n").unwrap();
    graph.sync_file_checked(&occupied).unwrap();

    let mut page = PageDto {
        activation: Some(handle.activation.as_u64()),
        name: "Prospective".into(),
        kind: PageKind::Page,
        title: "Prospective".into(),
        pre_block: None,
        blocks: vec![BlockDto {
            // DUP-8: spelled out at its `Default` value so a new `BlockDto`
            // field has to be decided here rather than arriving silently
            // defaulted.
            id: String::new(),
            raw: "my draft".into(),
            collapsed: false,
            children: Vec::new(),
            breadcrumb: Vec::new(),
            page_property: false,
            marker: None,
            priority: None,
            heading_level: None,
            scheduled: None,
            deadline: None,
            tags: Vec::new(),
            properties: Vec::new(),
        }],
        rev: None,
        format: Format::Md,
        read_only: false,
        path: promised,
        guide: false,
    };
    page.blocks[0].raw = "my draft".into();

    let error = graph.save_page(&page, None).unwrap_err();
    assert!(
        gh254_code(&error).starts_with("conflict."),
        "drift onto an existing file must be an ordinary conflict, got: {}",
        gh254_code(&error)
    );
    assert_eq!(
        fs::read_to_string(&occupied).unwrap(),
        "- external winner\n",
        "the external bytes must survive until the user answers"
    );
    assert!(
        !root.join("pages/Prospective.md").exists(),
        "the stale pin must not be created as an ambiguous twin"
    );

    // The part that actually distinguishes the re-resolve from merely refusing
    // the save: the user must be able to ANSWER this conflict. That requires
    // the editor's identity to have moved with its target — if the activation
    // were still registered against the abandoned `.md` pin, the override would
    // be refused as not-live and the draft would be stranded, which is the
    // failure mode the naive routes produced.
    let shown = graph
        .outstanding_conflict_override(&page)
        .unwrap()
        .expect("the drift conflict must mint answerable authority");
    graph
        .force_save_page_at_revision(&page, None, shown)
        .expect("the absent editor must be able to answer the conflict it hit");
    assert!(
        fs::read_to_string(&occupied).unwrap().contains("my draft"),
        "\"Keep mine\" must land on the file the editor actually drifted onto"
    );
    let _ = fs::remove_dir_all(&root);
}

/// "Use disk version" is an authority-answering action, decided by the same
/// source of truth as "Keep mine" — and it must decide without writing.
///
/// The frontend cannot make this decision itself. The raw-watcher path revokes
/// an observation with no page event to react to, so a locally recorded epoch
/// can be dead while every local value still compares equal; a map maintained
/// by eventual notifications cannot prove live membership. Presenting is the
/// only way to learn the truth, and the three outcomes are exactly what the
/// caller must tell apart: proceed, answer a newer banner, or re-observe a
/// dead one.
#[test]
fn gh254_presenting_an_observation_decides_without_writing() {
    for arm in ["authorised", "superseded", "withdrawn"] {
        let (root, path, graph, page) = gh254_loaded(arm);
        fs::write(&path, "- external winner\n").unwrap();
        graph.save_page(&page, page.rev.as_deref()).unwrap_err();
        let shown = graph
            .outstanding_conflict_override(&page)
            .unwrap()
            .expect("the refused save mints the banner's authority");
        let before = fs::read_to_string(&path).unwrap();

        let presented = match arm {
            "authorised" => shown.observation_epoch,
            // A stale callback naming the observation it was shown, while a
            // newer winner has been observed since.
            "superseded" => {
                fs::write(&path, "- newer external winner\n").unwrap();
                graph.save_page(&page, page.rev.as_deref()).unwrap_err();
                shown.observation_epoch
            }
            // The raw-watcher shape: authority revoked with no page event.
            "withdrawn" => {
                graph.revoke_conflict_authority(&path);
                shown.observation_epoch
            }
            _ => unreachable!(),
        };
        let before = if arm == "superseded" {
            fs::read_to_string(&path).unwrap()
        } else {
            before
        };

        let outcome = graph
            .present_conflict_override(
                "pages/Note.md",
                page.rev.as_deref(),
                page.activation.unwrap(),
                presented,
            )
            .unwrap();

        let expected = match arm {
            "authorised" => ConflictPresentation::Authorised,
            "superseded" => ConflictPresentation::Superseded,
            _ => ConflictPresentation::Withdrawn,
        };
        assert_eq!(outcome, expected, "arm {arm}");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            before,
            "presenting must never write, in any arm ({arm})"
        );
        let _ = fs::remove_dir_all(root);
    }
}

/// The stale-callback shape: a well-formed token naming a real editor that has
/// since been retired. It must be refused even though every other field —
/// path, name, revision, observation epoch — still matches.
#[test]
fn gh254_a_retired_activation_cannot_answer_its_old_conflict() {
    let (root, path, graph, page) = gh254_loaded("retired");
    fs::write(&path, "- external winner\n").unwrap();
    graph.save_page(&page, page.rev.as_deref()).unwrap_err();
    let shown = graph
        .outstanding_conflict_override(&page)
        .unwrap()
        .expect("the refused save mints the authority the banner shows");

    // Retire the exact editor the conflict was minted under — what clean
    // eviction, `forgetPage` and a genuine replacement each do.
    let activation = EditorActivation::from_u64(page.activation.unwrap());
    assert!(graph.retire_editor_activation("pages/Note.md", activation));

    assert!(
        graph
            .force_save_page_at_revision(&page, page.rev.as_deref(), shown)
            .is_err(),
        "a retired activation is no longer the live editor"
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "- external winner\n",
        "a refused override must not write"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gh254_successful_first_save_finishes_the_exact_activation_without_churn() {
    let root = scratch("gh254-inc3-first-save-activation");
    let graph = Graph::open(&root);
    graph.warm_cache();
    let prospective = graph
        .activate_absent_editor("First save", PageKind::Page)
        .unwrap();
    let mut page = PageDto {
        activation: Some(prospective.activation.as_u64()),
        name: "First save".into(),
        kind: PageKind::Page,
        title: "First save".into(),
        pre_block: None,
        blocks: vec![BlockDto {
            // DUP-8: spelled out at its `Default` value so a new `BlockDto`
            // field has to be decided here rather than arriving silently
            // defaulted.
            id: String::new(),
            raw: "created".into(),
            collapsed: false,
            children: Vec::new(),
            breadcrumb: Vec::new(),
            page_property: false,
            marker: None,
            priority: None,
            heading_level: None,
            scheduled: None,
            deadline: None,
            tags: Vec::new(),
            properties: Vec::new(),
        }],
        rev: None,
        format: Format::Md,
        read_only: false,
        path: prospective.target.clone(),
        guide: false,
    };

    let revision = graph.save_page(&page, None).unwrap();
    let finished = graph
        .finish_saved_editor_activation(prospective.activation)
        .expect("the successful first save must resolve its issuing activation");
    assert_eq!(finished.activation, prospective.activation);
    assert_eq!(finished.target, prospective.target);
    assert!(!finished.prospective);
    assert!(graph
        .finish_saved_editor_activation(prospective.activation)
        .is_none());

    page.rev = Some(revision);
    graph.save_page(&page, page.rev.as_deref()).unwrap();
    let reused = graph
        .activate_editor(
            &finished.target,
            ActivationIntent::Reuse,
            page.rev.as_deref(),
        )
        .unwrap();
    assert_eq!(reused.activation, prospective.activation);
    assert!(
        !reused.prospective,
        "an ordinary re-save must not churn or re-prospect the activation"
    );
    assert!(graph
        .finish_saved_editor_activation(EditorActivation::from_u64(
            prospective.activation.as_u64() + 1,
        ))
        .is_none());
    let _ = fs::remove_dir_all(root);
}

/// Reuse is idempotent; replace mints a second identity for a two-phase swap.
///
/// Both halves are load-bearing. Without idempotence, ordinary re-hydration of
/// an open page would burn the live editor's identity. Without replacement,
/// `reloadPage` would hand the disk snapshot the outgoing editor's identity.
#[test]
fn gh254_activation_reuse_is_idempotent_and_replace_is_not() {
    let (root, _path, graph, _page) = gh254_loaded("intent");
    let first = graph
        .activate_editor("pages/Note.md", ActivationIntent::Replace, None)
        .unwrap();
    let reused = graph
        .activate_editor("pages/Note.md", ActivationIntent::Reuse, None)
        .unwrap();
    assert_eq!(
        first.activation, reused.activation,
        "plain re-hydration must return the live activation, not mint one"
    );

    let replaced = graph
        .activate_editor("pages/Note.md", ActivationIntent::Replace, None)
        .unwrap();
    assert_ne!(
        first.activation, replaced.activation,
        "a genuine content replacement is a new editor instance"
    );
    assert!(
        graph.retire_editor_activation("pages/Note.md", first.activation),
        "A must remain live until the frontend installs B"
    );
    let reused_after_retiring_a = graph
        .activate_editor("pages/Note.md", ActivationIntent::Reuse, None)
        .unwrap();
    assert_eq!(
        reused_after_retiring_a.activation, replaced.activation,
        "compare-retiring A must not destroy the concurrently live B"
    );
    assert!(graph.retire_editor_activation("pages/Note.md", replaced.activation));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gh254_present_activation_refuses_a_snapshot_that_changed_after_read() {
    let (root, path, graph, page) = gh254_loaded("activation-expected-snapshot");
    fs::write(&path, "- winner after the DTO read\n").unwrap();

    let error = graph
        .activate_editor(
            "pages/Note.md",
            ActivationIntent::Replace,
            page.rev.as_deref(),
        )
        .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    let reused = graph
        .activate_editor("pages/Note.md", ActivationIntent::Reuse, None)
        .unwrap();
    assert_eq!(reused.activation.as_u64(), page.activation.unwrap());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gh254_implicit_force_cannot_spend_a_newer_observation_the_caller_never_named() {
    let (root, path, graph, page) = gh254_loaded("implicit-force-fails-closed");
    fs::write(&path, "- shown winner e1\n").unwrap();
    graph.save_page(&page, page.rev.as_deref()).unwrap_err();

    fs::write(&path, "- unseen winner e2\n").unwrap();
    graph.save_page(&page, page.rev.as_deref()).unwrap_err();

    let error = graph.force_save_page(&page).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(fs::read_to_string(&path).unwrap(), "- unseen winner e2\n");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gh254_token_cannot_cross_graph_instance_or_editor_episode() {
    let (root, path, graph, page) = gh254_loaded("scope");
    fs::write(&path, "- external winner\n").unwrap();
    let conflict = graph.save_page(&page, page.rev.as_deref()).unwrap_err();

    let reopened = Graph::open(&root);
    reopened.warm_cache();
    let mut transplanted = reopened.load_by_path("pages/Note.md").unwrap().unwrap();
    transplanted.blocks[0].raw = "transplanted mine".into();
    assert!(reopened.force_save_page(&transplanted).is_err());

    // Re-reading the same path no longer revokes, and that is the point of
    // increment 3's read/activation split: re-hydration happens constantly
    // (sidebar, live references, query hydration) and used to disarm a banner
    // the user could still see. The editor that was shown the conflict is
    // still live, so it can still answer it.
    graph.load_by_path("pages/Note.md").unwrap().unwrap();
    graph
        .force_save_page_at_revision(&page, page.rev.as_deref(), gh254_shown(&conflict))
        .expect("an unrelated read must not cost the live editor its answer");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gh254_token_cannot_cross_path_rename_or_successful_save() {
    // Exact-path scope: authority for Note is not authority for Other.
    let (root, path, graph, page) = gh254_loaded("path-scope");
    let other_path = root.join("pages/Other.md");
    fs::write(&other_path, "- other\n").unwrap();
    let mut other = graph.load_by_path("pages/Other.md").unwrap().unwrap();
    other.blocks[0].raw = "other mine".into();
    fs::write(&path, "- external winner\n").unwrap();
    graph.save_page(&page, page.rev.as_deref()).unwrap_err();
    assert!(graph.force_save_page(&other).is_err());

    // Any successful save on the token path revokes it, including a save
    // that becomes possible because disk returned to the loaded baseline.
    fs::write(&path, "- loaded\n").unwrap();
    graph.save_page(&page, page.rev.as_deref()).unwrap();
    assert!(graph.force_save_page(&page).is_err());
    let _ = fs::remove_dir_all(root);

    // Rename crosses the shared Tine-owned mutation boundary and revokes
    // source and destination rather than transplanting authority.
    let (root, path, graph, page) = gh254_loaded("rename-scope");
    fs::write(&path, "- external winner\n").unwrap();
    graph.save_page(&page, page.rev.as_deref()).unwrap_err();
    graph.rename_page("Note", "Renamed").unwrap();
    assert!(graph.force_save_page(&page).is_err());
    let _ = fs::remove_dir_all(root);
}

/// A rename's frontend refresh reloads only the pages it touched (GH #535), so
/// the backend retires only the MOVED page's activation: that editor dies with
/// the old name, and a later `Reuse` on the path must not inherit it. A
/// reference-rewritten page keeps its editor (see
/// `rename_refresh::a_rename_retires_only_the_moved_editor_and_a_stale_referrer_conflicts_reviewably`).
/// A failed rename retires nothing.
#[test]
fn gh254_successful_rename_retires_the_moved_editor_but_failure_does_not() {
    let root = scratch("gh254-inc3-rename-activation-lifecycle");
    let note_path = root.join("pages/Note.md");
    let other_path = root.join("pages/Other.md");
    fs::write(&note_path, "- [[Other]]\n").unwrap();
    fs::write(&other_path, "- [[Note]]\n").unwrap();
    let graph = Graph::open(&root);
    graph.warm_cache();

    let note = graph
        .activate_editor("pages/Note.md", ActivationIntent::Replace, None)
        .unwrap();
    let other = graph
        .activate_editor("pages/Other.md", ActivationIntent::Replace, None)
        .unwrap();

    assert!(graph.rename_page("Note", "").is_err());
    assert!(graph.retire_editor_activation("pages/Note.md", note.activation));
    assert!(graph.retire_editor_activation("pages/Other.md", other.activation));

    let note = graph
        .activate_editor("pages/Note.md", ActivationIntent::Replace, None)
        .unwrap();
    let other = graph
        .activate_editor("pages/Other.md", ActivationIntent::Replace, None)
        .unwrap();
    graph.rename_page("Note", "Renamed").unwrap();

    assert!(
        !graph.retire_editor_activation("pages/Note.md", note.activation),
        "the moved page's destroyed editor must be retired"
    );
    let reopened = graph
        .activate_editor("pages/Note.md", ActivationIntent::Reuse, None)
        .unwrap();
    assert_ne!(
        reopened.activation, note.activation,
        "Reuse on the moved path must mint for a new editor instance"
    );
    assert!(
        graph.retire_editor_activation("pages/Other.md", other.activation),
        "a reference-rewritten page's editor survives; the frontend replaces it"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gh254_newer_conflict_and_deletion_hook_advance_the_path_epoch() {
    let (root, path, graph, page) = gh254_loaded("epoch");
    fs::write(&path, "- winner one\n").unwrap();
    graph.save_page(&page, page.rev.as_deref()).unwrap_err();
    let first_epoch = graph
        .conflict_authority
        .lock()
        .unwrap()
        .tokens
        .get(&path)
        .unwrap()
        .observation_epoch;

    fs::write(&path, "- winner two\n").unwrap();
    graph.save_page(&page, page.rev.as_deref()).unwrap_err();
    let second_epoch = graph
        .conflict_authority
        .lock()
        .unwrap()
        .tokens
        .get(&path)
        .unwrap()
        .observation_epoch;
    assert!(second_epoch > first_epoch);

    graph.forget_file(&path);
    assert!(!graph
        .loaded_file_identities
        .read()
        .unwrap()
        .contains_key(&path));
    let state = graph.conflict_authority.lock().unwrap();
    assert!(!state.tokens.contains_key(&path));
    assert!(state.observation_epochs[&path] > second_epoch);
    let _ = fs::remove_dir_all(root);
}

/// A same-byte republication is the SAME STATE, so "Keep mine" goes through
/// and lands on the inode that is actually at the path now.
///
/// Bytes decide; a changed resource identity does not veto. Refusing here
/// would make force stricter than an ordinary save — which already treats a
/// same-byte replacement as the state it already has — and would hand the
/// user a fresh, visually identical banner to click again in exactly the
/// Syncthing scenario GH #254 exists for. Martin's 2026-08-09 ruling: state
/// decides, and the state the user was shown is still what is on disk.
#[cfg(any(unix, windows))]
#[test]
fn gh254_force_accepts_a_same_byte_republication_and_targets_the_live_inode() {
    let (root, path, graph, page) = gh254_loaded("force-identity");
    fs::write(&path, "- shown winner\n").unwrap();
    let conflict = graph.save_page(&page, page.rev.as_deref()).unwrap_err();
    let replacement = path.with_file_name(".same-byte-new-inode");
    fs::write(&replacement, "- shown winner\n").unwrap();
    gh254_replace(&path, &replacement).unwrap();

    graph
        .force_save_page_at_revision(&page, page.rev.as_deref(), gh254_shown(&conflict))
        .unwrap();
    assert!(
        fs::read_to_string(&path).unwrap().contains("mine"),
        "keep-mine did not land on the republished inode"
    );
    assert!(
        !replacement.exists(),
        "the staging name must not survive as a stray sibling"
    );
    let _ = fs::remove_dir_all(root);
}

/// The other half of the same rule: a DIFFERENT-byte winner on a new inode
/// is still refused, and is left exactly as that winner wrote it.
/// The observation a banner-class conflict named, as the UI echoes it back.
fn gh254_shown(error: &io::Error) -> ConflictOverride {
    ConflictOverride {
        observation_epoch: direct_save_conflict_epoch(error)
            .expect("a banner-class conflict names its observation"),
    }
}

/// Adversarial implementation verification, finding 1. Two force requests
/// issued under ONE banner: the button is not disabled while its request is
/// pending, and the per-page save queue serializes them. An external writer
/// publishes B after the banner showed A. The first request correctly
/// refuses B — and, being a coherent observation, mints fresh authority FOR
/// B. The second request, issued before B existed, must not be able to
/// spend it: the user never saw B.
#[cfg(any(unix, windows))]
#[test]
fn gh254_a_second_force_under_one_banner_cannot_spend_authority_for_an_unseen_winner() {
    let (root, path, graph, page) = gh254_loaded("force-double-click");
    fs::write(&path, "- shown winner A\n").unwrap();
    let shown = graph.save_page(&page, page.rev.as_deref()).unwrap_err();
    let banner = ConflictOverride {
        observation_epoch: direct_save_conflict_epoch(&shown)
            .expect("a banner-class conflict names its observation"),
    };

    // …and then B lands, unseen.
    fs::write(&path, "- winner B, which nobody was shown\n").unwrap();

    // Click one: refuses B and re-banners over it.
    let first = graph
        .force_save_page_at_revision(&page, page.rev.as_deref(), banner)
        .unwrap_err();
    assert_eq!(gh254_code(&first), "conflict.save_baseline_present");

    // Click two, already in flight under the SAME banner.
    let second = graph
        .force_save_page_at_revision(&page, page.rev.as_deref(), banner)
        .unwrap_err();
    assert_eq!(second.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "- winner B, which nobody was shown\n",
        "a duplicated request overwrote a winner the user never saw"
    );

    // The user, now shown B, can still resolve it deliberately.
    let over_b = ConflictOverride {
        observation_epoch: direct_save_conflict_epoch(&first).unwrap(),
    };
    graph
        .force_save_page_at_revision(&page, page.rev.as_deref(), over_b)
        .unwrap();
    assert!(fs::read_to_string(&path).unwrap().contains("mine"));
    let _ = fs::remove_dir_all(root);
}

/// Refusing a mis-addressed override must not SPEND the live one. Checking
/// ownership after taking would let one stray duplicate click disarm the
/// banner permanently: the user would keep seeing a conflict whose "Keep
/// mine" can never work, with only the destructive button left.
#[cfg(any(unix, windows))]
#[test]
fn gh254_a_mis_addressed_override_does_not_disarm_the_live_banner() {
    let (root, path, graph, page) = gh254_loaded("force-no-disarm");
    fs::write(&path, "- the winner\n").unwrap();
    let shown = graph.save_page(&page, page.rev.as_deref()).unwrap_err();
    let live = gh254_shown(&shown);

    let wrong = ConflictOverride {
        observation_epoch: live.observation_epoch + 7,
    };
    let refused = graph
        .force_save_page_at_revision(&page, page.rev.as_deref(), wrong)
        .unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(fs::read_to_string(&path).unwrap(), "- the winner\n");

    // The banner the user is looking at still resolves.
    graph
        .force_save_page_at_revision(&page, page.rev.as_deref(), live)
        .unwrap();
    assert!(fs::read_to_string(&path).unwrap().contains("mine"));
    let _ = fs::remove_dir_all(root);
}

/// The same handle also stops a stale override from a second editor episode
/// that loaded the same revision: it can only present an epoch it was shown,
/// and the live token has moved past it.
#[cfg(any(unix, windows))]
#[test]
fn gh254_an_override_naming_a_superseded_observation_is_refused() {
    let (root, path, graph, page) = gh254_loaded("force-stale-episode");
    fs::write(&path, "- first winner\n").unwrap();
    let first = graph.save_page(&page, page.rev.as_deref()).unwrap_err();
    let stale = ConflictOverride {
        observation_epoch: direct_save_conflict_epoch(&first).unwrap(),
    };

    // A later observation supersedes it.
    fs::write(&path, "- second winner\n").unwrap();
    let second = graph.save_page(&page, page.rev.as_deref()).unwrap_err();
    assert!(direct_save_conflict_epoch(&second).unwrap() > stale.observation_epoch);

    let refused = graph
        .force_save_page_at_revision(&page, page.rev.as_deref(), stale)
        .unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(fs::read_to_string(&path).unwrap(), "- second winner\n");
    let _ = fs::remove_dir_all(root);
}

#[cfg(any(unix, windows))]
#[test]
fn gh254_force_refuses_a_different_byte_winner_on_a_new_inode() {
    let (root, path, graph, page) = gh254_loaded("force-identity-diff");
    fs::write(&path, "- shown winner\n").unwrap();
    let shown = graph.save_page(&page, page.rev.as_deref()).unwrap_err();
    let replacement = path.with_file_name(".different-byte-new-inode");
    fs::write(&replacement, "- a newer winner nobody saw\n").unwrap();
    gh254_replace(&path, &replacement).unwrap();

    let error = graph
        .force_save_page_at_revision(&page, page.rev.as_deref(), gh254_shown(&shown))
        .unwrap_err();
    assert_eq!(gh254_code(&error), "conflict.save_baseline_present");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "- a newer winner nobody saw\n",
        "an unseen winner must survive Keep mine"
    );
    // The refusal minted a fresh conflict over the winner the user can now
    // actually see, so the second click resolves it deliberately.
    graph
        .force_save_page_at_revision(&page, page.rev.as_deref(), gh254_shown(&error))
        .unwrap();
    let _ = fs::remove_dir_all(root);
}

#[cfg(any(unix, windows))]
#[test]
fn gh254_s1_initial_present_baseline_mismatch_mints_exact_winner() {
    let (root, path, graph, page) = gh254_loaded("s1");
    fs::write(&path, "- s1 winner\n").unwrap();
    let error = graph.save_page(&page, page.rev.as_deref()).unwrap_err();
    assert_eq!(gh254_code(&error), "conflict.save_baseline_present");
    graph
        .force_save_page_at_revision(&page, page.rev.as_deref(), gh254_shown(&error))
        .unwrap();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gh254_s2_initial_absence_mints_absent() {
    let (root, path, graph, page) = gh254_loaded("s2");
    fs::remove_file(path).unwrap();
    let error = graph.save_page(&page, page.rev.as_deref()).unwrap_err();
    assert_eq!(gh254_code(&error), "conflict.save_baseline_absent");
    graph
        .force_save_page_at_revision(&page, page.rev.as_deref(), gh254_shown(&error))
        .unwrap();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gh254_s3_retired_snapshot_reads_bytes_and_identity_together() {
    let (root, path, graph, page) = gh254_loaded("s3");
    GRAPH_TEXT_WRITE_BEFORE_MUTATION.with(|hook| {
        let path = path.clone();
        *hook.borrow_mut() = Some(Box::new(move || fs::write(path, "- s3 winner\n")));
    });
    let error = graph.save_page(&page, page.rev.as_deref()).unwrap_err();
    assert_eq!(gh254_code(&error), "conflict.replace_retired_mismatch");
    graph
        .force_save_page_at_revision(&page, page.rev.as_deref(), gh254_shown(&error))
        .unwrap();
    let _ = fs::remove_dir_all(root);
}

#[cfg(any(unix, windows))]
#[test]
fn gh254_s4_pre_retirement_identity_change_observes_live_winner() {
    let (root, path, graph, page) = gh254_loaded("s4");
    let replacement = path.with_file_name(".s4-winner");
    fs::write(&replacement, "- s4 winner\n").unwrap();
    GRAPH_TEXT_WRITE_BEFORE_MUTATION.with(|hook| {
        let path = path.clone();
        *hook.borrow_mut() = Some(Box::new(move || gh254_replace(&path, &replacement)));
    });
    let error = graph.save_page(&page, page.rev.as_deref()).unwrap_err();
    assert_eq!(gh254_code(&error), "conflict.replace_pre_retirement");
    graph
        .force_save_page_at_revision(&page, page.rev.as_deref(), gh254_shown(&error))
        .unwrap();
    let _ = fs::remove_dir_all(root);
}

/// §4.6 / §4.3 F3: the editor writer's displacement fault point produces
/// "displaced, not yet published" — the live name gone, the
/// `.editor-recovery` claim holding the exact precondition.
#[test]
fn the_editor_displacement_hook_observes_the_unpublished_displaced_state() {
    let (root, path, graph, page) = gh254_loaded("editor-displacement-hook");
    let parent = path.parent().unwrap().to_path_buf();
    let observed: std::sync::Arc<std::sync::Mutex<Option<(bool, Vec<(String, Vec<u8>)>)>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let recorder = std::sync::Arc::clone(&observed);
    GRAPH_TEXT_WRITE_AFTER_RETIRE.with(|hook| {
        let path = path.clone();
        let parent = parent.clone();
        *hook.borrow_mut() = Some(Box::new(move || {
            let mut claims = fs::read_dir(&parent)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                .filter(|name| name.ends_with(".editor-recovery"))
                .map(|name| {
                    let bytes = fs::read(parent.join(&name)).unwrap();
                    (name, bytes)
                })
                .collect::<Vec<_>>();
            claims.sort();
            *recorder.lock().unwrap() = Some((path.exists(), claims));
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "injected displacement cut",
            ))
        }));
    });
    let error = graph.save_page(&page, page.rev.as_deref()).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted, "{error}");

    let (target_present, claims) = observed
        .lock()
        .unwrap()
        .take()
        .expect("the editor displacement hook must fire");
    assert!(
        !target_present,
        "the live name must already be gone at the displacement cut"
    );
    assert_eq!(claims.len(), 1, "exactly one editor-recovery claim");
    assert_eq!(
        claims[0].1,
        b"- loaded\n".to_vec(),
        "the claim holds the exact precondition bytes"
    );
    // In-process the writer still restores; the crash disposition is W4's,
    // and lands with the producer conversion.
    assert_eq!(fs::read_to_string(&path).unwrap(), "- loaded\n");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gh254_s5_retired_mismatch_mints_only_after_restore() {
    let (root, path, graph, page) = gh254_loaded("s5");
    GRAPH_TEXT_WRITE_BEFORE_MUTATION.with(|hook| {
        let path = path.clone();
        *hook.borrow_mut() = Some(Box::new(move || fs::write(path, "- s5 winner\n")));
    });
    let error = graph.save_page(&page, page.rev.as_deref()).unwrap_err();
    assert_eq!(gh254_code(&error), "conflict.replace_retired_mismatch");
    assert_eq!(fs::read_to_string(&path).unwrap(), "- s5 winner\n");
    graph
        .force_save_page_at_revision(&page, page.rev.as_deref(), gh254_shown(&error))
        .unwrap();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn gh254_s6_publication_collision_observes_after_restore_outcome() {
    for restore_succeeds in [false, true] {
        let (root, path, graph, page) = gh254_loaded(if restore_succeeds {
            "s6-restored"
        } else {
            "s6-live"
        });
        GRAPH_TEXT_WRITE_AFTER_RETIRE.with(|hook| {
            let path = path.clone();
            *hook.borrow_mut() = Some(Box::new(move || fs::write(path, "- s6 transient\n")));
        });
        if restore_succeeds {
            GRAPH_TEXT_WRITE_BEFORE_RESTORE.with(|hook| {
                let path = path.clone();
                *hook.borrow_mut() = Some(Box::new(move || fs::remove_file(path)));
            });
        }
        let error = graph.save_page(&page, page.rev.as_deref()).unwrap_err();
        assert_eq!(gh254_code(&error), "conflict.replace_publication_collision");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            if restore_succeeds {
                "- loaded\n"
            } else {
                "- s6 transient\n"
            }
        );
        graph
            .force_save_page_at_revision(&page, page.rev.as_deref(), gh254_shown(&error))
            .unwrap();
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn gh254_s7_absent_creation_losing_noreplace_race_mints_present() {
    let root = scratch("gh254-increment2-s7");
    let path = root.join("pages/New.md");
    let graph = Graph::open(&root);
    let mut page = PageDto {
        activation: None,
        name: "New".into(),
        kind: PageKind::Page,
        title: "New".into(),
        pre_block: None,
        blocks: vec![BlockDto {
            // DUP-8: spelled out at its `Default` value so a new `BlockDto`
            // field has to be decided here rather than arriving silently
            // defaulted.
            id: String::new(),
            raw: "mine".into(),
            collapsed: false,
            children: Vec::new(),
            breadcrumb: Vec::new(),
            page_property: false,
            marker: None,
            priority: None,
            heading_level: None,
            scheduled: None,
            deadline: None,
            tags: Vec::new(),
            properties: Vec::new(),
        }],
        rev: None,
        format: Format::Md,
        read_only: false,
        path: String::new(),
        guide: false,
    };
    // An ABSENT editor: no file, no revision. Increment 3 gives it a real
    // activation anyway, because it can meet an external-create conflict on
    // its very first save — which is exactly what this test then does. Direct
    // creation deliberately fails closed without a warm semantic snapshot,
    // so install the ordinary open-time evidence before arming the race.
    graph.warm_cache();
    let handle = graph.activate_absent_editor("New", PageKind::Page).unwrap();
    assert!(handle.prospective, "no file exists for New yet");
    page.activation = Some(handle.activation.as_u64());
    GRAPH_TEXT_WRITE_BEFORE_MUTATION.with(|hook| {
        let path = path.clone();
        *hook.borrow_mut() = Some(Box::new(move || fs::write(path, "- s7 winner\n")));
    });
    let error = graph.save_page(&page, None).unwrap_err();
    assert_eq!(gh254_code(&error), "conflict.create_publication_collision");
    graph
        .force_save_page_at_revision(&page, None, gh254_shown(&error))
        .unwrap();
    let _ = fs::remove_dir_all(root);
}

#[cfg(any(unix, windows))]
#[test]
fn gh254_s8_final_reread_covers_present_and_absent_arms() {
    for absent in [false, true] {
        let (root, path, graph, page) =
            gh254_loaded(if absent { "s8-absent" } else { "s8-present" });
        EDITOR_COMMIT_BEFORE_FINAL_REREAD.with(|hook| {
            let path = path.clone();
            *hook.borrow_mut() = Some(Box::new(move || {
                if absent {
                    fs::remove_file(path)
                } else {
                    let replacement = path.with_file_name(".s8-winner");
                    fs::write(&replacement, "- s8 winner\n")?;
                    gh254_replace(&path, &replacement)
                }
            }));
        });
        let error = graph.save_page(&page, page.rev.as_deref()).unwrap_err();
        assert_eq!(
            gh254_code(&error),
            if absent {
                "conflict.final_reread_absent"
            } else {
                "conflict.final_reread_present"
            }
        );
        graph
            .force_save_page_at_revision(&page, page.rev.as_deref(), gh254_shown(&error))
            .unwrap();
        let _ = fs::remove_dir_all(root);
    }
}

#[cfg(any(unix, windows))]
#[test]
fn gh254_s9_post_publication_validation_observes_after_cleanup() {
    let (root, path, graph, page) = gh254_loaded("s9");
    JOURNAL_PROJECTION_AFTER_PUBLISH.with(|hook| {
        let path = path.clone();
        *hook.borrow_mut() = Some(Box::new(move || {
            let replacement = path.with_file_name(".s9-winner");
            fs::write(&replacement, "- s9 winner\n")?;
            gh254_replace(&path, &replacement)
        }));
    });
    let error = graph.save_page(&page, page.rev.as_deref()).unwrap_err();
    assert_eq!(gh254_code(&error), "conflict.replace_post_publication");
    graph
        .force_save_page_at_revision(&page, page.rev.as_deref(), gh254_shown(&error))
        .unwrap();
    let _ = fs::remove_dir_all(root);
}

#[cfg(any(unix, windows))]
#[test]
fn gh254_tokenless_observation_failure_is_retryable_but_not_banner_class() {
    let (root, path, graph, page) = gh254_loaded("tokenless");
    let replacement = path.with_file_name(".tokenless-winner");
    fs::write(&replacement, "- winner\n").unwrap();
    GRAPH_TEXT_WRITE_BEFORE_MUTATION.with(|hook| {
        let path = path.clone();
        *hook.borrow_mut() = Some(Box::new(move || gh254_replace(&path, &replacement)));
    });
    CONFLICT_OBSERVATION.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(|| {
            Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "continued delivery churn",
            ))
        }));
    });
    let error = graph.save_page(&page, page.rev.as_deref()).unwrap_err();
    assert_eq!(gh254_code(&error), "conflict_retry.replace_pre_retirement");
    assert!(!gh254_code(&error).starts_with("conflict."));
    assert!(graph.force_save_page(&page).is_err());
    let _ = fs::remove_dir_all(root);
}

// P-01 (2026-09-01 debt audit): the graph-text identity gate is graph-global
// and exclusive across threads; the per-page lock is per-path. Three writers
// took the gate first and four took the page lock first, so an editor save and
// a PDF-highlight write of the SAME `hls__` page deadlocked each other — and
// because the saver holds the graph-global gate while it waits, every later
// graph-text write in the process wedged too. That is an app-wide hang, and it
// is invisible to `debug_assert`s: the shipped release profile compiles them
// out, so the release binary reached the deadlock instead of the assertion.
//
// The invariant is an ordering one and therefore static: any function that
// holds a page lock while it (transitively) acquires the identity gate is a
// deadlock against every function that takes them the other way round. This
// guard walks the model module's call graph (model.rs and its K3 seam files
// under model/) and fails on any such function.
#[test]
fn graph_text_writers_take_the_identity_gate_before_any_page_lock() {
    use std::collections::{HashMap, HashSet};

    // The model module is model.rs plus its K3 seam files under model/. Each
    // line keeps its file and line number, so a finding names the file on disk.
    let files = crate::test_support::model_module_files();
    let mut lines: Vec<&str> = Vec::new();
    let mut origin: Vec<(&str, usize)> = Vec::new();
    let mut file_starts: Vec<usize> = Vec::new();
    for (path, text) in &files {
        file_starts.push(lines.len());
        for (number, line) in text.lines().enumerate() {
            lines.push(line);
            origin.push((path.as_str(), number + 1));
        }
    }
    let is_fn_start = |line: &str| {
        line.starts_with("    fn ")
            || line.starts_with("    pub fn ")
            || line.starts_with("    pub(crate) fn ")
            || line.starts_with("    pub(super) fn ")
    };
    let mut starts: Vec<usize> = (0..lines.len())
        .filter(|i| is_fn_start(lines[*i]))
        .collect();
    starts.push(lines.len());
    // A body ends at the next function or at the end of its own file.
    file_starts.push(lines.len());
    let end_of = |a: usize, b: usize| {
        file_starts
            .iter()
            .copied()
            .find(|&edge| edge > a)
            .map_or(b, |edge| edge.min(b))
    };
    assert!(
        starts.len() > 400,
        "the function scan found only {} candidates; the source shape changed",
        starts.len()
    );

    fn name_of(line: &str) -> &str {
        let rest = line
            .trim_start()
            .trim_start_matches("pub(crate) ")
            .trim_start_matches("pub(super) ")
            .trim_start_matches("pub ")
            .trim_start_matches("fn ");
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        &rest[..end]
    }
    fn callees(body: &str) -> HashSet<&str> {
        let mut found = HashSet::new();
        let mut rest = body;
        while let Some(at) = rest.find("self.") {
            rest = &rest[at + 5..];
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(rest.len());
            if rest[end..].starts_with('(') {
                found.insert(&rest[..end]);
            }
        }
        found
    }

    let mut bodies: HashMap<&str, Vec<String>> = HashMap::new();
    for pair in starts.windows(2) {
        let (a, b) = (pair[0], end_of(pair[0], pair[1]));
        bodies
            .entry(name_of(lines[a]))
            .or_default()
            .push(lines[a..b].join("\n"));
    }

    // Transitive closure of "can acquire the graph-text identity gate".
    let mut acquires: HashSet<&str> = bodies
        .iter()
        .filter(|(_, list)| {
            list.iter()
                .any(|body| body.contains("lock_graph_text_identity_mutation()"))
        })
        .map(|(name, _)| *name)
        .collect();
    // Seeded on the writers that have always taken the gate directly, so this
    // sanity check cannot silently encode the fix it is guarding.
    for expected in ["save_page", "force_save_page_at_revision", "merge_pages"] {
        assert!(
            acquires.contains(expected),
            "the gate-acquisition seed is wrong: `{expected}` acquires the identity gate but \
             the scan did not see it, so this guard would pass vacuously"
        );
    }
    loop {
        let mut grew = false;
        for (name, list) in &bodies {
            if acquires.contains(name) {
                continue;
            }
            if list
                .iter()
                .any(|body| callees(body).iter().any(|c| acquires.contains(c)))
            {
                acquires.insert(name);
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }

    let mut inversions: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for pair in starts.windows(2) {
        let (a, b) = (pair[0], end_of(pair[0], pair[1]));
        let body = lines[a..b].join("\n");
        let Some(page_lock_at) = body.find("self.page_lock(") else {
            continue;
        };
        checked += 1;
        let gate_at = body.find("lock_graph_text_identity_mutation");
        if gate_at.is_some_and(|gate| gate < page_lock_at) {
            continue;
        }
        let under_lock = &body[page_lock_at..];
        let mut reaching: Vec<&str> = callees(under_lock)
            .into_iter()
            .filter(|c| acquires.contains(c))
            .collect();
        if reaching.is_empty() {
            continue;
        }
        reaching.sort_unstable();
        inversions.push(format!(
            "{} ({}:{}) holds a page lock and then reaches the identity gate via {reaching:?}",
            name_of(lines[a]),
            origin[a].0,
            origin[a].1
        ));
    }
    assert!(
        checked >= 16,
        "only {checked} page-lock holders were examined; the guard lost its subjects"
    );
    assert!(
        inversions.is_empty(),
        "graph-text identity gate / page lock order inverted — this deadlocks the whole \
         process against `save_page`. Take `lock_graph_text_identity_mutation()` before the \
         page lock, as `save_page`, `force_save_page_at_revision` and `merge_pages` do:\n{}",
        inversions.join("\n")
    );
}

// ---------------------------------------------------------------------------
// The observed property registry: lifecycle and cache identity
// (SPEC §6.2 end, §6.4, §5.9 guards (b) and (c); dossier B3, B4, O11).

/// A graph with one property key, one page NAMED after that key carrying a
/// `tine.type::` declaration, and one page that is not a key page.
fn registry_graph(tag: &str) -> PathBuf {
    let dir = scratch(tag);
    fs::write(
        dir.join("pages/Data.md"),
        "- row one\n  score:: 01\n- row two\n  score:: 02\n",
    )
    .unwrap();
    fs::write(
        dir.join("pages/score.md"),
        "tine.type:: number\n\n- the key page\n",
    )
    .unwrap();
    fs::write(dir.join("pages/Unrelated.md"), "- nothing to do with it\n").unwrap();
    dir
}

/// A declaration page changes the meaning of a property query on another page.
/// `01` is the number 1 under a number key and the text `01` under a text key.
#[test]
fn a_declared_type_change_retypes_the_next_sql_query() {
    let dir = registry_graph("registry-declared-type-query");
    let graph = ready_graph(&dir);

    let matched = |graph: &Graph| -> usize {
        when_ready(|| graph.run_query_bounded("(property score 1)", usize::MAX, usize::MAX))
            .groups
            .iter()
            .map(|group| group.blocks.len())
            .sum()
    };
    assert_eq!(
        matched(&graph),
        1,
        "under a number key `01` is the number 1"
    );

    let mut key_page = graph.load_named("score", PageKind::Page).unwrap().unwrap();
    key_page.pre_block = Some("tine.type:: text".to_string());
    graph.save_page(&key_page, key_page.rev.as_deref()).unwrap();
    wait_for_direct_query_projection(&graph);

    assert_eq!(matched(&graph), 0, "under a text key `01` is not `1`");
    let _ = fs::remove_dir_all(dir);
}

/// SPEC §5.9 guard (c), Direct Files half (E6): a query with NO property leaf
/// is still config-sensitive, because `journal_page_title_format` decides
/// whether a page in `pages/` is a journal day at all. The config is read at
/// open, so the guard is a reopen and the next execution must use the new
/// digest.
#[test]
fn a_journal_title_format_change_answers_a_journal_day_query_anew() {
    let dir = scratch("registry-journal-title-format");
    fs::create_dir_all(dir.join("logseq")).unwrap();
    // A title none of the default fallback patterns recognises (`yyyy_MM_dd`,
    // `MMM do, yyyy`, `yyyy-MM-dd`), so the classification turns on the
    // configured title format alone.
    fs::write(
        dir.join("pages/25-12-2020.md"),
        "- a day written as a page\n",
    )
    .unwrap();

    let day_rows = |graph: &Graph| -> usize {
        when_ready(|| graph.run_query_bounded("(between -10y +10y)", usize::MAX, usize::MAX))
            .groups
            .iter()
            .map(|group| group.blocks.len())
            .sum()
    };

    let before = ready_graph(&dir);
    assert_eq!(
        day_rows(&before),
        0,
        "under the default formats the title parses as nothing, so it is an ordinary page"
    );
    let before_digest = before.config.parse_config().digest();
    drop(before);

    fs::write(
        dir.join("logseq/config.edn"),
        "{:journal/page-title-format \"dd-MM-yyyy\"}\n",
    )
    .unwrap();
    let after = ready_graph(&dir);
    let after_digest = after.config.parse_config().digest();
    assert_ne!(
        before_digest, after_digest,
        "the title format is one of the six projected-fact inputs (§5.8)"
    );
    assert_eq!(
        day_rows(&after),
        1,
        "the same page is now a journal day inside the interval"
    );
    let _ = fs::remove_dir_all(dir);
}

/// O11: the registry built from the Direct Files READY PROJECTION STREAM and the
/// one built from the Direct Files cold document iterator are one table. The
/// producers disagree only about
/// opaque owner identity and page identity, neither of which the aggregation
/// carries.
#[test]
fn every_implemented_row_source_builds_the_same_registry() {
    let dir = registry_graph("registry-source-identity");
    fs::write(
        dir.join("pages/More.md"),
        "tags:: alpha, beta\n\n- another row\n  score:: 7\n  note:: [[Book]], plain\n",
    )
    .unwrap();
    let graph = Graph::open(&dir);
    graph
        .attach_direct_projection(dir.join(".registry-sources/projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    let config = graph.config.parse_config();

    let (document_rows, document_pages) = crate::query::property_owner_rows(&graph);
    let from_documents = crate::query::registry::build_registry(
        document_rows.into_iter(),
        &|page_id| document_pages.get(page_id).cloned(),
        &config,
    )
    .expect("every document row names its own page");

    assert!(
        from_documents
            .rows()
            .iter()
            .any(|row| row.normalized_name == "score"),
        "the fixture's key is actually in the table"
    );

    // The third source: the ready projection's raw stream. It exists so that a
    // registry read on a ready Direct Files graph does not walk every hydrated
    // document — an adapter onto the SAME `build_registry`, never a second
    // registry producer.
    wait_for_direct_query_projection(&graph);
    let (stream_rows, stream_pages) = graph
        .direct_projection_property_owner_rows()
        .expect("a ready projection answers the registry's row source");
    let from_projection = crate::query::registry::build_registry(
        stream_rows.into_iter(),
        &|page_id| stream_pages.get(page_id).cloned(),
        &config,
    )
    .expect("every projection row names its own page");
    assert_eq!(
        from_projection.rows(),
        from_documents.rows(),
        "the ready projection stream and the cold document walk build one table"
    );

    // And the graph's own registry, which now PREFERS the ready stream, is that
    // same table — the adapter is wired, not merely present. Equal tables alone
    // cannot show WHICH source answered (that is the point of the guard above),
    // so the projection's own read counter is the witness: a rebuild that fell
    // back to the document iterator touches the projection zero times.
    let reads_before = graph.direct_projection_indexed_reads_test();
    assert_eq!(graph.property_registry().rows(), from_documents.rows());
    assert!(
        graph.direct_projection_indexed_reads_test() > reads_before,
        "the oracle's registry on a ready Direct Files graph reads the projection"
    );
    let _ = fs::remove_dir_all(dir);
}

/// §6.2's guard on a REAL graph: the registry built from each of the three row
/// sources reports identical rows. Wave B proved it between two sources on a
/// constructed fixture; this proves it across all three on the anonymized
/// corpus, which is the graph whose property vocabulary the campaign's cost
/// bound is measured against.
///
/// ```text
/// TINE_REGISTRY_TIMING_GRAPH=~/research/logseq-anonymized \
///   cargo test --release -p tine-core registry_row_sources_agree_on_a_real_graph \
///   -- --ignored --nocapture
/// ```
///
/// Ignored by default: it needs a graph this repository does not ship. It
/// prints counts only, never content.
#[test]
#[ignore = "needs a real graph named by TINE_REGISTRY_TIMING_GRAPH"]
fn registry_row_sources_agree_on_a_real_graph() {
    let Ok(root) = std::env::var("TINE_REGISTRY_TIMING_GRAPH") else {
        panic!("set TINE_REGISTRY_TIMING_GRAPH to a graph directory");
    };
    let graph = Graph::open(std::path::Path::new(&root));
    let projection_dir =
        std::env::temp_dir().join(format!("tine-registry-sources-{}", std::process::id()));
    let _ = fs::remove_dir_all(&projection_dir);
    fs::create_dir_all(&projection_dir).unwrap();
    graph
        .attach_direct_projection(projection_dir.join("projection.sqlite"))
        .unwrap();
    graph.warm_cache();
    let config = graph.config.parse_config();

    let (document_rows, document_pages) = crate::query::property_owner_rows(&graph);
    let document_row_count = document_rows.len();
    let from_documents = crate::query::registry::build_registry(
        document_rows.into_iter(),
        &|page_id| document_pages.get(page_id).cloned(),
        &config,
    )
    .expect("every document row names its own page");

    wait_for_direct_query_projection(&graph);
    let (stream_rows, stream_pages) = graph
        .direct_projection_property_owner_rows()
        .expect("a ready projection answers the registry's row source");
    let stream_row_count = stream_rows.len();
    let from_projection = crate::query::registry::build_registry(
        stream_rows.into_iter(),
        &|page_id| stream_pages.get(page_id).cloned(),
        &config,
    )
    .expect("every projection row names its own page");

    println!(
        "REGISTRYSOURCES	graph=<named by env>	document_rows={document_row_count}	projection_rows={stream_row_count}	keys={}",
        from_documents.rows().len()
    );
    assert_eq!(
        from_projection.rows(),
        from_documents.rows(),
        "ready projection stream vs cold document walk"
    );
    let _ = fs::remove_dir_all(&projection_dir);
}

/// The registry build is a whole-graph pass, so its cost is reported rather
/// than assumed. The bound the dossier asks for is measured on the anonymized
/// corpus by the lane and recorded in the receipt; this test measures the same
/// code on a graph it can construct, so a regression that made the build
/// quadratic fails here instead of in the field.
#[test]
fn the_registry_build_is_measured_and_bounded() {
    let dir = scratch("registry-build-timing");
    for page in 0..200 {
        let mut text = String::new();
        for block in 0..10 {
            text.push_str(&format!(
                "- block {block}\n  score:: {block}\n  note:: [[Ref {block}]], plain text\n"
            ));
        }
        fs::write(dir.join(format!("pages/Page {page}.md")), text).unwrap();
    }
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let config = graph.config.parse_config();

    let mut micros = Vec::new();
    for _ in 0..20 {
        let started = Instant::now();
        let (rows, pages) = crate::query::property_owner_rows(&graph);
        let registry = crate::query::registry::build_registry(
            rows.into_iter(),
            &|page_id| pages.get(page_id).cloned(),
            &config,
        )
        .expect("every row names its own page");
        micros.push(started.elapsed().as_micros() as u64);
        assert!(!registry.rows().is_empty());
    }
    micros.sort_unstable();
    let median = micros[micros.len() / 2];
    println!("REGISTRYBUILD\tpages=200\tblocks=2000\tmedian_micros={median}");
    assert!(
        median < 2_000_000,
        "a 2000-block graph must not take seconds to aggregate: {median} us"
    );
    let _ = fs::remove_dir_all(dir);
}

/// The same measurement against a REAL graph, named by
/// `TINE_REGISTRY_TIMING_GRAPH`, so the dossier's bound can be re-measured on
/// the anonymized corpus by anyone who has it:
///
/// ```text
/// TINE_REGISTRY_TIMING_GRAPH=~/research/logseq-anonymized \
///   cargo test --release -p tine-core registry_build_timing_on_a_real_graph \
///   -- --ignored --nocapture
/// ```
///
/// Ignored by default: it needs a graph this repository does not ship, and it
/// prints a number rather than asserting one — the bound is a receipt line the
/// manager reads, not a threshold that should fail a build on a slow machine.
/// It prints page and block COUNTS and a duration, never any content.
#[test]
#[ignore = "needs a real graph named by TINE_REGISTRY_TIMING_GRAPH"]
fn registry_build_timing_on_a_real_graph() {
    let Ok(root) = std::env::var("TINE_REGISTRY_TIMING_GRAPH") else {
        panic!("set TINE_REGISTRY_TIMING_GRAPH to a graph directory");
    };
    let graph = Graph::open(std::path::Path::new(&root));
    graph.warm_cache();
    let config = graph.config.parse_config();

    let mut micros = Vec::new();
    let mut rows_seen = 0usize;
    let mut keys_seen = 0usize;
    for _ in 0..20 {
        let started = Instant::now();
        let (rows, pages) = crate::query::property_owner_rows(&graph);
        rows_seen = rows.len();
        let registry = crate::query::registry::build_registry(
            rows.into_iter(),
            &|page_id| pages.get(page_id).cloned(),
            &config,
        )
        .expect("every row names its own page");
        micros.push(started.elapsed().as_micros() as u64);
        keys_seen = registry.rows().len();
    }
    micros.sort_unstable();
    println!(
        "REGISTRYBUILD\tgraph=<named by env>\tproperty_rows={rows_seen}\tkeys={keys_seen}\tmedian_micros={}",
        micros[micros.len() / 2]
    );
}

/// A merge retires the source the moment its file leaves the graph, before the
/// destination is rewritten, so no index that calls itself complete ever
/// contains a page whose file is already in the trash (GH #543, fifth audit
/// A5-N4). When the destination write then fails, the rollback puts the source
/// file back — and its retirement has to come back with it, or a failed merge
/// leaves the graph missing a page that exists, with nothing queued to notice.
#[test]
fn a_failed_merge_puts_the_source_back_in_the_index_too() {
    let dir = scratch("merge-rollback-republishes-source");
    fs::write(dir.join("pages").join("source.md"), "- pangolin source\n").unwrap();
    fs::write(
        dir.join("pages").join("destination.md"),
        "- destination body\n",
    )
    .unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let indexed = |graph: &Graph| {
        graph.with_pages(|pages| pages.iter().any(|(entry, _)| entry.name == "source"))
    };
    assert!(indexed(&graph), "the source starts indexed");

    // An external writer lands between the merge's read of the destination and
    // its commit recheck: the write is refused and the merge rolls back.
    let dst = dir.join("pages").join("destination.md");
    EDITOR_COMMIT_BEFORE_RECHECK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            fs::write(&dst, "- destination body\n- typed elsewhere\n")
        }));
    });
    let error = graph
        .merge_pages("pages/source.md", "pages/destination.md")
        .unwrap_err();
    assert!(
        dir.join("pages").join("source.md").exists(),
        "the rollback must restore the source file: {error}"
    );
    assert!(
        indexed(&graph),
        "the failed merge left the index missing a page that is on disk: {error}"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// A merge rolls back by moving its source file out of the trash and back —
/// and that move is no-clobber, so it can refuse. Publishing the source's
/// retained bytes regardless describes a file that is no longer at that path:
/// search and cached reads then serve content that belongs to nothing on disk
/// (GH #543, sixth audit A6-N5). The bytes verified before the write prove
/// what was STAGED, not who owns the path afterwards.
#[test]
fn a_failed_merge_that_cannot_restore_its_source_publishes_nothing() {
    let dir = scratch("merge-rollback-cannot-restore");
    fs::write(dir.join("pages").join("source.md"), "- pangolin source\n").unwrap();
    fs::write(
        dir.join("pages").join("destination.md"),
        "- destination body\n",
    )
    .unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let cached = |graph: &Graph, token: &str| {
        graph.with_pages(|pages| {
            pages
                .iter()
                .any(|(_, doc)| doc::serialize(doc).contains(token))
        })
    };
    assert!(cached(&graph, "pangolin"), "the source starts indexed");

    // The destination changes under the merge, so its write is refused.
    let dst = dir.join("pages").join("destination.md");
    EDITOR_COMMIT_BEFORE_RECHECK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            fs::write(&dst, "- destination body\n- typed elsewhere\n")
        }));
    });
    // And something else takes the source path before the rollback can put the
    // file back: the no-clobber restore refuses, and that path now belongs to a
    // file this merge has never read.
    let src = dir.join("pages").join("source.md");
    GRAPH_TEXT_WRITE_DURING_ROLLBACK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || fs::write(&src, "- okapi occupant\n")));
    });

    let error = graph
        .merge_pages("pages/source.md", "pages/destination.md")
        .unwrap_err();
    assert_eq!(
        fs::read_to_string(dir.join("pages").join("source.md")).unwrap(),
        "- okapi occupant\n",
        "the fixture must leave the occupant owning the source path: {error}"
    );
    assert!(
        !cached(&graph, "pangolin"),
        "a failed merge published bytes it did not restore, so the index now \
         describes a file that is not there: {error}"
    );
    // And the file that DOES own that path is described (seventh audit A7-N1).
    // Publishing nothing was safer than publishing the wrong bytes, but it is
    // not right: the occupant is the honest current content of a live path, and
    // nothing else is queued to notice it.
    assert!(
        cached(&graph, "okapi occupant"),
        "the file that now owns the source path reached neither the cache nor \
         the index, and the failed merge queued nothing that would: {error}"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// The journal filename migration reads the current generation when it
/// publishes, but parses its replacement earlier. Without the graph identity
/// gate every other page-set mutation takes, an ordinary save can land in
/// between — and the migration then stamps that save's generation onto the
/// bytes it parsed before it, and its batch replaces the newer rows
/// (GH #543, sixth audit A6-N4).
#[test]
fn a_journal_filename_migration_serializes_against_other_writers() {
    let dir = scratch("journal-migration-identity-gate");
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq").join("config.edn"),
        "{:journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
    )
    .unwrap();
    fs::write(
        dir.join("journals").join("Friday, 26-06-2026.md"),
        "- okapi only here\n",
    )
    .unwrap();
    let graph = Arc::new(Graph::open(&dir));
    graph.warm_cache();

    // Observe the gate from the FIRST graph-text admission of the migration —
    // before it has moved anything. A prober thread asks for the gate there; if
    // the migration is holding it, the prober registers as a waiter (the gate's
    // test instrumentation counts them), and if it is not, the prober simply
    // takes the gate and no waiter ever appears.
    let probe_graph = Arc::clone(&graph);
    let probe: Arc<std::sync::Mutex<Option<std::thread::JoinHandle<()>>>> =
        Arc::new(std::sync::Mutex::new(None));
    let probe_slot = Arc::clone(&probe);
    GRAPH_TEXT_WRITE_AFTER_ADMISSION.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            let blocked = Arc::clone(&probe_graph);
            *probe_slot.lock().unwrap() = Some(std::thread::spawn(move || {
                let _gate = blocked.lock_graph_text_identity_mutation().unwrap();
            }));
            let gate = &probe_graph
                .graph_text_write_binding()
                .expect("test graph has graph writer binding")
                .gate;
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut state = gate.identity_mutation.lock().unwrap();
            while state.waiters == 0 {
                let remaining = deadline.checked_duration_since(Instant::now()).expect(
                    "the migration reached its first graph-text write without holding \
                             the graph identity gate, so an ordinary save can land between its \
                             parse and its publication and be replaced by the older bytes",
                );
                let (next, timeout) = gate
                    .identity_mutation_changed
                    .wait_timeout(state, remaining)
                    .unwrap();
                state = next;
                assert!(
                    !timeout.timed_out(),
                    "the migration reached its first graph-text write without holding the \
                     graph identity gate, so an ordinary save can land between its parse and \
                     its publication and be replaced by the older bytes"
                );
            }
            Ok(())
        }));
    });

    assert_eq!(graph.migrate_journal_filenames_checked().unwrap(), 1);
    probe
        .lock()
        .unwrap()
        .take()
        .expect("the migration never reached a graph-text write")
        .join()
        .unwrap();
    assert!(dir.join("journals").join("2026_06_26.md").exists());
    let _ = fs::remove_dir_all(&dir);
}

#[path = "model_tests_availability.rs"]
mod availability;

#[path = "model_tests_journal_lookup.rs"]
mod journal_lookup;

#[path = "model_tests_advanced_queries.rs"]
mod advanced_queries;

#[path = "model_tests_gh543_parse_passes.rs"]
mod gh543_parse_passes;
