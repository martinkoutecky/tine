use super::*;
use std::time::{Duration, Instant};

// DUP mapping semantics (2026-08-25 duplication audit): both tree walkers
// delegate every block to the one shared field mapping, so identical DTO
// input must produce identical parseable trees (identity, format flag,
// fresh lazy projection included).
#[test]
fn blockdto_walkers_agree_on_the_shared_field_mapping() {
    let leaf = BlockDto {
        id: "u-leaf".into(),
        raw: "TODO leaf\nkey:: value".into(),
        ..Default::default()
    };
    let root = BlockDto {
        id: "u-root".into(),
        raw: "DONE [#B] root".into(),
        children: vec![leaf],
        ..Default::default()
    };

    fn shape(
        blocks: &[DocBlock],
    ) -> Vec<(String, String, bool, Vec<(String, String, bool, usize)>)> {
        blocks
            .iter()
            .map(|b| {
                (
                    b.raw.clone(),
                    b.uuid.clone(),
                    b.is_org,
                    b.children
                        .iter()
                        .map(|c| (c.raw.clone(), c.uuid.clone(), c.is_org, c.children.len()))
                        .collect(),
                )
            })
            .collect()
    }

    for is_org in [false, true] {
        let via_query_walk = crate::query::application_query_doc_block(&root, is_org);
        let via_checked_walk = dto_blocks_to_doc_checked(std::slice::from_ref(&root), is_org)
            .unwrap()
            .remove(0);
        assert_eq!(
            shape(std::slice::from_ref(&via_query_walk)),
            shape(std::slice::from_ref(&via_checked_walk)),
            "is_org={is_org}: every walker must run the same per-block mapping"
        );
        // The mapping hands out a fresh projection memo: the first access
        // parses `raw` (facets visible), it is not inherited from anywhere.
        assert_eq!(
            via_checked_walk.projection().marker.as_deref(),
            Some("DONE")
        );
        assert_eq!(
            via_checked_walk.children[0].projection().marker.as_deref(),
            Some("TODO")
        );
        if !is_org {
            // Org takes properties from a :PROPERTIES: drawer, not `key::`.
            assert_eq!(
                via_checked_walk.children[0].projection().properties,
                vec![("key".to_string(), "value".to_string())]
            );
        }
        assert_eq!(via_checked_walk.projection().priority.as_deref(), Some("B"));
    }
}

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
// the old substring check. Managed DTO conversions use
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
            "crates/tine-core/src/model.rs",
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
        "I-12: every production DocBlock literal is reviewed; managed DTO mappings must use dto_block_to_doc_block and raw-only leaves must use DocBlock::new"
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

#[cfg(any(unix, windows))]
fn handoff_binding(graph: &Graph, seed: u128) -> (WorkspaceId, ProjectionEndpointBinding) {
    let workspace_id = WorkspaceId::from_uuid(Uuid::from_u128(seed));
    let endpoint = ProjectionEndpointBinding::enroll_graph(
        graph,
        crate::oplog::ProjectionEndpointId::from_uuid(Uuid::from_u128(seed + 1)),
        crate::oplog::DeviceId::from_uuid(Uuid::from_u128(seed + 2)),
    )
    .unwrap();
    (workspace_id, endpoint)
}

#[cfg(any(unix, windows))]
fn managed_write_gate(graph: &Graph) -> &Arc<ManagedTextWriteGate> {
    &graph.managed_write_binding().unwrap().gate
}

#[cfg(any(unix, windows))]
fn hold_replacement_handoff(root: &Path, seed: u128) -> io::Result<()> {
    let replacement = Graph::open(root);
    let (workspace_id, endpoint) = handoff_binding(&replacement, seed);
    let handoff = replacement.mint_handoff_safe(workspace_id, endpoint)?;
    MANAGED_WRITE_REPLACEMENT_HANDOFF.with(|held| {
        *held.borrow_mut() = Some(handoff);
    });
    Ok(())
}

#[cfg(any(unix, windows))]
fn release_replacement_handoff() {
    MANAGED_WRITE_REPLACEMENT_HANDOFF.with(|held| drop(held.borrow_mut().take()));
}

#[cfg(any(unix, windows))]
fn assert_handoff_blocked<T>(result: io::Result<T>) {
    match result {
        Ok(_) => panic!("managed text writer entered during external handoff"),
        Err(error) => assert_eq!(error.kind(), io::ErrorKind::WouldBlock),
    }
}

#[cfg(any(unix, windows))]
fn assert_handoff_release_admits_waiting_writer(
    graph: Arc<Graph>,
    handoff: HandoffSafe,
    label: &str,
    release: impl FnOnce(HandoffSafe),
) {
    let entered = Arc::new(std::sync::Barrier::new(2));
    let released = Arc::new(std::sync::Barrier::new(2));
    let (attempt_tx, attempt_rx) = std::sync::mpsc::channel();
    let before_name = format!("{label}-blocked");
    let after_name = format!("{label}-released");
    let worker = std::thread::spawn({
        let entered = Arc::clone(&entered);
        let released = Arc::clone(&released);
        move || {
            entered.wait();
            attempt_tx
                .send(
                    graph
                        .create_markdown_page_if_absent(&before_name, "- blocked\n")
                        .map(|_| ())
                        .map_err(|error| error.kind()),
                )
                .unwrap();
            released.wait();
            graph.create_markdown_page_if_absent(&after_name, "- admitted\n")
        }
    });

    entered.wait();
    assert_eq!(attempt_rx.recv().unwrap(), Err(io::ErrorKind::WouldBlock));
    release(handoff);
    released.wait();
    assert!(worker.join().unwrap().unwrap());
}

struct JournalProjectionFixture {
    root: PathBuf,
    graph: Graph,
    path: PathBuf,
    base: String,
    base_rev: String,
    page: PageDto,
    target: String,
    cleanup_root: bool,
}

impl JournalProjectionFixture {
    fn new(tag: &str, relative_path: &str, base: &str, edited: &str) -> Self {
        let root = scratch(tag);
        Self::from_root(root, relative_path, base, edited)
    }

    fn from_root(root: PathBuf, relative_path: &str, base: &str, edited: &str) -> Self {
        let path = root.join(relative_path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, base).unwrap();
        let graph = Graph::open(&root);
        let mut page = graph.load_by_path(relative_path).unwrap().unwrap();
        graph.warm_cache();
        let base_rev = page.rev.clone().unwrap();
        page.blocks[0].raw = edited.to_owned();
        let (_, target) = graph
            .serialize_page_dto_for_path(&page, &path, Some(base))
            .unwrap();
        Self {
            root,
            graph,
            path,
            base: base.to_owned(),
            base_rev,
            page,
            target,
            cleanup_root: true,
        }
    }

    fn commit<A>(
        &self,
        append: impl FnOnce() -> io::Result<A>,
    ) -> Result<JournalPageProjectionOutcome<A>, JournalPageCommitError<io::Error>> {
        self.commit_with_error(append)
    }

    fn commit_with_error<A, E>(
        &self,
        append: impl FnOnce() -> Result<A, E>,
    ) -> Result<JournalPageProjectionOutcome<A>, JournalPageCommitError<E>> {
        self.graph.commit_existing_page_with_journal(
            &self.page,
            &self.base_rev,
            self.base.as_bytes(),
            self.target.as_bytes(),
            append,
        )
    }

    fn close_for_restart(mut self) {
        self.cleanup_root = false;
    }
}

impl Drop for JournalProjectionFixture {
    fn drop(&mut self) {
        if self.cleanup_root {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

#[cfg(any(unix, windows))]
fn prime_journal_projection_restart_graph(graph: &Graph) {
    graph.warm_cache();
    let _write = graph.admit_managed_text_writer().unwrap();
    let _identity = graph.lock_graph_text_identity_mutation().unwrap();
    graph.guarded_graph_text_identity_index().unwrap();
}

#[cfg(any(unix, windows))]
struct JournalProjectionRestartTestRecord {
    root: PathBuf,
    path: PathBuf,
    relative_path: String,
    base: String,
    base_revision: String,
    target: String,
    revision: String,
    proof: (String, u64),
    append_calls: Rc<Cell<usize>>,
}

#[cfg(any(unix, windows))]
impl Drop for JournalProjectionRestartTestRecord {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[cfg(any(unix, windows))]
fn journal_projection_restart_record(
    tag: &str,
    relative_path: &str,
    base: &str,
    edited: &str,
) -> JournalProjectionRestartTestRecord {
    let fixture = JournalProjectionFixture::new(tag, relative_path, base, edited);
    JOURNAL_PROJECTION_BEFORE_PUBLISH.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(|| {
            Err(injected_journal_projection_cut(
                "after append before publication",
            ))
        }));
    });
    let proof = (format!("authenticated:{relative_path}"), 73_u64);
    let append_calls = Rc::new(Cell::new(0_usize));
    let outcome = fixture
        .commit(|| {
            append_calls.set(append_calls.get() + 1);
            Ok(proof.clone())
        })
        .unwrap();
    let JournalPageProjectionOutcome::CommittedPending(pending) = outcome else {
        panic!("synthetic append-before-publication cut was not retained")
    };
    assert_eq!(append_calls.get(), 1);
    assert_eq!(pending.append_proof(), &proof);
    assert_eq!(pending.target(), fixture.target.as_bytes());
    assert_eq!(fs::read(&fixture.path).unwrap(), fixture.base.as_bytes());
    drop(pending);

    let record = JournalProjectionRestartTestRecord {
        root: fixture.root.clone(),
        path: fixture.path.clone(),
        relative_path: relative_path.to_owned(),
        base: fixture.base.clone(),
        base_revision: fixture.base_rev.clone(),
        target: fixture.target.clone(),
        revision: content_rev(&fixture.target),
        proof,
        append_calls,
    };
    fixture.close_for_restart();
    record
}

fn injected_journal_projection_cut(label: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, label)
}

#[cfg(any(unix, windows))]
#[test]
fn journal_projection_exact_base_appends_once_then_publishes_markdown_and_org() {
    for (tag, path, base, edited) in [
        (
            "journal-projection-success-md",
            "pages/nested/Exact.md",
            "- base markdown\n",
            "target markdown",
        ),
        (
            "journal-projection-success-org",
            "journals/archive/2026_08_03.org",
            "* base org\n",
            "target org",
        ),
    ] {
        let fixture = JournalProjectionFixture::new(tag, path, base, edited);
        let calls = Cell::new(0_usize);
        let outcome = fixture
            .commit(|| {
                assert_eq!(fs::read(&fixture.path).unwrap(), fixture.base.as_bytes());
                calls.set(calls.get() + 1);
                Ok(format!("proof:{path}"))
            })
            .unwrap();
        let JournalPageProjectionOutcome::Durable(durable) = outcome else {
            panic!("successful journal projection remained pending")
        };
        assert_eq!(calls.get(), 1);
        assert_eq!(durable.append_proof(), &format!("proof:{path}"));
        assert_eq!(durable.target().relative_path(), path);
        assert_eq!(durable.target().target(), fixture.target.as_bytes());
        assert_eq!(durable.target().revision(), content_rev(&fixture.target));
        assert_eq!(fs::read(&fixture.path).unwrap(), fixture.target.as_bytes());
    }
}

#[cfg(any(unix, windows))]
#[test]
fn journal_projection_accepts_only_semantically_equal_markdown_layout_trivia() {
    let fixture = JournalProjectionFixture::new(
        "journal-projection-authenticated-layout",
        "pages/Authenticated Layout.md",
        "- first\n\n- second\n",
        "edited",
    );
    // The serializer preserves the fixture's one blank separator. Supply
    // a genuinely byte-distinct authenticated layout with one additional
    // separator while keeping every parsed block and its raw body equal.
    let exact_target = "- edited\n\n\n- second\n";
    assert_ne!(fixture.target, exact_target);
    assert!(guarded_markdown_documents_match(
        &fixture.target,
        exact_target
    ));
    let calls = Cell::new(0_usize);
    let outcome = fixture
        .graph
        .commit_existing_page_with_journal(
            &fixture.page,
            &fixture.base_rev,
            fixture.base.as_bytes(),
            exact_target.as_bytes(),
            || {
                calls.set(calls.get() + 1);
                Ok::<_, ()>("authenticated-layout-proof")
            },
        )
        .unwrap();
    assert!(matches!(outcome, JournalPageProjectionOutcome::Durable(_)));
    assert_eq!(calls.get(), 1);
    assert_eq!(fs::read(&fixture.path).unwrap(), exact_target.as_bytes());
}

#[cfg(any(unix, windows))]
#[test]
fn journal_projection_refuses_semantically_different_markdown_layout_targets_before_append() {
    fn assert_refused(label: &str, mut fixture: JournalProjectionFixture, exact_target: &str) {
        let before = regular_file_tree(&fixture.root);
        let calls = Cell::new(0_usize);
        let error = fixture
            .graph
            .commit_existing_page_with_journal(
                &fixture.page,
                &fixture.base_rev,
                fixture.base.as_bytes(),
                exact_target.as_bytes(),
                || {
                    calls.set(calls.get() + 1);
                    Ok::<(), ()>(())
                },
            )
            .err()
            .unwrap_or_else(|| panic!("{label} semantic mismatch committed"));
        let precommit = error
            .precommit()
            .unwrap_or_else(|| panic!("{label} semantic mismatch was not precommit"));
        assert_eq!(precommit.kind(), io::ErrorKind::InvalidData, "{label}");
        assert_eq!(calls.get(), 0, "{label}");
        assert_eq!(regular_file_tree(&fixture.root), before, "{label}");
        fixture.cleanup_root = false;
        let _ = fs::remove_dir_all(&fixture.root);
    }

    assert_refused(
        "content",
        JournalProjectionFixture::new(
            "journal-projection-mismatch-content",
            "pages/Content.md",
            "- first\n\n- second\n",
            "edited",
        ),
        "- different\n\n- second\n",
    );
    assert_refused(
        "order-and-ancestry",
        JournalProjectionFixture::new(
            "journal-projection-mismatch-order",
            "pages/Order.md",
            "- first\n  - child\n- second\n",
            "edited",
        ),
        "- second\n\n- edited\n  - child\n",
    );
    assert_refused(
        "page-property",
        JournalProjectionFixture::new(
            "journal-projection-mismatch-property",
            "pages/Property.md",
            "status:: accepted\n\n- first\n",
            "edited",
        ),
        "status:: changed\n\n- edited\n",
    );

    let original_id = "11111111-1111-1111-1111-111111111111";
    let replacement_id = "22222222-2222-2222-2222-222222222222";
    let base = format!("- first\n  id:: {original_id}\n");
    let fixture = JournalProjectionFixture::new(
        "journal-projection-mismatch-explicit-id",
        "pages/Explicit Id.md",
        &base,
        &format!("edited\nid:: {original_id}"),
    );
    let exact_target = format!("- edited\n  id:: {replacement_id}\n");
    assert_refused("explicit-id", fixture, &exact_target);
}

#[cfg(any(unix, windows))]
#[test]
fn journal_projection_keeps_org_byte_exact_when_layout_only_target_differs() {
    let fixture = JournalProjectionFixture::new(
        "journal-projection-org-layout",
        "journals/2026_08_05.org",
        "* first\n* second\n",
        "edited",
    );
    let exact_target = "* edited\n\n* second\n";
    let calls = Cell::new(0_usize);
    let error = fixture
        .graph
        .commit_existing_page_with_journal(
            &fixture.page,
            &fixture.base_rev,
            fixture.base.as_bytes(),
            exact_target.as_bytes(),
            || {
                calls.set(calls.get() + 1);
                Ok::<(), ()>(())
            },
        )
        .err()
        .expect("Org layout-only difference must remain refused before append");
    assert_eq!(
        error.precommit().map(io::Error::kind),
        Some(io::ErrorKind::InvalidData)
    );
    assert_eq!(calls.get(), 0);
    assert_eq!(fs::read(&fixture.path).unwrap(), fixture.base.as_bytes());
}

#[cfg(any(unix, windows))]
#[test]
fn journal_projection_precommit_conflicts_append_zero_and_change_zero_bytes() {
    fn refused(label: &str, fixture: &JournalProjectionFixture, base_rev: &str) {
        let before = regular_file_tree(&fixture.root);
        let calls = Cell::new(0_usize);
        let error = fixture
            .graph
            .commit_existing_page_with_journal(
                &fixture.page,
                base_rev,
                fixture.base.as_bytes(),
                fixture.target.as_bytes(),
                || {
                    calls.set(calls.get() + 1);
                    Ok::<(), ()>(())
                },
            )
            .err()
            .unwrap_or_else(|| panic!("{label} precommit conflict produced an outcome"));
        let precommit = error
            .precommit()
            .unwrap_or_else(|| panic!("{label} error was not precommit"));
        assert!(matches!(
            precommit.kind(),
            io::ErrorKind::AlreadyExists | io::ErrorKind::InvalidInput
        ));
        assert_eq!(calls.get(), 0);
        assert_eq!(regular_file_tree(&fixture.root), before);
    }

    let stale = JournalProjectionFixture::new(
        "journal-projection-stale",
        "pages/Stale.md",
        "- base\n",
        "target",
    );
    refused("stale", &stale, &"0".repeat(64));

    let resource = JournalProjectionFixture::new(
        "journal-projection-resource",
        "pages/Resource.md",
        "- base\n",
        "target",
    );
    fs::rename(
        &resource.path,
        resource.root.join("pages/.Resource.retained-old"),
    )
    .unwrap();
    fs::write(&resource.path, resource.base.as_bytes()).unwrap();
    refused("resource", &resource, &resource.base_rev);

    let portable = JournalProjectionFixture::new(
        "journal-projection-portable",
        "pages/Portable.md",
        "- base\n",
        "target",
    );
    fs::write(portable.root.join("pages/portable.md"), "- alias\n").unwrap();
    refused("portable", &portable, &portable.base_rev);

    #[cfg(unix)]
    {
        let hardlink = JournalProjectionFixture::new(
            "journal-projection-hardlink",
            "pages/Hardlink.md",
            "- base\n",
            "target",
        );
        fs::hard_link(&hardlink.path, hardlink.root.join("pages/Alias.md")).unwrap();
        refused("hardlink", &hardlink, &hardlink.base_rev);
    }

    let semantic = JournalProjectionFixture::new(
        "journal-projection-semantic",
        "pages/One.md",
        "title:: Shared\n\n- base\n",
        "target",
    );
    fs::create_dir_all(semantic.root.join("other")).unwrap();
    fs::write(
        semantic.root.join("other/Two.org"),
        "#+title: Shared\n* other owner\n",
    )
    .unwrap();
    semantic.graph.invalidate_cache();
    semantic.graph.warm_cache();
    {
        let _identity = semantic.graph.lock_graph_text_identity_mutation().unwrap();
        let index = semantic.graph.guarded_graph_text_identity_index().unwrap();
        assert_eq!(
            index
                .paths_by_semantic_key
                .get(&(0, crate::refs::page_key("Shared")))
                .map(std::collections::BTreeSet::len),
            Some(2),
            "page name {:?}, semantics {:?}",
            semantic.page.name,
            index
                .files_by_exact_path
                .iter()
                .map(|(path, record)| (
                    path.as_str(),
                    record.semantic.name.as_str(),
                    record.semantic.kind
                ))
                .collect::<Vec<_>>()
        );
    }
    refused("semantic", &semantic, &semantic.base_rev);

    #[cfg(unix)]
    {
        let parent = JournalProjectionFixture::new(
            "journal-projection-parent",
            "pages/Parent.md",
            "- base\n",
            "target",
        );
        {
            let _identity = parent.graph.lock_graph_text_identity_mutation().unwrap();
            parent.graph.guarded_graph_text_identity_index().unwrap();
        }
        let old_parent = parent.root.join("pages-old");
        fs::rename(parent.root.join("pages"), &old_parent).unwrap();
        fs::create_dir(parent.root.join("pages")).unwrap();
        fs::hard_link(old_parent.join("Parent.md"), &parent.path).unwrap();
        refused("parent", &parent, &parent.base_rev);
    }
}

#[cfg(any(unix, windows))]
#[test]
fn journal_projection_append_error_preserves_non_io_type_and_changes_no_graph_bytes_or_cache() {
    #[derive(Debug, Eq, PartialEq)]
    enum AppendSentinel {
        DurableAppendRejected,
    }

    let fixture = JournalProjectionFixture::new(
        "journal-projection-append-error",
        "pages/Append.md",
        "- base\n",
        "target",
    );
    let before = regular_file_tree(&fixture.root);
    let cache_before = fixture.graph.cache.read().unwrap().clone();
    let calls = Cell::new(0_usize);
    let error = fixture
        .commit_with_error(|| {
            calls.set(calls.get() + 1);
            Err::<(), _>(AppendSentinel::DurableAppendRejected)
        })
        .err()
        .expect("append error must cross the graph boundary");
    assert_eq!(error.append(), Some(&AppendSentinel::DurableAppendRejected));
    assert!(matches!(
        error,
        JournalPageCommitError::Append(AppendSentinel::DurableAppendRejected)
    ));
    assert_eq!(calls.get(), 1);
    assert_eq!(regular_file_tree(&fixture.root), before);
    assert_exact_budget_cache_unchanged(&fixture.graph, &cache_before);
}

#[cfg(any(unix, windows))]
#[test]
fn journal_projection_failure_cuts_are_committed_pending_and_retry_without_append() {
    for cut in 0..5 {
        let fixture = JournalProjectionFixture::new(
            &format!("journal-projection-cut-{cut}"),
            "pages/Cut.md",
            "- base\n",
            "target",
        );
        match cut {
            0 => MANAGED_WRITE_AFTER_RETIRE.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(|| {
                    Err(injected_journal_projection_cut("after retire"))
                }));
            }),
            1 => JOURNAL_PROJECTION_AFTER_PUBLISH.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(|| {
                    Err(injected_journal_projection_cut("after publish"))
                }));
            }),
            2 => JOURNAL_PROJECTION_AFTER_TARGET_REREAD.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(|| {
                    Err(injected_journal_projection_cut("after target reread"))
                }));
            }),
            3 => FAIL_NEXT_PROJECTION_DIRECTORY_SYNC.with(|fail| fail.set(true)),
            4 => JOURNAL_PROJECTION_BEFORE_CACHE_PUBLICATION.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(|| {
                    Err(injected_journal_projection_cut("before cache publication"))
                }));
            }),
            _ => unreachable!(),
        }
        let calls = Cell::new(0_usize);
        let outcome = fixture
            .commit(|| {
                calls.set(calls.get() + 1);
                Ok((cut, "opaque-proof"))
            })
            .unwrap();
        let JournalPageProjectionOutcome::CommittedPending(pending) = outcome else {
            panic!("cut {cut} did not report committed-pending")
        };
        assert_eq!(calls.get(), 1);
        assert_eq!(pending.append_proof(), &(cut, "opaque-proof"));
        assert_eq!(pending.relative_path(), "pages/Cut.md");
        assert_eq!(pending.target(), fixture.target.as_bytes());
        assert!(!pending.last_error().to_string().is_empty());

        let retried = fixture
            .graph
            .retry_committed_journal_page_projection(pending);
        let JournalPageProjectionOutcome::Durable(durable) = retried else {
            panic!("cut {cut} did not recover on exact retry")
        };
        assert_eq!(calls.get(), 1, "retry must not append again");
        assert_eq!(durable.append_proof(), &(cut, "opaque-proof"));
        assert_eq!(durable.target().target(), fixture.target.as_bytes());
        assert_eq!(fs::read(&fixture.path).unwrap(), fixture.target.as_bytes());
    }
}

#[cfg(any(unix, windows))]
#[test]
fn journal_projection_restart_reconstructs_record_only_and_publishes_markdown_and_org() {
    for (tag, path, base, edited) in [
        (
            "journal-projection-restart-md",
            "notes/nonstandard/deep/Restart.markdown",
            "- base markdown\n",
            "restart markdown target",
        ),
        (
            "journal-projection-restart-org",
            "archive/nonstandard/deep/Restart.org",
            "* base org\n",
            "restart org target",
        ),
    ] {
        let record = journal_projection_restart_record(tag, path, base, edited);
        let graph = Graph::open(&record.root);
        prime_journal_projection_restart_graph(&graph);
        let graph_work_before = crate::fast_commit::graph_wide_commit_work();
        let forbidden_before = crate::fast_commit::forbidden_commit_work();
        let mutations = Rc::new(Cell::new(0_usize));
        MANAGED_WRITE_BEFORE_MUTATION.with(|hook| {
            let mutations = Rc::clone(&mutations);
            *hook.borrow_mut() = Some(Box::new(move || {
                mutations.set(mutations.get() + 1);
                Ok(())
            }));
        });

        let outcome = graph.recover_committed_journal_page_projection(
            record.proof.clone(),
            &record.relative_path,
            &record.base_revision,
            record.base.as_bytes(),
            record.target.as_bytes(),
            &record.revision,
        );
        let JournalPageProjectionOutcome::Durable(durable) = outcome else {
            panic!("restart recovery did not publish {path}")
        };
        assert_eq!(durable.append_proof(), &record.proof);
        assert_eq!(durable.target().relative_path(), path);
        assert_eq!(durable.target().target(), record.target.as_bytes());
        assert_eq!(durable.target().revision(), record.revision);
        assert_eq!(fs::read(&record.path).unwrap(), record.target.as_bytes());
        assert_eq!(mutations.get(), 1);

        let graph_work = crate::fast_commit::graph_wide_commit_work().since(graph_work_before);
        assert_eq!(graph_work.text_inventory_scans, 0);
        assert_eq!(graph_work.text_inventory_entries, 0);
        assert!(crate::fast_commit::forbidden_commit_work()
            .since(forbidden_before)
            .is_none());
    }
}

#[cfg(any(unix, windows))]
#[test]
fn journal_projection_restart_already_target_reproves_durable_state() {
    let record = journal_projection_restart_record(
        "journal-projection-restart-already-target",
        "elsewhere/nested/Already.md",
        "- base\n",
        "already target",
    );
    fs::write(&record.path, record.target.as_bytes()).unwrap();
    let graph = Graph::open(&record.root);
    prime_journal_projection_restart_graph(&graph);
    let graph_work_before = crate::fast_commit::graph_wide_commit_work();
    let forbidden_before = crate::fast_commit::forbidden_commit_work();
    let mutations = Rc::new(Cell::new(0_usize));
    MANAGED_WRITE_BEFORE_MUTATION.with(|hook| {
        let mutations = Rc::clone(&mutations);
        *hook.borrow_mut() = Some(Box::new(move || {
            mutations.set(mutations.get() + 1);
            Ok(())
        }));
    });

    let outcome = graph.recover_committed_journal_page_projection(
        record.proof.clone(),
        &record.relative_path,
        &record.base_revision,
        record.base.as_bytes(),
        record.target.as_bytes(),
        &record.revision,
    );
    let JournalPageProjectionOutcome::Durable(durable) = outcome else {
        panic!("already exact restart target was not reproved")
    };
    assert_eq!(durable.append_proof(), &record.proof);
    assert_eq!(durable.target().target(), record.target.as_bytes());
    assert_eq!(fs::read(&record.path).unwrap(), record.target.as_bytes());
    assert_eq!(
        mutations.get(),
        0,
        "already-target recovery rewrote the file"
    );
    MANAGED_WRITE_BEFORE_MUTATION.with(|hook| drop(hook.borrow_mut().take()));
    let graph_work = crate::fast_commit::graph_wide_commit_work().since(graph_work_before);
    assert_eq!(graph_work.text_inventory_scans, 0);
    assert_eq!(graph_work.text_inventory_entries, 0);
    assert!(crate::fast_commit::forbidden_commit_work()
        .since(forbidden_before)
        .is_none());
}

#[cfg(any(unix, windows))]
#[test]
fn journal_projection_restart_preserves_divergent_external_winner() {
    let record = journal_projection_restart_record(
        "journal-projection-restart-divergent",
        "elsewhere/nested/Divergent.org",
        "* base\n",
        "restart target",
    );
    let external = b"* external winner\n";
    fs::write(&record.path, external).unwrap();
    let graph = Graph::open(&record.root);
    prime_journal_projection_restart_graph(&graph);
    let graph_work_before = crate::fast_commit::graph_wide_commit_work();
    let forbidden_before = crate::fast_commit::forbidden_commit_work();
    let mutations = Rc::new(Cell::new(0_usize));
    MANAGED_WRITE_BEFORE_MUTATION.with(|hook| {
        let mutations = Rc::clone(&mutations);
        *hook.borrow_mut() = Some(Box::new(move || {
            mutations.set(mutations.get() + 1);
            Ok(())
        }));
    });

    let outcome = graph.recover_committed_journal_page_projection(
        record.proof.clone(),
        &record.relative_path,
        &record.base_revision,
        record.base.as_bytes(),
        record.target.as_bytes(),
        &record.revision,
    );
    let JournalPageProjectionOutcome::CommittedPending(pending) = outcome else {
        panic!("divergent external winner was silently replaced")
    };
    assert_eq!(pending.append_proof(), &record.proof);
    assert_eq!(pending.relative_path(), record.relative_path);
    assert_eq!(pending.target(), record.target.as_bytes());
    assert_eq!(pending.last_error().kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(&record.path).unwrap(), external);
    assert!(matches!(
        graph.retry_committed_journal_page_projection(pending),
        JournalPageProjectionOutcome::CommittedPending(_)
    ));
    assert_eq!(fs::read(&record.path).unwrap(), external);
    assert_eq!(mutations.get(), 0, "divergent winner was rewritten");
    MANAGED_WRITE_BEFORE_MUTATION.with(|hook| drop(hook.borrow_mut().take()));
    let graph_work = crate::fast_commit::graph_wide_commit_work().since(graph_work_before);
    assert_eq!(graph_work.text_inventory_scans, 0);
    assert_eq!(graph_work.text_inventory_entries, 0);
    assert!(crate::fast_commit::forbidden_commit_work()
        .since(forbidden_before)
        .is_none());
}

#[cfg(any(unix, windows))]
#[test]
fn journal_projection_external_writer_is_precommit_winner_or_preserved_after_commit() {
    let before = JournalProjectionFixture::new(
        "journal-projection-external-before",
        "pages/Race.md",
        "- base\n",
        "target",
    );
    fs::write(&before.path, "- external before\n").unwrap();
    let calls = Cell::new(0_usize);
    assert!(before
        .commit(|| {
            calls.set(calls.get() + 1);
            Ok(())
        })
        .is_err());
    assert_eq!(calls.get(), 0);
    assert_eq!(fs::read(&before.path).unwrap(), b"- external before\n");

    let after = JournalProjectionFixture::new(
        "journal-projection-external-after",
        "pages/Race.md",
        "- base\n",
        "target",
    );
    MANAGED_WRITE_BEFORE_MUTATION.with(|hook| {
        let path = after.path.clone();
        *hook.borrow_mut() = Some(Box::new(move || fs::write(path, "- external after\n")));
    });
    let calls = Cell::new(0_usize);
    let outcome = after
        .commit(|| {
            calls.set(calls.get() + 1);
            Ok("journal-won")
        })
        .unwrap();
    let JournalPageProjectionOutcome::CommittedPending(pending) = outcome else {
        panic!("post-append external winner must remain committed-pending")
    };
    assert_eq!(calls.get(), 1);
    assert_eq!(pending.append_proof(), &"journal-won");
    assert_eq!(fs::read(&after.path).unwrap(), b"- external after\n");
    assert!(matches!(
        after.graph.retry_committed_journal_page_projection(pending),
        JournalPageProjectionOutcome::CommittedPending(_)
    ));
    assert_eq!(fs::read(&after.path).unwrap(), b"- external after\n");
}

#[cfg(any(unix, windows))]
#[test]
fn journal_projection_nested_configured_paths_work_and_private_paths_are_refused() {
    for (tag, path, base, edited) in [
        (
            "journal-projection-nested-config-md",
            "content/wiki/deep/Topic.md",
            "- base md\n",
            "target md",
        ),
        (
            "journal-projection-nested-config-org",
            "diary/archive/2026_08_03.org",
            "* base org\n",
            "target org",
        ),
    ] {
        let root = scratch(tag);
        fs::create_dir_all(root.join("logseq")).unwrap();
        fs::write(
            root.join("logseq/config.edn"),
            "{:pages-directory \"content/wiki/deep\"\n :journals-directory \"diary/archive\"}\n",
        )
        .unwrap();
        let fixture = JournalProjectionFixture::from_root(root, path, base, edited);
        let outcome = fixture.commit(|| Ok("proof")).unwrap();
        assert!(matches!(outcome, JournalPageProjectionOutcome::Durable(_)));
        assert_eq!(fs::read(&fixture.path).unwrap(), fixture.target.as_bytes());
    }

    let root = scratch("journal-projection-private-refusal");
    let path = root.join(".private/Secret.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "- secret\n").unwrap();
    let graph = Graph::open(&root);
    let mut page = markdown_page_dto("Secret", "Secret", "- changed\n").unwrap();
    page.path = ".private/Secret.md".to_owned();
    page.rev = Some(content_rev("- secret\n"));
    let calls = Cell::new(0_usize);
    assert!(graph
        .commit_existing_page_with_journal(
            &page,
            page.rev.as_deref().unwrap(),
            b"- secret\n",
            b"- changed\n",
            || {
                calls.set(calls.get() + 1);
                Ok::<(), ()>(())
            },
        )
        .is_err());
    assert_eq!(calls.get(), 0);
    assert_eq!(fs::read(&path).unwrap(), b"- secret\n");
    let restart = graph.recover_committed_journal_page_projection(
        "authenticated-private-record",
        ".private/Secret.md",
        &content_rev("- secret\n"),
        b"- secret\n",
        b"- changed\n",
        &content_rev("- changed\n"),
    );
    let JournalPageProjectionOutcome::CommittedPending(pending) = restart else {
        panic!("restart recovery admitted a private graph path")
    };
    assert_eq!(pending.append_proof(), &"authenticated-private-record");
    assert_eq!(pending.target(), b"- changed\n");
    assert_eq!(pending.last_error().kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(fs::read(&path).unwrap(), b"- secret\n");
    let _ = fs::remove_dir_all(root);
}

#[cfg(any(unix, windows))]
#[test]
fn journal_projection_warm_repeats_have_zero_inventory_and_forbidden_work() {
    let mut fixture = JournalProjectionFixture::new(
        "journal-projection-warm-work",
        "pages/Warm.md",
        "- base\n",
        "target 0",
    );
    let first = fixture.commit(|| Ok(0_u64)).unwrap();
    let JournalPageProjectionOutcome::Durable(first) = first else {
        panic!("warmup commit remained pending")
    };
    let mut base = fixture.target.clone();
    let mut revision = first.target().revision().to_owned();
    fixture.page.rev = Some(revision.clone());
    let graph_work_before = crate::fast_commit::graph_wide_commit_work();
    let forbidden_before = crate::fast_commit::forbidden_commit_work();
    for index in 1..=8_u64 {
        fixture.page.blocks[0].raw = format!("target {index}");
        let (_, target) = fixture
            .graph
            .serialize_page_dto_for_path(&fixture.page, &fixture.path, Some(&base))
            .unwrap();
        let outcome = fixture
            .graph
            .commit_existing_page_with_journal(
                &fixture.page,
                &revision,
                base.as_bytes(),
                target.as_bytes(),
                || Ok::<_, ()>(index),
            )
            .unwrap();
        let JournalPageProjectionOutcome::Durable(durable) = outcome else {
            panic!("warm commit {index} remained pending")
        };
        assert_eq!(durable.append_proof(), &index);
        base = target;
        revision = durable.target().revision().to_owned();
        fixture.page.rev = Some(revision.clone());
    }
    let graph_work = crate::fast_commit::graph_wide_commit_work().since(graph_work_before);
    assert_eq!(graph_work.text_inventory_scans, 0);
    assert_eq!(graph_work.text_inventory_entries, 0);
    assert!(crate::fast_commit::forbidden_commit_work()
        .since(forbidden_before)
        .is_none());
    assert_eq!(fs::read(&fixture.path).unwrap(), base.as_bytes());
}

#[cfg(any(unix, windows))]
#[test]
fn journal_projection_source_keeps_append_and_retry_authority_structural() {
    let fixture = JournalProjectionFixture::new(
        "journal-projection-append-authority-retry",
        "pages/Authority.md",
        "- base\n",
        "target",
    );
    prime_journal_projection_restart_graph(&fixture.graph);
    JOURNAL_PROJECTION_BEFORE_PUBLISH.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(|| {
            Err(injected_journal_projection_cut(
                "after append before publication",
            ))
        }));
    });
    let append_calls = Cell::new(0_usize);
    let graph_work_before = crate::fast_commit::graph_wide_commit_work();
    let forbidden_before = crate::fast_commit::forbidden_commit_work();
    let outcome = fixture
        .commit(|| {
            append_calls.set(append_calls.get() + 1);
            Ok("opaque-retry-proof")
        })
        .unwrap();
    let JournalPageProjectionOutcome::CommittedPending(pending) = outcome else {
        panic!("append-before-publication cut was not retained for retry")
    };
    assert_eq!(append_calls.get(), 1);
    let retried = fixture
        .graph
        .retry_committed_journal_page_projection(pending);
    let JournalPageProjectionOutcome::Durable(durable) = retried else {
        panic!("retry did not publish the authenticated record")
    };
    assert_eq!(
        append_calls.get(),
        1,
        "retry appended a second journal record"
    );
    assert_eq!(durable.append_proof(), &"opaque-retry-proof");
    assert_eq!(fs::read(&fixture.path).unwrap(), fixture.target.as_bytes());
    assert_eq!(
        crate::fast_commit::graph_wide_commit_work().since(graph_work_before),
        crate::fast_commit::GraphWideCommitWork::default()
    );
    assert!(crate::fast_commit::forbidden_commit_work()
        .since(forbidden_before)
        .is_none());

    let record = journal_projection_restart_record(
        "journal-projection-append-authority-restart",
        "pages/Restart Authority.md",
        "- base\n",
        "target",
    );
    let graph = Graph::open(&record.root);
    prime_journal_projection_restart_graph(&graph);
    let graph_work_before = crate::fast_commit::graph_wide_commit_work();
    let forbidden_before = crate::fast_commit::forbidden_commit_work();
    let outcome = graph.recover_committed_journal_page_projection(
        record.proof.clone(),
        &record.relative_path,
        &record.base_revision,
        record.base.as_bytes(),
        record.target.as_bytes(),
        &record.revision,
    );
    let JournalPageProjectionOutcome::Durable(durable) = outcome else {
        panic!("restart did not publish the authenticated record")
    };
    assert_eq!(
        record.append_calls.get(),
        1,
        "restart appended a second journal record"
    );
    assert_eq!(durable.append_proof(), &record.proof);
    assert_eq!(fs::read(&record.path).unwrap(), record.target.as_bytes());
    assert_eq!(
        crate::fast_commit::graph_wide_commit_work().since(graph_work_before),
        crate::fast_commit::GraphWideCommitWork::default()
    );
    assert!(crate::fast_commit::forbidden_commit_work()
        .since(forbidden_before)
        .is_none());
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
fn direct_managed_and_sparse_new_page_paths_share_the_safe_codec() {
    for (label, config, format) in [
        ("legacy", "", FileNameFormat::Legacy),
        (
            "triple-lowbar",
            "{:file/name-format :triple-lowbar}\n",
            FileNameFormat::TripleLowbar,
        ),
    ] {
        let dir = scratch(&format!("filename-paths-{label}"));
        if !config.is_empty() {
            fs::create_dir_all(dir.join("logseq")).unwrap();
            fs::write(dir.join("logseq/config.edn"), config).unwrap();
        }
        let graph = Graph::open(&dir);
        let permit = graph.admit_managed_text_writer().unwrap();
        for title in ["2026-07-23_18:01:20", "%2F", "CON", "Release 1.0"] {
            let stem = encode_page_name(title, format);
            let relative = format!("pages/{stem}.md");
            assert_eq!(graph.path_for(title, PageKind::Page), dir.join(&relative));
            assert_eq!(
                graph
                    .managed_path_for(&permit, title, PageKind::Page)
                    .unwrap(),
                dir.join(&relative)
            );
            assert_eq!(
                graph
                    .new_sparse_page_path(title, PageKind::Page)
                    .unwrap()
                    .as_str(),
                relative
            );
        }
        let _ = fs::remove_dir_all(dir);
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
    assert!(
        has("lp survey", "paper"),
        "alias:: query should navigate to its owning page"
    );
    assert!(
        has("paper notes", "paper"),
        "aliases:: query should navigate to its owning page"
    );
    assert!(
        !has("private", "Private"),
        "quoted custom value stays literal"
    );
    // Neither filed nor referenced → not offered (so autocomplete still says
    // "Create" for a genuinely new name).
    assert!(!has("nonexistent", "nonexistent"));
    let _ = fs::remove_dir_all(&dir);
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tine-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("journals")).unwrap();
    fs::create_dir_all(dir.join("pages")).unwrap();
    dir
}

/// The artifact-class split, proved at the primitive itself: both barriers
/// call the same syscall on the same directory, and only the reconstructible
/// projection class degrades when Android refuses it.
#[cfg(unix)]
#[test]
fn only_the_reconstructible_projection_barrier_degrades_on_android() {
    use crate::filesystem_durability::DurabilityArtifactClass;

    let dir = scratch("projection-barrier-artifact-class");
    let chain = vec![Dir::open_ambient_dir(&dir, ambient_authority()).unwrap()];

    {
        let _refusal = InjectedProjectionDirectoryBarrierFailure::enter(
            DurabilityArtifactClass::SharedReconstructibleProjection,
            libc::EINVAL,
            true,
            &dir,
        )
        .unwrap();
        sync_reconstructible_projection_chain(&chain)
            .expect("Android cannot provide this barrier for reconstructible bytes");
        preflight_reconstructible_projection_chain(&chain).unwrap();
    }

    {
        // The graph tree also holds artifacts it is the SOLE authority for —
        // conflict copies, trash, withdrawn bytes. Those keep the strict
        // barrier on every platform, Android included.
        let _refusal = InjectedProjectionDirectoryBarrierFailure::enter(
            DurabilityArtifactClass::PrivateDurableAuthority,
            libc::EINVAL,
            true,
            &dir,
        )
        .unwrap();
        let error = sync_projection_chain_required(&chain).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(
            error
                .to_string()
                .contains("fsync of the projection parent directory"),
            "a bare errno is not diagnosable from a device receipt: {error}"
        );
        assert_eq!(
            preflight_projection_chain(&chain).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }

    {
        // Off Android the reconstructible barrier is strict too.
        let _refusal = InjectedProjectionDirectoryBarrierFailure::enter(
            DurabilityArtifactClass::SharedReconstructibleProjection,
            libc::EINVAL,
            false,
            &dir,
        )
        .unwrap();
        assert_eq!(
            sync_reconstructible_projection_chain(&chain)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }

    {
        // A real I/O failure is not a capability refusal and stays fatal
        // even on Android.
        let _refusal = InjectedProjectionDirectoryBarrierFailure::enter(
            DurabilityArtifactClass::SharedReconstructibleProjection,
            libc::EIO,
            true,
            &dir,
        )
        .unwrap();
        let error = sync_reconstructible_projection_chain(&chain).unwrap_err();
        assert!(
            error.to_string().contains("Input/output error"),
            "a real I/O failure must stay fatal even on Android: {error}"
        );
    }

    let _ = fs::remove_dir_all(&dir);
}

/// The same split at the *rename* primitive, which is what Android CI run
/// 32091898520 actually caught: `renameat2(RENAME_NOREPLACE) publishing the
/// projection failed at "Smoke.md" -> ".Smoke.md.49a4ed18…"` with `EINVAL`.
/// The reconstructible projection publishes through an exclusive
/// reservation instead; the sole-authority class keeps the atomic primitive
/// and fails.
#[cfg(unix)]
#[test]
fn only_the_reconstructible_projection_rename_falls_back_when_the_flag_is_unsupported() {
    use crate::filesystem_durability::DurabilityArtifactClass;

    let dir = scratch("projection-noreplace-artifact-class");
    let capability = Dir::open_ambient_dir(&dir, ambient_authority()).unwrap();

    for errno in [libc::EINVAL, libc::ENOSYS, libc::EOPNOTSUPP, libc::ENOTSUP] {
        fs::write(dir.join("source"), b"staged projection bytes").unwrap();
        let staged =
            canonical_projection_file_resource_id(&fs::File::open(dir.join("source")).unwrap())
                .unwrap();

        let _refusal = InjectedProjectionNoreplaceRenameFailure::enter(
            DurabilityArtifactClass::SharedReconstructibleProjection,
            errno,
            &dir,
        )
        .unwrap();
        rename_reconstructible_projection_noreplace(&capability, "source", "destination")
            .expect("a filesystem without rename2 flags must still publish");

        assert!(!dir.join("source").exists());
        assert_eq!(
            fs::read(dir.join("destination")).unwrap(),
            b"staged projection bytes"
        );
        // The published name must be the staged INODE, not a copy of its
        // bytes into the reservation placeholder.
        assert_eq!(
            canonical_projection_file_resource_id(
                &fs::File::open(dir.join("destination")).unwrap()
            )
            .unwrap(),
            staged,
            "the fallback must publish the exact staged inode ({errno})"
        );
        fs::remove_file(dir.join("destination")).unwrap();
    }

    {
        // The sole-authority class never degrades: no second copy exists to
        // rebuild these bytes from, so the atomic primitive is the contract.
        fs::write(dir.join("source"), b"sole authority bytes").unwrap();
        let _refusal = InjectedProjectionNoreplaceRenameFailure::enter(
            DurabilityArtifactClass::PrivateDurableAuthority,
            libc::EINVAL,
            &dir,
        )
        .unwrap();
        let error = rename_projection_noreplace(&capability, "source", "destination").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(
            error
                .to_string()
                .contains("renameat2(RENAME_NOREPLACE) publishing the projection"),
            "the enriched detail is what made this diagnosable: {error}"
        );
        assert!(!dir.join("destination").exists());
        assert_eq!(
            fs::read(dir.join("source")).unwrap(),
            b"sole authority bytes"
        );
        fs::remove_file(dir.join("source")).unwrap();
    }

    {
        // A real I/O failure is not a capability answer. It stays fatal for
        // the reconstructible class too, and nothing is reserved.
        fs::write(dir.join("source"), b"unmoved bytes").unwrap();
        let _refusal = InjectedProjectionNoreplaceRenameFailure::enter(
            DurabilityArtifactClass::SharedReconstructibleProjection,
            libc::EIO,
            &dir,
        )
        .unwrap();
        let error =
            rename_reconstructible_projection_noreplace(&capability, "source", "destination")
                .unwrap_err();
        assert!(
            error.to_string().contains("Input/output error"),
            "a real I/O failure must stay fatal: {error}"
        );
        assert!(
            !dir.join("destination").exists(),
            "a fatal errno must not reserve the destination name"
        );
        assert_eq!(fs::read(dir.join("source")).unwrap(), b"unmoved bytes");
        fs::remove_file(dir.join("source")).unwrap();
    }

    let _ = fs::remove_dir_all(&dir);
}

/// The artifact classification must cover the whole guarded replacement,
/// not only standalone projection publication. Android refused the first
/// target-retirement rename in this transaction before the staged bytes
/// could be published.
#[cfg(unix)]
#[test]
fn managed_projection_replacement_falls_back_but_direct_files_stays_strict() {
    use crate::filesystem_durability::DurabilityArtifactClass;

    let dir = scratch("managed-replacement-artifact-class");
    let path = dir.join("pages/Target.md");
    fs::write(&path, b"- original\n").unwrap();
    let graph = Graph::open(&dir);
    let write = graph.admit_managed_text_writer().unwrap();
    let identity = canonical_projection_file_resource_id(&fs::File::open(&path).unwrap()).unwrap();

    {
        let _refusal = InjectedProjectionNoreplaceRenameFailure::enter(
            DurabilityArtifactClass::SharedReconstructibleProjection,
            libc::EINVAL,
            &dir.join("pages"),
        )
        .unwrap();
        graph
            .managed_atomic_replace_bound(
                &write,
                &path,
                b"- managed replacement\n",
                identity,
                Some(b"- original\n"),
                None,
                EditorPublicationAuthority::ReconstructibleManagedProjection,
                Some([0x12, 0x34, 0x56, 0x78]),
            )
            .expect("a reconstructible managed projection must use the capability fallback");
    }
    assert_eq!(fs::read(&path).unwrap(), b"- managed replacement\n");

    let managed_identity =
        canonical_projection_file_resource_id(&fs::File::open(&path).unwrap()).unwrap();
    {
        let _refusal = InjectedProjectionDirectoryBarrierFailure::enter(
            DurabilityArtifactClass::SharedReconstructibleProjection,
            libc::EINVAL,
            true,
            &dir.join("pages"),
        )
        .unwrap();
        graph
            .managed_atomic_replace_bound(
                &write,
                &path,
                b"- managed barrier replacement\n",
                managed_identity,
                Some(b"- managed replacement\n"),
                None,
                EditorPublicationAuthority::ReconstructibleManagedProjection,
                Some([0x12, 0x34, 0x56, 0x78]),
            )
            .expect("a reconstructible managed projection may degrade an Android barrier");
    }
    assert_eq!(fs::read(&path).unwrap(), b"- managed barrier replacement\n");

    let managed_identity =
        canonical_projection_file_resource_id(&fs::File::open(&path).unwrap()).unwrap();
    {
        let _refusal = InjectedProjectionNoreplaceRenameFailure::enter(
            DurabilityArtifactClass::PrivateDurableAuthority,
            libc::EINVAL,
            &dir.join("pages"),
        )
        .unwrap();
        let error = graph
            .managed_atomic_replace_bound(
                &write,
                &path,
                b"- direct replacement\n",
                managed_identity,
                Some(b"- managed barrier replacement\n"),
                None,
                EditorPublicationAuthority::DirectFile,
                None,
            )
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{error}");
    }
    assert_eq!(
        fs::read(&path).unwrap(),
        b"- managed barrier replacement\n",
        "Direct Files must not weaken the sole-authority publication contract"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn managed_barrier_collapse_does_not_change_direct_files_retire_publish_barriers() {
    use crate::durability_counters::{Barrier, BarrierSession};

    let dir = scratch("direct-retire-publish-barrier-guard");
    let path = dir.join("pages/Target.md");
    fs::write(&path, b"- original\n").unwrap();
    let graph = Graph::open(&dir);
    let write = graph.admit_managed_text_writer().unwrap();

    let measure = |before: &[u8], after: &[u8], authority, turn| {
        let identity =
            canonical_projection_file_resource_id(&fs::File::open(&path).unwrap()).unwrap();
        let session = BarrierSession::begin();
        graph
            .managed_atomic_replace_bound(
                &write,
                &path,
                after,
                identity,
                Some(before),
                None,
                authority,
                turn,
            )
            .unwrap();
        let directories = session.counts().get(Barrier::Directory);
        BarrierSession::detach_current_thread();
        directories
    };

    let managed = measure(
        b"- original\n",
        b"- managed\n",
        EditorPublicationAuthority::ReconstructibleManagedProjection,
        Some([0x12, 0x34, 0x56, 0x78]),
    );
    let direct = measure(
        b"- managed\n",
        b"- direct\n",
        EditorPublicationAuthority::DirectFile,
        None,
    );

    assert_eq!(managed, 1, "managed W1 closes with one leaf barrier");
    assert_eq!(
            direct, 4,
            "Direct Files must retain strict preflight, retire, publish, and recovery-name removal barriers"
        );
    assert_eq!(fs::read(&path).unwrap(), b"- direct\n");
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn android_shared_storage_permission_refusal_is_only_a_flagged_rename_capability_answer() {
    let permission = io::Error::from_raw_os_error(libc::EACCES);
    assert!(reconstructible_flagged_rename_capability_refusal(
        &permission,
        true
    ));
    assert!(
        !reconstructible_flagged_rename_capability_refusal(&permission, false),
        "desktop EACCES remains a real permission failure"
    );

    let io_failure = io::Error::from_raw_os_error(libc::EIO);
    assert!(
        !reconstructible_flagged_rename_capability_refusal(&io_failure, true),
        "Android may degrade only the shared-filesystem capability refusal"
    );
}

/// The guarantee the flag was there to provide, kept by the fallback: an
/// occupied destination is refused, never overwritten, and it is refused
/// with the same `AlreadyExists` the flagged rename raises so every guarded
/// conflict caller above is unchanged.
#[cfg(unix)]
#[test]
fn the_projection_rename_fallback_refuses_an_occupied_destination_rather_than_clobbering_it() {
    use crate::filesystem_durability::DurabilityArtifactClass;

    let dir = scratch("projection-noreplace-fallback-occupied");
    fs::write(dir.join("source"), b"staged projection bytes").unwrap();
    fs::write(dir.join("destination"), b"live bytes that must survive").unwrap();
    let live =
        canonical_projection_file_resource_id(&fs::File::open(dir.join("destination")).unwrap())
            .unwrap();
    let capability = Dir::open_ambient_dir(&dir, ambient_authority()).unwrap();

    let _refusal = InjectedProjectionNoreplaceRenameFailure::enter(
        DurabilityArtifactClass::SharedReconstructibleProjection,
        libc::EINVAL,
        &dir,
    )
    .unwrap();
    let error = rename_reconstructible_projection_noreplace(&capability, "source", "destination")
        .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{error}");
    assert_eq!(
        fs::read(dir.join("destination")).unwrap(),
        b"live bytes that must survive"
    );
    assert_eq!(
        canonical_projection_file_resource_id(&fs::File::open(dir.join("destination")).unwrap())
            .unwrap(),
        live,
        "the occupied destination must keep its exact inode"
    );
    assert_eq!(
        fs::read(dir.join("source")).unwrap(),
        b"staged projection bytes"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// A reservation that cannot be completed is rolled back. Otherwise a failed
/// publication would leave a zero-length file at a live page name — which is
/// worse than the refusal it replaced.
#[cfg(unix)]
#[test]
fn a_projection_rename_fallback_that_cannot_complete_leaves_no_empty_destination() {
    use crate::filesystem_durability::DurabilityArtifactClass;

    let dir = scratch("projection-noreplace-fallback-rollback");
    let capability = Dir::open_ambient_dir(&dir, ambient_authority()).unwrap();

    let _refusal = InjectedProjectionNoreplaceRenameFailure::enter(
        DurabilityArtifactClass::SharedReconstructibleProjection,
        libc::EINVAL,
        &dir,
    )
    .unwrap();
    // The source never existed, so the plain rename fails after the
    // destination has already been reserved.
    let error = rename_reconstructible_projection_noreplace(&capability, "absent", "Live Page.md")
        .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::NotFound, "{error}");
    assert!(
        !dir.join("Live Page.md").exists(),
        "a failed publication must not leave the reservation behind"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn consumed_external_document_dto_matches_exact_parse_across_formats_and_identity() {
    let dir = scratch("consumed-exact-page-dto");
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq/config.edn"),
        br#"{:journal/file-name-format "dd-MM-yyyy"
            :journal/page-title-format "yyyy-MM-dd"}"#,
    )
    .unwrap();
    let graph = Graph::open(&dir);
    let cases = [
            (
                "markdown",
                "pages/Plain.md",
                "- ordinary markdown\n",
                "Plain",
                PageKind::Page,
                false,
            ),
            (
                "markdown-read-only",
                "pages/Read Only.md",
                "- root\r  ```\r  - fake\r  ```",
                "Read Only",
                PageKind::Page,
                true,
            ),
            (
                "org-editable-properties",
                "pages/Editable Org.org",
                "#+TITLE: Editable Org\n\n* TODO Buy milk\nSCHEDULED: <2026-06-25 Thu>\n:PROPERTIES:\n:id: 6679-abc\n:END:\n",
                "Editable Org",
                PageKind::Page,
                false,
            ),
            (
                "org-read-only",
                "pages/Read Only.org",
                "* root\n*** child\n",
                "Read Only",
                PageKind::Page,
                true,
            ),
            (
                "explicit-title",
                "pages/Physical title.md",
                "title:: Explicit title\n\n- titled body\n",
                "Explicit title",
                PageKind::Page,
                false,
            ),
            (
                "journal-title",
                "pages/Physical journal.md",
                "title:: 25-07-2026\n\n- journal body\n",
                "2026-07-25",
                PageKind::Journal,
                false,
            ),
        ];

    for (label, relative, source, expected_name, expected_kind, expected_read_only) in cases {
        let path = ManagedPath::parse(relative.to_owned()).unwrap();
        let consumed = graph
            .parse_external_document(&path, source.as_bytes(), false)
            .unwrap()
            .into_exact_page_dto(&path, source)
            .unwrap();
        let exact = graph
            .parse_exact_page_dto(&path, source.as_bytes())
            .unwrap();
        assert_eq!(
            serde_json::to_value(&consumed).unwrap(),
            serde_json::to_value(&exact).unwrap(),
            "{label}: consumed parser result must preserve exact DTO semantics"
        );
        assert_eq!(consumed.name, expected_name, "{label}");
        assert_eq!(consumed.kind, expected_kind, "{label}");
        assert_eq!(consumed.read_only, expected_read_only, "{label}");
        if label == "org-editable-properties" {
            assert_eq!(consumed.format, Format::Org);
            assert_eq!(consumed.path, relative);
            assert!(consumed.rev.is_some());
            assert_eq!(
                consumed.pre_block.as_deref(),
                Some("#+TITLE: Editable Org\n")
            );
            assert!(!consumed.blocks[0].id.is_empty());
            assert!(consumed.blocks[0]
                .properties
                .iter()
                .any(|(key, value)| key.eq_ignore_ascii_case("id") && value == "6679-abc"));
        }
    }

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn managed_projection_trash_publication_walks_the_typed_nofollow_chain() {
    let root = scratch("managed-projection-trash-capability");
    let relative = "pages/Typed Recovery.org";
    let expected = b"#+TITLE: Typed Recovery\r\n\r\n* exact bytes\r\n";
    fs::write(root.join(relative), expected).unwrap();
    let graph = Graph::open(&root);
    let path = ManagedPath::parse(relative.to_owned()).unwrap();

    assert!(
        !root.join("logseq/.tine-trash/journals").exists(),
        "the recovery chain begins absent"
    );
    let destination = graph
        .preserve_projection_in_trash(&path, ManagedTextKind::Journal, expected)
        .unwrap();
    assert_eq!(
        destination.parent(),
        Some(root.join("logseq/.tine-trash/journals").as_path())
    );
    assert_eq!(
        destination
            .extension()
            .and_then(|extension| extension.to_str()),
        Some("org")
    );
    assert_eq!(fs::read(&destination).unwrap(), expected);

    // Exact retries retain one immutable recovery leaf, while a divergent
    // occupant is a refusal rather than an overwrite.
    assert_eq!(
        graph
            .preserve_projection_in_trash(&path, ManagedTextKind::Journal, expected)
            .unwrap(),
        destination
    );
    fs::write(&destination, b"foreign recovery bytes").unwrap();
    assert_eq!(
        graph
            .preserve_projection_in_trash(&path, ManagedTextKind::Journal, expected)
            .unwrap_err()
            .kind(),
        io::ErrorKind::AlreadyExists
    );

    // A malformed intermediate component is rejected through the same
    // no-follow chain, before it can authorize a semantic deletion.
    let malformed_root = scratch("managed-projection-trash-malformed-chain");
    fs::write(malformed_root.join(relative), expected).unwrap();
    fs::create_dir_all(malformed_root.join("logseq")).unwrap();
    fs::write(
        malformed_root.join("logseq/.tine-trash"),
        b"not a recovery directory",
    )
    .unwrap();
    let malformed = Graph::open(&malformed_root);
    let error = malformed
        .preserve_projection_in_trash(&path, ManagedTextKind::Journal, expected)
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(fs::read(malformed_root.join(relative)).unwrap(), expected);

    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&malformed_root);
}

#[cfg(unix)]
#[test]
fn managed_projection_trash_refuses_a_missing_typed_kind_that_cannot_be_created() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = scratch("managed-projection-trash-kind-create-refusal");
    let relative = "pages/Kind Creation Refusal.md";
    let expected = b"- source survives an unavailable recovery kind\n";
    fs::write(root.join(relative), expected).unwrap();
    let trash_root = root.join("logseq/.tine-trash");
    fs::create_dir_all(&trash_root).unwrap();
    let original_permissions = fs::metadata(&trash_root).unwrap().permissions();
    let mut no_create_permissions = original_permissions.clone();
    no_create_permissions.set_mode(0o555);
    fs::set_permissions(&trash_root, no_create_permissions).unwrap();

    let graph = Graph::open(&root);
    let path = ManagedPath::parse(relative.to_owned()).unwrap();
    let result = graph.preserve_projection_in_trash(&path, ManagedTextKind::Page, expected);
    fs::set_permissions(&trash_root, original_permissions).unwrap();

    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(fs::read(root.join(relative)).unwrap(), expected);
    assert!(
        !trash_root.join("pages").exists(),
        "a failed typed-kind creation must not leave a partial namespace"
    );
    let _ = fs::remove_dir_all(&root);
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

/// The read-only view exists so managed storage can answer whole-graph
/// questions from its own projected tree. It must answer them -- and it must
/// not be able to write that tree back, because the oplog owns it.
#[test]
fn a_derived_read_only_graph_reads_the_tree_but_cannot_write_it() {
    let dir = scratch("derived-read-only-graph");
    fs::write(
        dir.join("pages/Alpha.md"),
        "- alpha mentions [[Target]]\n- and again [[Target]]\n",
    )
    .unwrap();
    fs::write(dir.join("pages/Target.md"), "- the target page\n").unwrap();

    let view = Graph::open_derived_read_only(&dir);

    // Reads: the whole point. Backlinks are the query that was dead in
    // managed mode, so assert that one specifically.
    let groups = view.backlinks("Target");
    assert_eq!(
        groups
            .iter()
            .map(|group| group.page.as_str())
            .collect::<Vec<_>>(),
        vec!["Alpha"],
        "the read-only view must resolve backlinks from the projected tree"
    );
    assert!(
        view.list_pages().iter().any(|entry| entry.name == "Alpha"),
        "the read-only view must enumerate pages"
    );

    // Writes: refused at the single graph-text admission, whatever the
    // caller. A command routed here by mistake fails loudly rather than
    // leaving a file behind the oplog's back.
    let mut page = view.load_by_path("pages/Alpha.md").unwrap().unwrap();
    let base_rev = page.rev.clone();
    page.blocks[0].raw = "- alpha edited behind the oplog".into();
    let refused = view.save_page(&page, base_rev.as_deref()).unwrap_err();
    assert_eq!(
        refused.kind(),
        io::ErrorKind::PermissionDenied,
        "a graph-text write through the read-only view must be refused: {refused}"
    );
    assert_eq!(
        fs::read_to_string(dir.join("pages/Alpha.md")).unwrap(),
        "- alpha mentions [[Target]]\n- and again [[Target]]\n",
        "the refused save must not have touched the file"
    );

    // Control: the same directory opened normally still saves, so the test
    // proves the flag and not some unrelated breakage in the fixture.
    let writable = Graph::open(&dir);
    let mut page = writable.load_by_path("pages/Alpha.md").unwrap().unwrap();
    let base_rev = page.rev.clone();
    page.blocks[0].raw = "- alpha edited by the owner".into();
    writable
        .save_page(&page, base_rev.as_deref())
        .expect("an ordinary graph still writes");

    let _ = fs::remove_dir_all(&dir);
}

/// `assets/` is outside the oplog's document domain, so importing an image
/// while managed storage owns graph text must keep working.
#[test]
fn a_derived_read_only_graph_still_accepts_asset_writes() {
    let dir = scratch("derived-read-only-assets");
    let source = dir.join("incoming.png");
    fs::write(&source, b"\x89PNG\r\n\x1a\n").unwrap();

    let view = Graph::open_derived_read_only(&dir);
    let stored = view
        .import_asset(&source, Some("picture.png"))
        .expect("asset writes are outside the graph-text boundary");
    assert!(
        dir.join("assets").join(&stored).exists(),
        "the imported asset must land in assets/"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// Every Settings toggle writes `logseq/config.edn`, and configuration is
/// **not** oplog-owned: the managed scanner classifies it
/// `GraphTextScanPathClass::Configuration` and the baseline adapter drops
/// those rows as "not managed content", so no managed path, import or
/// projection ever covers it. Persisting a setting must therefore keep
/// working while managed storage owns graph text.
#[test]
fn a_derived_read_only_graph_still_writes_graph_configuration() {
    let dir = scratch("derived-read-only-config");
    let view = Graph::open_derived_read_only(&dir);

    view.set_favorites(&["Alpha".to_owned(), "Beta".to_owned()])
        .expect("configuration is outside the graph-text boundary");
    view.set_start_of_week(3)
        .expect("configuration is outside the graph-text boundary");

    let written = fs::read_to_string(dir.join("logseq/config.edn"))
        .expect("the setting must have been persisted");
    assert!(
        written.contains(":favorites [\"Alpha\" \"Beta\"]"),
        "favorites must round-trip into config.edn: {written}"
    );
    assert!(
        written.contains(":start-of-week 3"),
        "start of week must round-trip into config.edn: {written}"
    );

    // The same view still cannot touch graph text, so the config capability
    // did not widen into the oplog's domain.
    fs::write(dir.join("pages/Alpha.md"), "- alpha\n").unwrap();
    let view = Graph::open_derived_read_only(&dir);
    let mut page = view.load_by_path("pages/Alpha.md").unwrap().unwrap();
    let base_rev = page.rev.clone();
    page.blocks[0].raw = "- alpha edited behind the oplog".into();
    assert_eq!(
        view.save_page(&page, base_rev.as_deref())
            .unwrap_err()
            .kind(),
        io::ErrorKind::PermissionDenied,
        "a config write must not have widened the graph-text boundary"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// `logseq/.tine-trash` sits next to `assets`, `publish` and `.tine-sync` in
/// `graph_text_scope::fixed_excluded`, so nothing under it is scanned,
/// imported or projected. Trashing an orphaned asset is an asset-side write
/// into that tree and must stay available under managed storage; trashing a
/// recognized sync-conflict copy is likewise outside graph discovery and can
/// be discarded into that tree; trashing a journal file is a graph-text
/// deletion and must not.
#[test]
fn a_derived_read_only_graph_trashes_assets_but_not_journals() {
    let dir = scratch("derived-read-only-trash");
    fs::create_dir_all(dir.join("assets")).unwrap();
    fs::write(dir.join("assets/orphan.png"), b"\x89PNG\r\n\x1a\n").unwrap();
    fs::write(dir.join("journals/2026_08_07.md"), "- a journal day\n").unwrap();
    let conflict = "Alpha.sync-conflict-20260810-120000-DEVICE.md";
    fs::write(dir.join("pages").join(conflict), "- conflict evidence\n").unwrap();

    let view = Graph::open_derived_read_only(&dir);

    view.trash_asset("orphan.png")
        .expect("the trash tree is outside the graph-text boundary");
    assert!(
        !dir.join("assets/orphan.png").exists(),
        "the orphaned asset must have left assets/"
    );
    assert_eq!(
        view.asset_trash_stats().count,
        1,
        "the orphaned asset must be recoverable from the trash"
    );

    let removed = view
        .empty_asset_trash()
        .expect("emptying the asset trash is an asset-side write");
    assert_eq!(removed, 1, "the emptied entry must be counted");

    let conflict_path = format!("pages/{conflict}");
    let staged = view
        .stage_sync_conflict_trash(&conflict_path, b"- conflict evidence\n")
        .expect("excluded conflict evidence can be staged recoverably");
    assert!(!dir.join("pages").join(conflict).exists());
    view.rollback_sync_conflict_trash(&staged)
        .expect("a definitely unauthored operation can restore its staged evidence");
    assert_eq!(
        fs::read_to_string(dir.join("pages").join(conflict)).unwrap(),
        "- conflict evidence\n"
    );
    view.trash_sync_conflict(&conflict_path)
        .expect("a conflict copy is excluded from the graph-text domain");
    assert!(!dir.join("pages").join(conflict).exists());
    assert_eq!(view.asset_trash_stats().conflicts, 1);

    // A journal file is graph text. Its deletion belongs to the oplog and
    // stays refused at the single graph-text admission.
    let refused = view.trash_journal_file("2026_08_07.md").unwrap_err();
    assert_eq!(
        refused.kind(),
        io::ErrorKind::PermissionDenied,
        "a journal deletion must stay refused under managed storage: {refused}"
    );
    assert!(
        dir.join("journals/2026_08_07.md").exists(),
        "the refused journal deletion must not have touched the file"
    );

    let _ = fs::remove_dir_all(&dir);
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
        let parsed = parse_external_document(&graph, entry, content, true).unwrap();
        assert_eq!(parsed.source_round_trips, Some(true));
        assert_eq!(
            crate::outline::parse_attempts(),
            expected_attempts,
            "{relative}"
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

fn bootstrap_capture_scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "tine-bootstrap-capture-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
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

fn set_managed_content_budget_limit(limit: u64) {
    MANAGED_TEXT_INVENTORY_LIMITS_OVERRIDE.with(|override_limits| {
        *override_limits.borrow_mut() = Some(ManagedTextInventoryLimits {
            retained_content_bytes: limit,
            ..MANAGED_TEXT_INVENTORY_LIMITS
        });
    });
}

fn clear_managed_content_budget_limit() {
    MANAGED_TEXT_INVENTORY_LIMITS_OVERRIDE.with(|override_limits| {
        *override_limits.borrow_mut() = None;
    });
}

fn last_managed_content_budget_peak() -> u64 {
    MANAGED_TEXT_BUDGET_LAST_PEAK.with(Cell::get)
}

#[test]
fn publisher_p1_managed_text_classifier_uses_longest_component_root_and_preserves_exact_path() {
    let dir = scratch("managed-text-classifier-longest-root");
    let mut graph = Graph::open(&dir);
    graph.config.pages_dir = "managed/text".to_owned();
    graph.config.journals_dir = "managed/text/daily".to_owned();

    let nested = ManagedPath::parse("managed/text/daily/2026/07/naïve.md").unwrap();
    assert_eq!(
        graph.classify_managed_text_path(&nested),
        Ok(ManagedTextKind::Journal)
    );
    assert_eq!(nested.as_str(), "managed/text/daily/2026/07/naïve.md");
    assert_eq!(
        graph.classify_managed_text_path(
            &ManagedPath::parse("managed/text/projects/2026/roadmap.md").unwrap()
        ),
        Ok(ManagedTextKind::Page)
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn publisher_p1_managed_text_classifier_rejects_boundary_misses_outside_paths_and_equal_roots() {
    let dir = scratch("managed-text-classifier-rejections");
    let mut graph = Graph::open(&dir);
    graph.config.pages_dir = "pages".to_owned();
    graph.config.journals_dir = "pages-journal".to_owned();
    for path in ["pages-old/file.md", "outside/file.md"] {
        assert!(
            graph
                .classify_managed_text_path(&ManagedPath::parse(path).unwrap())
                .is_err(),
            "accepted {path}"
        );
    }
    assert_eq!(
        graph.classify_managed_text_path(&ManagedPath::parse("pages-journal/a.md").unwrap()),
        Ok(ManagedTextKind::Journal)
    );

    graph.config.journals_dir = "pages".to_owned();
    assert!(graph
        .classify_managed_text_path(&ManagedPath::parse("pages/a.md").unwrap())
        .is_err());

    for malformed_pages_root in ["bad*", "COM¹"] {
        graph.config.pages_dir = malformed_pages_root.to_owned();
        graph.config.journals_dir = "journals".to_owned();
        assert!(graph
            .classify_managed_text_path(&ManagedPath::parse("journals/2026/07/24.md").unwrap())
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

    let execution = g.run_graph_search(needle, 0, 8, false);
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
    assert!(g
        .run_graph_search(needle, 0, 8, false)
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
    MANAGED_WRITE_BEFORE_MUTATION.with(|hook| {
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
    let cached_revisions = graph.disk_revs.read().unwrap().clone();
    let cache_generation = graph.cache_gen.load(std::sync::atomic::Ordering::Acquire);
    page.blocks[0].raw = format!("user staged {branch} bytes");
    let staged_bytes = format!("- user staged {branch} bytes\n").into_bytes();

    let replacement = parent.join(".foreign-replacement");
    let foreign_bytes = format!("- foreign {branch} winner\n").into_bytes();
    fs::write(&replacement, &foreign_bytes).unwrap();
    let foreign_identity =
        canonical_projection_file_resource_id(&fs::File::open(&replacement).unwrap()).unwrap();

    if restoration_branch {
        MANAGED_WRITE_AFTER_RETIRE.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(|| {
                Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "injected post-retirement validation failure",
                ))
            }));
        });
        MANAGED_WRITE_BEFORE_RESTORE.with(|hook| {
            let path = path.clone();
            let replacement = replacement.clone();
            *hook.borrow_mut() = Some(Box::new(move || fs::rename(replacement, path)));
        });
    } else {
        MANAGED_WRITE_AFTER_RETIRE.with(|hook| {
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
    assert_eq!(
        *graph.disk_revs.read().unwrap(),
        cached_revisions,
        "{branch} failure must not advance cached disk revisions"
    );
    assert_eq!(
        graph.cache_gen.load(std::sync::atomic::Ordering::Acquire),
        cache_generation,
        "{branch} failure must not advance cache generation"
    );
    graph.with_pages(|pages| {
        let (_, document) = pages
            .iter()
            .find(|(entry, _)| entry.rel_path == "external/Exact.md")
            .unwrap();
        assert_eq!(document.roots[0].raw, "loaded baseline");
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
// which that refactor legitimately removed — but the link-count parity
// above it is not legacy and is still load-bearing: the Direct creation
// census refuses `file.link_count != 1`, and on Windows that count comes
// from `GetFileInformationByHandle` on a handle opened BEFORE the hard link
// existed. If a held handle reported a stale 1, the hard-link refusal would
// be defeated on Windows only. The v1 half is dropped; the platform
// assertion is not.
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
        "a held handle must observe the new link, or the creation census \
             cannot refuse a hard-linked graph text file on Windows"
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
    let permit = graph.admit_retained_managed_text_writer().unwrap();
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
        ManagedTextInventoryLimits {
            managed_files: 1,
            ..MANAGED_TEXT_INVENTORY_LIMITS
        },
        ManagedTextInventoryLimits {
            directory_depth: 1,
            ..MANAGED_TEXT_INVENTORY_LIMITS
        },
        ManagedTextInventoryLimits {
            all_entries: 1,
            ..MANAGED_TEXT_INVENTORY_LIMITS
        },
        ManagedTextInventoryLimits {
            directories: 1,
            ..MANAGED_TEXT_INVENTORY_LIMITS
        },
        ManagedTextInventoryLimits {
            pending_directories: 1,
            ..MANAGED_TEXT_INVENTORY_LIMITS
        },
        ManagedTextInventoryLimits {
            path_bytes: 1,
            ..MANAGED_TEXT_INVENTORY_LIMITS
        },
    ] {
        assert!(graph
            .text_entries_with_limits_and_budget(&permit, false, limits, None, vec![("", 0)], true,)
            .is_err());
    }
    drop(permit);
    MANAGED_TEXT_INVENTORY_LIMITS_OVERRIDE.with(|override_limits| {
        *override_limits.borrow_mut() = Some(ManagedTextInventoryLimits {
            retained_content_bytes: 1,
            ..MANAGED_TEXT_INVENTORY_LIMITS
        });
    });
    assert_eq!(
        graph
            .load_by_path("external/d00/P00.md")
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    MANAGED_TEXT_INVENTORY_LIMITS_OVERRIDE.with(|override_limits| {
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
    // sync_file's cold-cache branch (the parsed-doc cache not yet built), which
    // drops the page-list memo WITHOUT bumping cache_gen. Before the fix,
    // find_entry kept serving the pre-create index here (missing the new file)
    // until some other op happened to bump the generation.
    let dir = scratch("find-entry-cache-cold-sync");
    fs::write(dir.join("pages").join("Existing.md"), "- body\n").unwrap();
    let g = Graph::open(&dir);

    // Do NOT warm the doc cache: find_entry builds only its own index, so
    // self.cache stays cold and sync_file below takes the else-branch.
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
    let g = Graph::open(&dir);
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
        .hits
        .iter()
        .any(|hit| matches!(hit,
            crate::query_plan::QueryHit::Page { page, .. } if page.path == future
        )));
    assert!(!g.search("future-search-sentinel", 8).is_empty());
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
        .with(|charge| charge.set(Some(INITIAL_SHADOW_LIMITS.peak_build_bytes)));
    graph
        .rename_page("Alpha", "Beta")
        .expect("bounded rename must not enter retained-shadow construction");
    let after = graph.guarded_graph_text_identity_report();

    assert_eq!(after.complete_builds, before.complete_builds);
    assert_eq!(
        GRAPH_TEXT_FIRST_CAPTURE_CHARGE_OVERRIDE.with(Cell::take),
        Some(INITIAL_SHADOW_LIMITS.peak_build_bytes),
        "rename consumed the retained-shadow capture hook"
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
        // An explicit title remains the effective identity; the physical
        // move must not silently reinterpret it from the new filename.
        entry.name == "Original" && entry.rel_path == "pages/Renamed.md"
    }));
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
    assert!(
        GRAPH_TEXT_CONTENT_READS.with(Cell::get) >= 2,
        "a known watcher parse failure must force exact disk revalidation"
    );
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

    MANAGED_TEXT_INVENTORY_LIMITS_OVERRIDE.with(|override_limits| {
        *override_limits.borrow_mut() = Some(ManagedTextInventoryLimits {
            directories: 2,
            ..MANAGED_TEXT_INVENTORY_LIMITS
        });
    });
    let error = graph.save_page(&page, page.rev.as_deref()).unwrap_err();
    MANAGED_TEXT_INVENTORY_LIMITS_OVERRIDE.with(|override_limits| {
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
        .managed_write_binding()
        .expect("test graph has managed writer binding")
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
        let permit = first_graph.admit_retained_managed_text_writer()?;
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
        let permit = second_graph.admit_retained_managed_text_writer()?;
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
    let permit = graph.admit_retained_managed_text_writer().unwrap();

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
        let permit = graph.admit_retained_managed_text_writer().unwrap();
        let expected = graph.cache_generation();
        let built = graph.load_all_pages_with_permit(&permit);
        if drift {
            graph
                .cache_gen
                .fetch_add(1, std::sync::atomic::Ordering::Release);
        }

        let outcome = graph.install_built(expected, built);

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
        MANAGED_WRITE_BEFORE_MUTATION.with(|hook| {
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

/// Falsification probe for the 2026-08-07 invariant inventory, which
/// concluded that a warm Direct save of an existing page does no O(graph)
/// work. That verdict rests on reading the code, not on running it against a
/// graph with real scale and shape. If `complete_builds` climbs while
/// repeatedly saving one page, the verdict is wrong and the whole-graph
/// identity rebuild returns to the top of the cut-list.
///
/// Opt-in, because it needs a corpus this repository does not ship:
/// `TINE_REAL_GRAPH=~/research/logseq-anonymized`. The corpus is copied, so
/// Wave 3 Cc C6 acceptance gate over the anonymized corpus, through the
/// MANAGED constructor (`managed_entry_for_managed_path`), not the graph-wide
/// converter that `crates/tine-core/tests/graph.rs::managed_inventory_kind_census`
/// exercises. Prints aggregate kind counts only; corpus content is never
/// printed. Before C6 every eligible file under `journals/` was a Journal
/// regardless of its stem; after C6 identity follows the journal-title parse,
/// so a `journals/<non-date>.md` file is a Page, as in OG and Direct mode.
#[test]
#[ignore = "manual Wave 3 Cc gate: managed-constructor inventory kinds; set TINE_MANAGED_INVENTORY_CENSUS_GRAPH"]
fn managed_constructor_inventory_kind_census() {
    let Some(source) = std::env::var_os("TINE_MANAGED_INVENTORY_CENSUS_GRAPH") else {
        eprintln!("skipped: set TINE_MANAGED_INVENTORY_CENSUS_GRAPH to a corpus directory");
        return;
    };
    let graph = Graph::open(PathBuf::from(source));
    let inventory = graph.initial_shadow_raw_managed_text_inventory().unwrap();
    let (mut pages, mut journals, mut journal_dir_pages, mut rejected) =
        (0usize, 0usize, 0usize, 0usize);
    for (path, _bytes) in &inventory {
        match graph.managed_entry_for_managed_path(path) {
            Ok(entry) => match entry.kind {
                PageKind::Journal => journals += 1,
                PageKind::Page => {
                    pages += 1;
                    if entry.rel_path.starts_with("journals/") {
                        journal_dir_pages += 1;
                    }
                }
            },
            Err(_) => rejected += 1,
        }
    }
    eprintln!(
        "managed_constructor_inventory_kind_census total={} pages={} journals={} journal_dir_pages={} rejected={}",
        inventory.len(),
        pages,
        journals,
        journal_dir_pages,
        rejected
    );
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
    assert_eq!(
        guarded_test_resave(&graph_b, &mut page_b, "refused")
            .unwrap_err()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(
        graph_b.guarded_graph_text_identity_stats().0,
        1,
        "link-count refusal must not rebuild a sibling's complete index"
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
fn resource_epoch_does_not_make_existing_sibling_saves_rebuild() {
    let dir = scratch("guarded-resource-epoch-warm");
    fs::write(dir.join("pages/A.md"), "- a\n").unwrap();
    fs::write(dir.join("pages/B.md"), "- b\n").unwrap();
    let (graph_a, graph_b) = guarded_test_warm_pair(&dir);
    let mut page_a = graph_a.load_by_path("pages/A.md").unwrap().unwrap();
    let mut page_b = graph_b.load_by_path("pages/B.md").unwrap().unwrap();

    guarded_test_resave(&graph_a, &mut page_a, "a transition").unwrap();
    guarded_test_resave(&graph_b, &mut page_b, "b local save").unwrap();
    assert_eq!(graph_b.guarded_graph_text_identity_stats().0, 1);
    let epochs = graph_b.guarded_graph_text_identity_epochs();
    assert_ne!(epochs.0, Some(epochs.1));
    assert!(graph_b.guarded_graph_text_identity_stats().2);

    let before_warm = crate::fast_commit::graph_wide_commit_work();
    guarded_test_resave(&graph_b, &mut page_b, "b warm one").unwrap();
    let after_warm_one = graph_b.guarded_graph_text_identity_epochs();
    assert_ne!(after_warm_one.0, Some(after_warm_one.1));
    assert!(
        after_warm_one.1 > epochs.1,
        "an ordinary save must still advance the shared resource epoch"
    );
    guarded_test_resave(&graph_b, &mut page_b, "b warm two").unwrap();
    let after_warm_two = graph_b.guarded_graph_text_identity_epochs();
    assert_ne!(after_warm_two.0, Some(after_warm_two.1));
    assert!(
        after_warm_two.1 > after_warm_one.1,
        "each ordinary save must advance the shared resource epoch once"
    );
    assert_eq!(
        crate::fast_commit::graph_wide_commit_work().since(before_warm),
        crate::fast_commit::GraphWideCommitWork::default()
    );
    assert_eq!(graph_b.guarded_graph_text_identity_stats().0, 1);
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
    assert_eq!(
        guarded_test_resave(&graph, &mut page, "hardlink refused")
            .unwrap_err()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(
        fs::read_to_string(&primary).unwrap(),
        "- after delete\n",
        "link-count refusal must not write the target"
    );
    assert_eq!(graph.guarded_graph_text_identity_stats().0, 0);

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
    let before_reads = managed_text_capture_reads();
    let before_builds = graph.guarded_graph_text_identity_report().complete_builds;

    graph.trash_journal_file("2026_08_25.md").unwrap();

    assert_eq!(managed_text_capture_reads(), before_reads);
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
    let g = Graph::open(&dir);
    g.warm_cache();
    // (task ?b #{"TODO"}) maps to the existing Task predicate.
    let r = g.run_advanced_query(r#"[:find (pull ?b [*]) :where (task ?b #{"TODO"})]"#, None);
    assert!(r.supported);
    assert!(r.ran.contains(&"task".to_string()));
    let total: usize = r.groups.iter().map(|grp| grp.blocks.len()).sum();
    assert_eq!(total, 2, "both TODO blocks match");
    // A clause outside the subset (a raw [?e :a ?v] join) → nothing supported.
    let u = g.run_advanced_query("[:find ?b :where [?b :block/foo ?v]]", None);
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
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let query = r#"[:find (pull ?b [*])
                        :in $ ?current-page
                        :where
                        [?p :block/name ?current-page]
                        [?b :block/refs ?p]
                        (task ?b #{"TODO"})]
                       :inputs [:current-page]"#;

    let result = graph.run_advanced_query(query, Some("Focus A"));
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
    let g = Graph::open(&dir);
    g.warm_cache();

    let count = |src: &str| -> usize {
        let r = g.run_advanced_query(src, None);
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
    let r = g.run_advanced_query("[:find ?b :where (bogus ?b)]", None);
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
    let g = Graph::open(&dir);
    g.warm_cache();
    let skeleton = "[:find (pull ?b [*])\n \
             :where\n \
             ;; supported: (priority ?b \"A\") (page-ref ?b \"Nope\") (property ?b :k \"v\")\n \
             ;; (scheduled ?b) (deadline ?b) (page ?b \"Nowhere\")\n \
             (task ?b #{\"TODO\" \"DOING\"})]";
    let r = g.run_advanced_query(skeleton, None);
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
fn persisted_query_sources_cannot_reach_unbounded_cache_keys_or_parser_recursion() {
    let dir = scratch("query-source-recursion-bound");
    fs::write(dir.join("pages").join("P.md"), "- TODO ship\n").unwrap();
    let g = Graph::open(&dir);
    g.warm_cache();

    // This is the graph-authored shape that previously overflowed the Rust
    // stack when a persisted query macro rendered. Keep it below the byte
    // ceiling so the independent nesting guard is the reason it fails shut.
    let nested = format!("{}(task TODO){}", "(and ".repeat(1_000), ")".repeat(1_000));
    assert!(crate::query::query_source_within_limit(&nested));
    assert!(!crate::query::query_nesting_within_limit(&nested));
    let simple = g.run_query_bounded(&nested, 20_000, 32 * 1024 * 1024);
    assert!(simple.groups.is_empty());
    assert!(g.derived_cache.read().unwrap().is_none());

    let advanced = format!("[:find (pull ?b [*]) :where {nested}]");
    let result = g.run_advanced_query(&advanced, None);
    assert!(!result.supported);
    assert_eq!(result.ignored, vec!["query-nesting-too-deep"]);
    assert!(g.advanced_cache.read().unwrap().is_none());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn advanced_query_reuses_cached_result_until_graph_changes() {
    let dir = scratch("adv-memo");
    fs::write(dir.join("pages").join("P.md"), "- TODO ship\n").unwrap();
    fs::write(
        dir.join("pages").join("Notes.md"),
        "alias:: Scratch\n- ordinary note\n",
    )
    .unwrap();
    let g = Graph::open(&dir);
    g.warm_cache();
    let q = r#"[:find (pull ?b [*]) :where (task ?b #{"TODO"})]"#;

    let first = g.run_advanced_query_cached(q, None);
    let second = g.run_advanced_query_cached(q, None);
    assert!(
        Arc::ptr_eq(&first, &second),
        "identical advanced query should be served from the memo cache"
    );
    assert_eq!(first.groups.len(), 1);
    let bounded_key = format!("AQ\0{}\0{}\0n:\0{q}", 20_000, 32 * 1024 * 1024);
    let _ = g.run_advanced_query_bounded_cached(q, None, 20_000, 32 * 1024 * 1024);
    let bounded_first = g
        .advanced_cache
        .read()
        .unwrap()
        .as_ref()
        .unwrap()
        .results
        .get(&bounded_key)
        .unwrap()
        .0
        .result
        .clone();

    let mut notes = g.load_named("Notes", PageKind::Page).unwrap().unwrap();
    notes.blocks[0].raw = "still unrelated".into();
    g.save_page(&notes, notes.rev.as_deref()).unwrap();
    let after_unrelated = g.run_advanced_query_cached(q, None);
    let _ = g.run_advanced_query_bounded_cached(q, None, 20_000, 32 * 1024 * 1024);
    let bounded_after_unrelated = g
        .advanced_cache
        .read()
        .unwrap()
        .as_ref()
        .unwrap()
        .results
        .get(&bounded_key)
        .unwrap()
        .0
        .result
        .clone();
    assert!(
        Arc::ptr_eq(&first, &after_unrelated),
        "an unrelated edit must retain the advanced-query memo"
    );
    assert!(Arc::ptr_eq(&bounded_first, &bounded_after_unrelated));

    let mut notes = g.load_named("Notes", PageKind::Page).unwrap().unwrap();
    notes.pre_block = Some("alias:: Renamed Scratch\n".into());
    g.save_page(&notes, notes.rev.as_deref()).unwrap();
    let after_alias_change = g.run_advanced_query_cached(q, None);
    assert!(
        !Arc::ptr_eq(&first, &after_alias_change),
        "a semantic alias change must invalidate graph-wide derived results"
    );

    let mut dto = g.load_named("P", PageKind::Page).unwrap().unwrap();
    dto.blocks[0].raw = dto.blocks[0].raw.replace("TODO", "DONE");
    g.save_page(&dto, dto.rev.as_deref()).unwrap();

    let third = g.run_advanced_query_cached(q, None);
    let _ = g.run_advanced_query_bounded_cached(q, None, 20_000, 32 * 1024 * 1024);
    let bounded_after_affected = g
        .advanced_cache
        .read()
        .unwrap()
        .as_ref()
        .unwrap()
        .results
        .get(&bounded_key)
        .unwrap()
        .0
        .result
        .clone();
    assert!(
        !Arc::ptr_eq(&first, &third),
        "graph mutation must invalidate the advanced-query memo"
    );
    assert!(!Arc::ptr_eq(&bounded_first, &bounded_after_affected));
    assert!(third.groups.is_empty());
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
fn bounded_query_memo_survives_unrelated_edits_and_recomputes_affected_pages() {
    let dir = scratch("bounded-query-scoped-memo");
    fs::write(dir.join("pages").join("Tasks.md"), "- TODO ship\n").unwrap();
    fs::write(
        dir.join("pages").join("Notes.md"),
        "alias:: Scratch\n- ordinary note\n",
    )
    .unwrap();
    let g = Graph::open(&dir);
    g.warm_cache();

    let first = g.run_query_bounded("(task TODO)", 20_000, 32 * 1024 * 1024);
    let second = g.run_query_bounded("(task TODO)", 20_000, 32 * 1024 * 1024);
    assert!(Arc::ptr_eq(&first.groups, &second.groups));

    let mut notes = g.load_named("Notes", PageKind::Page).unwrap().unwrap();
    notes.blocks[0].raw = "still an ordinary note".into();
    g.save_page(&notes, notes.rev.as_deref()).unwrap();
    let after_unrelated = g.run_query_bounded("(task TODO)", 20_000, 32 * 1024 * 1024);
    assert!(
        Arc::ptr_eq(&first.groups, &after_unrelated.groups),
        "an unrelated edit must retain the scoped bounded-query memo"
    );

    let mut tasks = g.load_named("Tasks", PageKind::Page).unwrap().unwrap();
    tasks.blocks[0].raw = "DONE ship".into();
    g.save_page(&tasks, tasks.rev.as_deref()).unwrap();
    let after_affected = g.run_query_bounded("(task TODO)", 20_000, 32 * 1024 * 1024);
    assert!(!Arc::ptr_eq(&first.groups, &after_affected.groups));
    assert!(after_affected.groups.is_empty());
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
fn overflowed_bounded_memo_recomputes_when_an_omitted_match_stops_matching() {
    let dir = scratch("bounded-overflow-negative-transition");
    fs::write(dir.join("pages").join("A.md"), "- TODO first\n").unwrap();
    fs::write(dir.join("pages").join("B.md"), "- TODO second\n").unwrap();
    fs::write(dir.join("pages").join("Notes.md"), "- unrelated\n").unwrap();
    let g = Graph::open(&dir);
    g.warm_cache();

    let first = g.run_query_bounded("(task TODO)", 1, 32 * 1024 * 1024);
    assert!(first.exceeded);
    assert_eq!(first.total, 2);
    let mut notes = g.load_named("Notes", PageKind::Page).unwrap().unwrap();
    notes.blocks[0].raw = "still unrelated".into();
    g.save_page(&notes, notes.rev.as_deref()).unwrap();
    let after_unrelated = g.run_query_bounded("(task TODO)", 1, 32 * 1024 * 1024);
    assert!(Arc::ptr_eq(&first.groups, &after_unrelated.groups));
    assert!(after_unrelated.exceeded);
    assert_eq!(after_unrelated.total, 2);

    let admitted = first.groups[0].page.clone();
    let omitted = if admitted == "A" { "B" } else { "A" };
    let mut page = g.load_named(omitted, PageKind::Page).unwrap().unwrap();
    page.blocks[0].raw = "DONE no longer matches".into();
    g.save_page(&page, page.rev.as_deref()).unwrap();

    let after = g.run_query_bounded("(task TODO)", 1, 32 * 1024 * 1024);
    assert!(!Arc::ptr_eq(&first.groups, &after.groups));
    assert!(!after.exceeded);
    assert_eq!(after.total, 1);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn advanced_cache_invalidation_preserves_nul_inside_opaque_query_source() {
    let dir = scratch("advanced-cache-nul-query");
    fs::write(dir.join("pages").join("P.md"), "- DONE ship\n").unwrap();
    let g = Graph::open(&dir);
    g.warm_cache();
    let query = "[:find (pull ?b [*]) :where \0 (task ?b #{\"TODO\"})]";
    let first = g.run_advanced_query_cached(query, None);
    assert!(first.groups.is_empty());

    let mut page = g.load_named("P", PageKind::Page).unwrap().unwrap();
    page.blocks[0].raw = "TODO ship".into();
    g.save_page(&page, page.rev.as_deref()).unwrap();
    let warm = g.run_advanced_query_cached(query, None);
    let fresh = Graph::open(&dir).run_advanced_query(query, None);
    assert_eq!(warm.groups.len(), 1);
    assert_eq!(warm.groups.len(), fresh.groups.len());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn derived_and_advanced_memos_are_lru_bounded() {
    let dir = scratch("memo-lru-bound");
    let g = Graph::open(&dir);
    for i in 0..(DERIVED_CACHE_MAX_ENTRIES + 20) {
        let _ = g.derived_memo(format!("test\0{i}"), Vec::new);
        let _ = g.advanced_memo(format!("test\0{i}"), || crate::query::AdvancedResult {
            groups: Vec::new(),
            ran: Vec::new(),
            ignored: Vec::new(),
            supported: true,
        });
    }
    let oversized_key = "x".repeat(DERIVED_CACHE_MAX_ENTRY_BYTES / 2 + 1);
    let _ = g.derived_memo(oversized_key.clone(), Vec::new);
    let _ = g.advanced_memo(oversized_key.clone(), || crate::query::AdvancedResult {
        groups: Vec::new(),
        ran: Vec::new(),
        ignored: Vec::new(),
        supported: true,
    });
    let derived = g.derived_cache.read().unwrap();
    let advanced = g.advanced_cache.read().unwrap();
    assert_eq!(
        derived.as_ref().unwrap().results.len(),
        DERIVED_CACHE_MAX_ENTRIES
    );
    assert!(!derived
        .as_ref()
        .unwrap()
        .results
        .contains_key(&oversized_key));
    assert!(!advanced
        .as_ref()
        .unwrap()
        .results
        .contains_key(&oversized_key));
    assert_eq!(
        advanced.as_ref().unwrap().results.len(),
        DERIVED_CACHE_MAX_ENTRIES
    );
    let oldest = format!("test\0{}", 0);
    assert!(!derived.as_ref().unwrap().results.contains_key(&oldest));
    assert!(!advanced.as_ref().unwrap().results.contains_key(&oldest));
    let _ = fs::remove_dir_all(&dir);
}

// ---- sparse projection exact-write bridge ----

fn projection_recovery_bytes(parent: &Path) -> Vec<Vec<u8>> {
    fs::read_dir(parent)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.ends_with(".projection.recovery"))
        })
        .map(|entry| fs::read(entry.path()).unwrap())
        .collect()
}

fn projection_recovery_paths(parent: &Path) -> Vec<PathBuf> {
    fs::read_dir(parent)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_name().to_str().is_some_and(|name| {
                name.ends_with(".projection.recovery")
                    || name.ends_with(".projection.staged")
                    || name.ends_with(".projection.withdrawn")
                    || name.ends_with(".projection-staged-recovery")
            })
        })
        .map(|entry| entry.path())
        .collect()
}

#[cfg(unix)]
#[test]
fn projection_late_write_remove_and_restore_collisions_preserve_every_version() {
    // A portable loser introduced after the previous scan cannot redirect
    // exact projection authority or block the accepted target.
    let dir = scratch("projection-controllable-write-window");
    let target = dir.join("pages/LateWrite.md");
    let alias = dir.join("pages/latewrite.md");
    fs::write(&target, b"- base\n").unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    PROJECTION_LATE_COLLISION.with(|hook| {
        let alias = alias.clone();
        *hook.borrow_mut() = Some(Box::new(move || fs::write(alias, b"- alias\n")));
    });
    graph
        .write_projection_exact("pages/LateWrite.md", Some(b"- base\n"), b"- target\n")
        .unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"- target\n");
    assert_eq!(fs::read(&alias).unwrap(), b"- alias\n");
    assert!(projection_recovery_bytes(target.parent().unwrap())
        .iter()
        .any(|bytes| bytes == b"- base\n"));
    let _ = fs::remove_dir_all(&dir);

    // A same-inode alias introduced in the corresponding removal window
    // does not expand exact-path authority into a directory-wide inode
    // search. Retire the accepted target, preserve its bytes as recovery
    // evidence, and leave the independently named alias untouched.
    let dir = scratch("projection-controllable-remove-window");
    let target = dir.join("pages/LateRemove.md");
    let alias = dir.join("pages/LateRemoveAlias.md");
    fs::write(&target, b"- base\n").unwrap();
    let identity =
        canonical_projection_file_resource_id(&fs::File::open(&target).unwrap()).unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let generation = graph.cache_generation();
    PROJECTION_LATE_COLLISION.with(|hook| {
        let target = target.clone();
        let alias = alias.clone();
        *hook.borrow_mut() = Some(Box::new(move || fs::hard_link(target, alias)));
    });
    graph
        .remove_projection_exact("pages/LateRemove.md", b"- base\n")
        .unwrap();
    assert!(!target.exists());
    assert_eq!(fs::read(&alias).unwrap(), b"- base\n");
    assert_eq!(
        canonical_projection_file_resource_id(&fs::File::open(&alias).unwrap()).unwrap(),
        identity
    );
    assert!(projection_recovery_bytes(alias.parent().unwrap())
        .iter()
        .any(|bytes| bytes == b"- base\n"));
    assert!(graph.cache_generation() > generation);
    fs::remove_file(&alias).unwrap();
    let _ = fs::remove_dir_all(&dir);

    // Force an unwind after retirement, then introduce a hard-link alias
    // only in the restoration window. Exact-path authority restores the
    // accepted target without treating the independently named alias as a
    // graph-wide collision.
    let dir = scratch("projection-controllable-restore-window");
    let target = dir.join("pages/LateRestore.md");
    let alias = dir.join("pages/LateRestoreAlias.md");
    fs::write(&target, b"- base\n").unwrap();
    let original_identity =
        canonical_projection_file_resource_id(&fs::File::open(&target).unwrap()).unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let generation = graph.cache_generation();
    let revisions = graph.disk_revs.read().unwrap().clone();
    PROJECTION_AFTER_RETIRE_COLLISION.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(|| {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "injected post-retirement unwind",
            ))
        }));
    });
    PROJECTION_BEFORE_RESTORE.with(|hook| {
        let parent = target.parent().unwrap().to_path_buf();
        let alias = alias.clone();
        *hook.borrow_mut() = Some(Box::new(move || {
            let recovery = fs::read_dir(&parent)?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .find(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.ends_with(".projection.recovery"))
                })
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
            fs::hard_link(recovery, alias)
        }));
    });
    let error = graph
        .remove_projection_exact("pages/LateRestore.md", b"- base\n")
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted, "{error}");
    assert_eq!(error.to_string(), "injected post-retirement unwind");
    assert_eq!(fs::read(&target).unwrap(), b"- base\n");
    assert_eq!(fs::read(&alias).unwrap(), b"- base\n");
    assert_eq!(
        canonical_projection_file_resource_id(&fs::File::open(&alias).unwrap()).unwrap(),
        original_identity
    );
    assert!(projection_recovery_bytes(target.parent().unwrap())
        .iter()
        .any(|bytes| bytes == b"- base\n"));
    assert_eq!(graph.cache_generation(), generation);
    assert_eq!(*graph.disk_revs.read().unwrap(), revisions);
    fs::remove_file(&alias).unwrap();
    graph
        .recover_projection_exact("pages/LateRestore.md", b"- base\n")
        .unwrap();
    let _ = fs::remove_dir_all(&dir);

    // A portable loser appearing after publication remains untouched while
    // the exact accepted target completes normally.
    let dir = scratch("projection-controllable-post-publish-window");
    let target = dir.join("pages/LatePublished.md");
    let alias = dir.join("pages/latepublished.md");
    fs::write(&target, b"- base\n").unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    PROJECTION_POST_PUBLISH_COLLISION.with(|hook| {
        let alias = alias.clone();
        *hook.borrow_mut() = Some(Box::new(move || fs::write(alias, b"- alias\n")));
    });
    graph
        .write_projection_exact("pages/LatePublished.md", Some(b"- base\n"), b"- target\n")
        .unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"- target\n");
    assert_eq!(fs::read(&alias).unwrap(), b"- alias\n");
    let retained = projection_recovery_paths(target.parent().unwrap())
        .into_iter()
        .map(|path| fs::read(path).unwrap())
        .collect::<Vec<_>>();
    assert!(retained.iter().any(|bytes| bytes == b"- base\n"));
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn projection_mutating_hook_errors_never_restore_unvalidated_bytes() {
    let base = b"- base\n";
    let projected = b"- target\n";
    let unknown = b"- unknown hook bytes\n";

    // A post-publish hook can change the exact live file without creating
    // a graph collision. The changed object must be withdrawn under the
    // attempt-derived published-evidence name before the hook error returns.
    let dir = scratch("projection-post-publish-byte-error");
    let target = dir.join("pages/PostPublishBytes.md");
    fs::write(&target, base).unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let generation = graph.cache_generation();
    let revisions = graph.disk_revs.read().unwrap().clone();
    PROJECTION_POST_PUBLISH_COLLISION.with(|hook| {
        let target = target.clone();
        *hook.borrow_mut() = Some(Box::new(move || {
            fs::write(&target, unknown)?;
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "injected post-publish byte rewrite failure",
            ))
        }));
    });
    let error = graph
        .write_projection_exact("pages/PostPublishBytes.md", Some(base), projected)
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{error}");
    if target.exists() {
        assert_eq!(fs::read(&target).unwrap(), base);
    }
    let evidence = projection_recovery_paths(target.parent().unwrap());
    assert!(evidence.iter().any(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".projection.withdrawn"))
            && fs::read(path).unwrap() == unknown
    }));
    assert_eq!(graph.cache_generation(), generation);
    assert_eq!(*graph.disk_revs.read().unwrap(), revisions);
    let _ = fs::remove_dir_all(&dir);

    // Mutating the retired object and then returning a hook error must not
    // make those changed bytes eligible for restoration to the live name.
    let dir = scratch("projection-post-retire-byte-error");
    let target = dir.join("pages/PostRetireBytes.md");
    fs::write(&target, base).unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let generation = graph.cache_generation();
    let revisions = graph.disk_revs.read().unwrap().clone();
    PROJECTION_AFTER_RETIRE_COLLISION.with(|hook| {
        let parent = target.parent().unwrap().to_path_buf();
        *hook.borrow_mut() = Some(Box::new(move || {
            let recovery = projection_recovery_paths(&parent)
                .into_iter()
                .find(|path| fs::read(path).unwrap() == base)
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
            fs::write(recovery, unknown)?;
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "injected post-retire byte rewrite failure",
            ))
        }));
    });
    let error = graph
        .write_projection_exact("pages/PostRetireBytes.md", Some(base), projected)
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{error}");
    assert!(!target.exists());
    let retained = projection_recovery_paths(target.parent().unwrap())
        .into_iter()
        .map(|path| fs::read(path).unwrap())
        .collect::<Vec<_>>();
    assert!(retained.iter().any(|bytes| bytes == unknown));
    assert!(retained.iter().any(|bytes| bytes == projected));
    assert_eq!(graph.cache_generation(), generation);
    assert_eq!(*graph.disk_revs.read().unwrap(), revisions);
    let _ = fs::remove_dir_all(&dir);

    // Replacing the recovery name in the restoration hook must still run
    // exact recovery-object validation even though that hook itself errors.
    let dir = scratch("projection-before-restore-replacement-error");
    let target = dir.join("pages/BeforeRestoreBytes.md");
    fs::write(&target, base).unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let generation = graph.cache_generation();
    let revisions = graph.disk_revs.read().unwrap().clone();
    PROJECTION_AFTER_RETIRE_COLLISION.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(|| {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "injected post-retire restoration trigger",
            ))
        }));
    });
    PROJECTION_BEFORE_RESTORE.with(|hook| {
        let parent = target.parent().unwrap().to_path_buf();
        *hook.borrow_mut() = Some(Box::new(move || {
            let recovery = projection_recovery_paths(&parent)
                .into_iter()
                .find(|path| fs::read(path).unwrap() == base)
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
            fs::remove_file(&recovery)?;
            fs::write(&recovery, unknown)?;
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "injected before-restore replacement failure",
            ))
        }));
    });
    let error = graph
        .write_projection_exact("pages/BeforeRestoreBytes.md", Some(base), projected)
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted, "{error}");
    assert!(
        error
            .to_string()
            .contains("post-hook projection validation also failed"),
        "{error}"
    );
    assert!(!target.exists());
    let retained = projection_recovery_paths(target.parent().unwrap())
        .into_iter()
        .map(|path| fs::read(path).unwrap())
        .collect::<Vec<_>>();
    assert!(retained.iter().any(|bytes| bytes == unknown));
    assert!(retained.iter().any(|bytes| bytes == projected));
    assert_eq!(graph.cache_generation(), generation);
    assert_eq!(*graph.disk_revs.read().unwrap(), revisions);
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn projection_pre_retirement_published_recovery_collision_preserves_authority() {
    use crate::oplog::projection_store::ProjectionReceiptStore;

    let dir = scratch("projection-remove-pre-retirement-published-collision");
    let receipts = dir.with_file_name(format!(
        "tine-projection-remove-pre-retirement-published-collision-receipts-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&receipts);
    fs::create_dir_all(&receipts).unwrap();
    let path = "pages/PreRetirementCollision.md";
    let target = dir.join(path);
    let base = b"- exact base\n";
    let unknown = b"- unknown published-name collision\n";
    fs::write(&target, base).unwrap();
    let workspace_id = WorkspaceId::from_uuid(Uuid::from_u128(91_046_080));
    let intent = crate::oplog::ProjectionIntent::new(
        workspace_id,
        crate::oplog::PageId::from_uuid(Uuid::from_u128(91_046_081)),
        ManagedPath::parse(path).unwrap(),
        crate::oplog::FrontierV2::default(),
        Vec::new(),
        crate::oplog::ProjectionPrecondition::Base(BlobDescription::of(base)),
        crate::oplog::ProjectionTargetKind::Absent,
        BlobDescription::of(&[]),
        Vec::new(),
    )
    .unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let generation = graph.cache_generation();
    let revisions = graph.disk_revs.read().unwrap().clone();
    let store = ProjectionReceiptStore::open(&receipts, workspace_id).unwrap();
    store.publish_intent(&intent, Some(base)).unwrap();
    let reservation = store.reserve_attempt(&intent).unwrap();
    let target_path = graph.projection_page_target(path).unwrap();
    let retired_path = dir.join("pages").join(reservation.recovery_filename());
    let published_path = dir
        .join("pages")
        .join(projection_attempt_published_recovery_filename(&target_path, &reservation).unwrap());
    assert!(!retired_path.exists());
    assert!(!published_path.exists());
    assert!(store.load_completion(&intent).unwrap().is_none());

    PROJECTION_LATE_COLLISION.with(|hook| {
        let published_path = published_path.clone();
        *hook.borrow_mut() = Some(Box::new(move || fs::write(published_path, unknown)));
    });
    let mut authority = store.begin_mutation(&intent, Some(&reservation)).unwrap();
    let error = graph
        .remove_page_projection(path, base, &mut authority)
        .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{error}");
    assert_eq!(fs::read(&target).unwrap(), base);
    assert_eq!(fs::read(&published_path).unwrap(), unknown);
    assert!(!retired_path.exists());
    assert_eq!(graph.cache_generation(), generation);
    assert_eq!(*graph.disk_revs.read().unwrap(), revisions);
    assert!(!graph.recent_writes.lock().unwrap().contains_key(&target));
    assert!(store.load_completion(&intent).unwrap().is_none());
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&receipts);
}

#[cfg(unix)]
#[test]
fn projection_hook_errors_revalidate_and_retain_attempt_bound_recovery() {
    use crate::oplog::projection_store::ProjectionReceiptStore;

    fn projection_intent(
        workspace_id: WorkspaceId,
        page_seed: u128,
        path: &str,
        base: &[u8],
        target: &[u8],
    ) -> crate::oplog::ProjectionIntent {
        crate::oplog::ProjectionIntent::new(
            workspace_id,
            crate::oplog::PageId::from_uuid(Uuid::from_u128(page_seed)),
            ManagedPath::parse(path).unwrap(),
            crate::oplog::FrontierV2::default(),
            Vec::new(),
            crate::oplog::ProjectionPrecondition::Base(BlobDescription::of(base)),
            crate::oplog::ProjectionTargetKind::Present,
            BlobDescription::of(target),
            Vec::new(),
        )
        .unwrap()
    }

    let base = b"- base\n";
    let projected = b"- target\n";

    // A removal hook can recreate the retired live name with unrelated
    // regular bytes and then fail. Those bytes belong to the failed
    // attempt, not to the graph, so they must be withdrawn under that
    // attempt's exact published-evidence name before restoration.
    let dir = scratch("projection-remove-post-publish-byte-error-authority");
    let receipts = dir.with_file_name(format!(
        "tine-projection-remove-post-publish-byte-error-authority-receipts-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&receipts);
    fs::create_dir_all(&receipts).unwrap();
    let path = "pages/RemoveHookError.md";
    let target = dir.join(path);
    let unknown = b"- unknown removal hook bytes\n";
    fs::write(&target, base).unwrap();
    let workspace_id = WorkspaceId::from_uuid(Uuid::from_u128(91_046_090));
    let intent = projection_intent(workspace_id, 91_046_091, path, base, &[]);
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let generation = graph.cache_generation();
    let revisions = graph.disk_revs.read().unwrap().clone();
    let store = ProjectionReceiptStore::open(&receipts, workspace_id).unwrap();
    store.publish_intent(&intent, Some(base)).unwrap();
    let reservation = store.reserve_attempt(&intent).unwrap();
    let target_path = graph.projection_page_target(path).unwrap();
    let retired_path = dir.join("pages").join(reservation.recovery_filename());
    let published_path = dir
        .join("pages")
        .join(projection_attempt_published_recovery_filename(&target_path, &reservation).unwrap());
    let mut authority = store.begin_mutation(&intent, Some(&reservation)).unwrap();
    PROJECTION_POST_PUBLISH_COLLISION.with(|hook| {
        let target = target.clone();
        *hook.borrow_mut() = Some(Box::new(move || {
            fs::write(&target, unknown)?;
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "injected removal post-publication byte rewrite failure",
            ))
        }));
    });
    let error = graph
        .remove_page_projection(path, base, &mut authority)
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{error}");
    assert_eq!(fs::read(&published_path).unwrap(), unknown);
    assert_eq!(fs::read(&retired_path).unwrap(), base);
    if target.exists() {
        assert_eq!(fs::read(&target).unwrap(), base);
    }
    assert_eq!(graph.cache_generation(), generation);
    assert_eq!(*graph.disk_revs.read().unwrap(), revisions);
    assert!(!graph.recent_writes.lock().unwrap().contains_key(&target));
    assert!(store.load_completion(&intent).unwrap().is_none());
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&receipts);

    // A clean post-publication hook error leaves an exact live target but
    // no completion. Reopening both durable authorities must reconstruct
    // completion without the process-local test attempt catalog.
    let dir = scratch("projection-post-publish-hook-error-clean-authority");
    let receipts = dir.with_file_name(format!(
        "tine-projection-post-publish-hook-error-clean-authority-receipts-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&receipts);
    fs::create_dir_all(&receipts).unwrap();
    let path = "pages/HookErrorClean.md";
    let target = dir.join(path);
    fs::write(&target, base).unwrap();
    let workspace_id = WorkspaceId::from_uuid(Uuid::from_u128(91_046_100));
    let intent = projection_intent(workspace_id, 91_046_101, path, base, projected);
    let graph = Graph::open(&dir);
    let store = ProjectionReceiptStore::open(&receipts, workspace_id).unwrap();
    store.publish_intent(&intent, Some(base)).unwrap();
    let reservation = store.reserve_attempt(&intent).unwrap();
    let target_path = graph.projection_page_target(path).unwrap();
    let attempted_name =
        projection_attempt_target_recovery_filename(&target_path, &reservation).unwrap();
    let mut authority = store.begin_mutation(&intent, Some(&reservation)).unwrap();
    PROJECTION_POST_PUBLISH_COLLISION.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(|| {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "injected clean post-publication hook failure",
            ))
        }));
    });
    let error = graph
        .write_page_projection(path, Some(base), projected, &mut authority)
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted, "{error}");
    assert_eq!(fs::read(&target).unwrap(), projected);
    assert!(!dir.join("pages").join(&attempted_name).exists());
    drop(authority);
    drop(store);
    drop(graph);

    let graph = Graph::open(&dir);
    let store = ProjectionReceiptStore::open(&receipts, workspace_id).unwrap();
    let mut recovery = store.begin_mutation(&intent, None).unwrap();
    let proof = graph
        .recover_page_projection(path, Some(base), projected, &mut recovery)
        .unwrap();
    store
        .reconstruct_completion(recovery, &intent, projected, &proof)
        .unwrap();
    assert!(store.load_completion(&intent).unwrap().is_some());
    assert_eq!(fs::read(&target).unwrap(), projected);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&receipts);

    // Exact durable projection authority retains a portable sibling
    // without letting it redirect the operation or hide the injected hook
    // error. Reopened recovery uses the attempt-bound evidence while both
    // exact spellings remain visible.
    let dir = scratch("projection-post-publish-hook-error-collision-authority");
    let receipts = dir.with_file_name(format!(
        "tine-projection-post-publish-hook-error-collision-authority-receipts-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&receipts);
    fs::create_dir_all(&receipts).unwrap();
    let path = "pages/HookError.md";
    let target = dir.join(path);
    let alias = dir.join("pages/hookerror.md");
    fs::write(&target, base).unwrap();
    let workspace_id = WorkspaceId::from_uuid(Uuid::from_u128(91_046_110));
    let intent = projection_intent(workspace_id, 91_046_111, path, base, projected);
    let graph = Graph::open(&dir);
    let store = ProjectionReceiptStore::open(&receipts, workspace_id).unwrap();
    store.publish_intent(&intent, Some(base)).unwrap();
    let reservation = store.reserve_attempt(&intent).unwrap();
    let target_path = graph.projection_page_target(path).unwrap();
    let attempted_name =
        projection_attempt_target_recovery_filename(&target_path, &reservation).unwrap();
    let published_name =
        projection_attempt_published_recovery_filename(&target_path, &reservation).unwrap();
    let retired_name = reservation.recovery_filename().to_owned();
    let mut authority = store.begin_mutation(&intent, Some(&reservation)).unwrap();
    PROJECTION_POST_PUBLISH_COLLISION.with(|hook| {
        let alias = alias.clone();
        *hook.borrow_mut() = Some(Box::new(move || {
            fs::write(alias, b"- alias\n")?;
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "injected post-publication hook failure",
            ))
        }));
    });
    let error = graph
        .write_page_projection(path, Some(base), projected, &mut authority)
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted, "{error}");
    assert_eq!(fs::read(&target).unwrap(), projected);
    assert_eq!(fs::read(&alias).unwrap(), b"- alias\n");
    assert!(!dir.join("pages").join(&attempted_name).exists());
    assert!(!dir.join("pages").join(&published_name).exists());
    assert_eq!(
        fs::read(dir.join("pages").join(&retired_name)).unwrap(),
        base
    );
    drop(authority);
    drop(store);
    drop(graph);

    let graph = Graph::open(&dir);
    let store = ProjectionReceiptStore::open(&receipts, workspace_id).unwrap();
    let mut recovery = store.begin_mutation(&intent, None).unwrap();
    let proof = graph
        .recover_page_projection(path, Some(base), projected, &mut recovery)
        .unwrap();
    store
        .reconstruct_completion(recovery, &intent, projected, &proof)
        .unwrap();
    assert!(store.load_completion(&intent).unwrap().is_some());
    assert_eq!(fs::read(&target).unwrap(), projected);
    assert_eq!(fs::read(&alias).unwrap(), b"- alias\n");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&receipts);

    // Replacing one exact attempt-derived object with mismatching bytes
    // must block reopened recovery before completion or fallback mutation.
    let dir = scratch("projection-post-publish-hook-error-tamper-authority");
    let receipts = dir.with_file_name(format!(
        "tine-projection-post-publish-hook-error-tamper-authority-receipts-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&receipts);
    fs::create_dir_all(&receipts).unwrap();
    let path = "pages/HookErrorTamper.md";
    let target = dir.join(path);
    fs::write(&target, base).unwrap();
    let workspace_id = WorkspaceId::from_uuid(Uuid::from_u128(91_046_120));
    let intent = projection_intent(workspace_id, 91_046_121, path, base, projected);
    let graph = Graph::open(&dir);
    let store = ProjectionReceiptStore::open(&receipts, workspace_id).unwrap();
    store.publish_intent(&intent, Some(base)).unwrap();
    let reservation = store.reserve_attempt(&intent).unwrap();
    let target_path = graph.projection_page_target(path).unwrap();
    let attempted_name =
        projection_attempt_target_recovery_filename(&target_path, &reservation).unwrap();
    let attempted_path = dir.join("pages").join(attempted_name);
    let mut authority = store.begin_mutation(&intent, Some(&reservation)).unwrap();
    PROJECTION_POST_PUBLISH_COLLISION.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(|| {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "injected tamper post-publication hook failure",
            ))
        }));
    });
    assert_eq!(
        graph
            .write_page_projection(path, Some(base), projected, &mut authority)
            .unwrap_err()
            .kind(),
        io::ErrorKind::Interrupted
    );
    fs::write(&attempted_path, b"- replaced evidence\n").unwrap();
    drop(authority);
    drop(store);
    drop(graph);

    let before = regular_file_tree(&dir);
    let graph = Graph::open(&dir);
    let store = ProjectionReceiptStore::open(&receipts, workspace_id).unwrap();
    let mut recovery = store.begin_mutation(&intent, None).unwrap();
    let error = graph
        .recover_page_projection(path, Some(base), projected, &mut recovery)
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{error}");
    assert_eq!(regular_file_tree(&dir), before);
    assert!(store.load_completion(&intent).unwrap().is_none());
    assert!(store.reserve_fallback_attempt(&intent).is_err());
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&receipts);
}

#[cfg(unix)]
#[test]
fn exact_projection_authority_ignores_portable_losers_and_resource_aliases() {
    // Exact creation grants no authority over the portable loser.
    let dir = scratch("projection-late-collision-write");
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let target = dir.join("pages/nested/ProjectionWrite.md");
    let alias = dir.join("pages/nested/projectionwrite.md");
    fs::create_dir_all(alias.parent().unwrap()).unwrap();
    fs::write(&alias, b"- alias\n").unwrap();
    graph
        .write_projection_exact("pages/nested/ProjectionWrite.md", None, b"- projected\n")
        .unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"- projected\n");
    assert_eq!(fs::read(&alias).unwrap(), b"- alias\n");
    assert!(projection_recovery_bytes(alias.parent().unwrap()).is_empty());
    let _ = fs::remove_dir_all(&dir);

    // Removal owns the exact accepted path. A newly-created hard link is a
    // separately named survivor, not a reason to scan every graph sibling.
    let dir = scratch("projection-late-collision-remove");
    let target = dir.join("pages/ProjectionRemove.md");
    let alias = dir.join("pages/ProjectionRemoveAlias.md");
    fs::write(&target, b"- base\n").unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    fs::hard_link(&target, &alias).unwrap();
    let generation = graph.cache_generation();
    graph
        .remove_projection_exact("pages/ProjectionRemove.md", b"- base\n")
        .unwrap();
    assert!(!target.exists());
    assert_eq!(fs::read(&alias).unwrap(), b"- base\n");
    assert!(projection_recovery_bytes(alias.parent().unwrap())
        .iter()
        .any(|bytes| bytes == b"- base\n"));
    assert!(graph.cache_generation() > generation);
    fs::remove_file(&alias).unwrap();
    let _ = fs::remove_dir_all(&dir);

    // Present-target recovery proves only the accepted exact path.
    let dir = scratch("projection-late-collision-recovery");
    let graph = Graph::open(&dir);
    graph
        .write_projection_exact("pages/ProjectionRecover.md", None, b"- target\n")
        .unwrap();
    let target = dir.join("pages/ProjectionRecover.md");
    let alias = dir.join("pages/projectionrecover.md");
    fs::write(&alias, b"- alias\n").unwrap();
    graph
        .recover_projection_exact("pages/ProjectionRecover.md", b"- target\n")
        .unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"- target\n");
    assert_eq!(fs::read(&alias).unwrap(), b"- alias\n");
    let _ = fs::remove_dir_all(&dir);

    // Recovered removal proves exact absence without deleting the loser.
    let dir = scratch("projection-late-collision-removed-recovery");
    let target = dir.join("pages/ProjectionRemoved.md");
    fs::write(&target, b"- base\n").unwrap();
    let graph = Graph::open(&dir);
    graph
        .remove_projection_exact("pages/ProjectionRemoved.md", b"- base\n")
        .unwrap();
    let alias = dir.join("pages/projectionremoved.md");
    fs::write(&alias, b"- alias\n").unwrap();
    let evidence = projection_recovery_bytes(target.parent().unwrap());
    graph
        .recover_removed_projection_exact("pages/ProjectionRemoved.md", b"- base\n")
        .unwrap();
    assert!(!target.exists());
    assert_eq!(fs::read(&alias).unwrap(), b"- alias\n");
    assert_eq!(
        projection_recovery_bytes(target.parent().unwrap()),
        evidence
    );
    let _ = fs::remove_dir_all(&dir);

    // Removal confirmation likewise cannot delete a portable loser.
    let dir = scratch("projection-late-collision-confirmation");
    let graph = Graph::open(&dir);
    let alias = dir.join("pages/projectionconfirm.md");
    fs::write(&alias, b"- alias\n").unwrap();
    graph
        .confirm_removed_projection_exact("pages/ProjectionConfirm.md")
        .unwrap();
    assert_eq!(fs::read(&alias).unwrap(), b"- alias\n");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn projection_exact_proof_binds_path_bytes_digest_and_exact_preconditions() {
    let dir = scratch("projection-exact-base");
    let path = dir.join("pages/Projection.md");
    let graph = Graph::open(&dir);

    let proof = graph
        .write_projection_exact("pages/Projection.md", None, b"- first\n")
        .unwrap();
    assert_eq!(proof.path(), "pages/Projection.md");
    assert_eq!(proof.bytes(), b"- first\n");
    let expected_digest: [u8; 32] = Sha256::digest(b"- first\n").into();
    assert_eq!(proof.digest(), &expected_digest);
    assert!(proof.recovery_evidence().is_empty());

    let proof = graph
        .write_projection_exact("pages/Projection.md", Some(b"- first\n"), b"- second\n")
        .unwrap();
    assert_eq!(proof.path(), "pages/Projection.md");
    assert_eq!(proof.bytes(), b"- second\n");
    let evidence = proof
        .recovery_evidence()
        .first()
        .expect("replacement must bind retained displacement");
    assert_eq!(evidence.len(), b"- first\n".len() as u64);
    assert_eq!(
        evidence.digest(),
        &<[u8; 32]>::from(Sha256::digest(b"- first\n"))
    );
    assert!(evidence.path().starts_with("pages/.Projection.md."));
    assert!(evidence.filename().ends_with(".projection.recovery"));
    assert_eq!(
        graph
            .read_projection_recovery_evidence("pages/Projection.md", evidence)
            .unwrap(),
        b"- first\n"
    );
    assert!(graph
        .write_projection_exact("pages/Projection.md", Some(b"- stale\n"), b"- clobber\n")
        .is_err());
    assert_eq!(fs::read(&path).unwrap(), b"- second\n");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn root_projection_admission_writes_the_exact_root_path_without_panicking() {
    let dir = scratch("projection-root-admission");
    let graph = Graph::open(&dir);

    // OG's recursive `get-files` walk starts at the graph root itself, so a
    // graph-root page is ordinary graph text. Its projection target has no
    // parent components at all; admitting it must neither panic nor move it
    // into `pages/`.
    let projected = graph
        .write_projection_exact("Root.md", None, b"- target\n")
        .unwrap();
    assert_eq!(projected.path(), "Root.md");
    assert_eq!(fs::read(dir.join("Root.md")).unwrap(), b"- target\n");
    assert!(!dir.join("pages/Root.md").exists());

    // A root-level name outside the graph-text scope still fails closed.
    let error = graph
        .write_projection_exact(".Hidden.md", None, b"- target\n")
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(!dir.join(".Hidden.md").exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn projection_recovery_evidence_paths_are_root_safe_and_forgery_fails_closed() {
    let root_filename = ".Root.md.00000000000000000000000000000001.projection.recovery".to_owned();
    let root = ProjectionRecoveryEvidence::new(
        "Root.md",
        root_filename.clone(),
        Some(ContentDigest::from_bytes([1; 32])),
        b"- root\n",
    )
    .unwrap();
    assert_eq!(root.path(), root_filename);

    let nested_filename =
        ".Target.md.00000000000000000000000000000002.projection.recovery".to_owned();
    let nested = ProjectionRecoveryEvidence::new(
        "pages/deep/Target.md",
        nested_filename.clone(),
        Some(ContentDigest::from_bytes([2; 32])),
        b"- nested\n",
    )
    .unwrap();
    assert_eq!(nested.path(), format!("pages/deep/{nested_filename}"));

    for invalid_target in ["/Root.md", "pages//Target.md", "Target.txt"] {
        let error = ProjectionRecoveryEvidence::new(
            invalid_target,
            ".Target.md.00000000000000000000000000000003.projection.recovery".to_owned(),
            Some(ContentDigest::from_bytes([3; 32])),
            b"- invalid\n",
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    let dir = scratch("projection-recovery-evidence-forgery");
    let graph = Graph::open(&dir);
    let valid_filename =
        ".Target.md.00000000000000000000000000000004.projection.recovery".to_owned();
    let forgeries = [
        ProjectionRecoveryEvidence {
            relative_path: format!("/pages/{valid_filename}"),
            filename: valid_filename.clone(),
            resource_id: Some(ContentDigest::from_bytes([4; 32])),
            digest: [0; 32],
            len: 0,
        },
        ProjectionRecoveryEvidence {
            relative_path: format!("journals/{valid_filename}"),
            filename: valid_filename.clone(),
            resource_id: Some(ContentDigest::from_bytes([4; 32])),
            digest: [0; 32],
            len: 0,
        },
        ProjectionRecoveryEvidence {
            relative_path: "pages/.Wrong.md.00000000000000000000000000000004.projection.recovery"
                .to_owned(),
            filename: ".Wrong.md.00000000000000000000000000000004.projection.recovery".to_owned(),
            resource_id: Some(ContentDigest::from_bytes([4; 32])),
            digest: [0; 32],
            len: 0,
        },
    ];
    for evidence in &forgeries {
        let error = graph
            .read_projection_recovery_evidence("pages/Target.md", evidence)
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn projection_replacement_binds_original_identity_and_rediscovers_late_stale_writes() {
    let dir = scratch("projection-stale-handle-evidence");
    let path = dir.join("pages/Projection.md");
    fs::write(&path, b"- base\n").unwrap();
    let stale_handle = fs::OpenOptions::new().write(true).open(&path).unwrap();
    let graph = Graph::open(&dir);

    PROJECTION_STALE_RECOVERY_WRITE.with(|write| {
        *write.borrow_mut() = Some((stale_handle, b"- late stale handle\n".to_vec()));
    });
    let proof = graph
        .write_projection_exact("pages/Projection.md", Some(b"- base\n"), b"- target\n")
        .unwrap();
    let evidence = proof
        .recovery_evidence()
        .first()
        .expect("replacement must retain its displaced inode");

    assert_eq!(fs::read(&path).unwrap(), b"- target\n");
    assert_eq!(evidence.len(), b"- base\n".len() as u64);
    assert_eq!(
        evidence.digest(),
        &<[u8; 32]>::from(Sha256::digest(b"- base\n"))
    );
    assert_eq!(
        graph
            .read_projection_recovery_evidence("pages/Projection.md", evidence)
            .unwrap(),
        b"- late stale handle\n"
    );
    let enumerated = graph
        .projection_recovery_evidence("pages/Projection.md")
        .unwrap();
    let rediscovered = enumerated
        .iter()
        .find(|candidate| candidate.filename() == evidence.filename())
        .expect("retained evidence must be discoverable after proof");
    assert_eq!(
        rediscovered.digest(),
        &<[u8; 32]>::from(Sha256::digest(b"- late stale handle\n"))
    );
    assert_eq!(rediscovered.len(), b"- late stale handle\n".len() as u64);
    assert!(dir.join(evidence.path()).exists());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn projection_write_proof_catalogs_all_retained_displacements_canonically() {
    let dir = scratch("projection-multiple-retained-evidence");
    let path = dir.join("pages/Projection.md");
    fs::write(&path, b"- base\n").unwrap();
    let graph = Graph::open(&dir);

    let first = graph
        .write_projection_exact("pages/Projection.md", Some(b"- base\n"), b"- first\n")
        .unwrap();
    assert_eq!(first.recovery_evidence().len(), 1);
    let second = graph
        .write_projection_exact("pages/Projection.md", Some(b"- first\n"), b"- second\n")
        .unwrap();
    assert_eq!(second.recovery_evidence().len(), 2);
    assert!(second
        .recovery_evidence()
        .windows(2)
        .all(|pair| pair[0].filename() < pair[1].filename()));
    assert!(second
        .recovery_evidence()
        .iter()
        .any(|evidence| evidence.digest() == &<[u8; 32]>::from(Sha256::digest(b"- base\n"))));
    assert!(second
        .recovery_evidence()
        .iter()
        .any(|evidence| evidence.digest() == &<[u8; 32]>::from(Sha256::digest(b"- first\n"))));
    assert_eq!(fs::read(&path).unwrap(), b"- second\n");

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn retained_displacement_digest_drift_is_rediscovered_without_deletion() {
    let dir = scratch("projection-late-post-proof-write");
    let path = dir.join("pages/Projection.md");
    fs::write(&path, b"- base\n").unwrap();
    let mut stale_handle = fs::OpenOptions::new().write(true).open(&path).unwrap();
    let graph = Graph::open(&dir);

    let proof = graph
        .write_projection_exact("pages/Projection.md", Some(b"- base\n"), b"- target\n")
        .unwrap();
    let first = proof
        .recovery_evidence()
        .first()
        .expect("replacement must retain evidence");
    assert_eq!(
        first.digest(),
        &<[u8; 32]>::from(Sha256::digest(b"- base\n"))
    );

    stale_handle.set_len(0).unwrap();
    stale_handle.rewind().unwrap();
    stale_handle.write_all(b"- late after proof\n").unwrap();
    stale_handle.sync_all().unwrap();
    let later = graph
        .projection_recovery_evidence("pages/Projection.md")
        .unwrap();
    let rediscovered = later
        .iter()
        .find(|evidence| evidence.filename() == first.filename())
        .expect("retained inode must remain discoverable");
    assert_eq!(
        rediscovered.digest(),
        &<[u8; 32]>::from(Sha256::digest(b"- late after proof\n"))
    );
    assert_ne!(rediscovered.digest(), first.digest());
    assert!(dir.join(rediscovered.path()).is_file());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn projection_exact_never_clobbers_pre_publish_or_proves_post_publish_changes() {
    let dir = scratch("projection-exact-reread");
    let path = dir.join("pages/Projection.md");
    fs::write(&path, b"- base\n").unwrap();
    let graph = Graph::open(&dir);

    PROJECTION_PUBLICATION_RACE_REPLACEMENT.with(|replacement| {
        *replacement.borrow_mut() = Some(b"- external race\n".to_vec());
    });
    assert!(graph
        .write_projection_exact("pages/Projection.md", Some(b"- base\n"), b"- target\n")
        .is_err());
    assert_eq!(fs::read(&path).unwrap(), b"- external race\n");
    assert!(
        projection_recovery_bytes(&dir.join("pages")).is_empty(),
        "pre-displacement race created recovery authority for an uncaptured inode"
    );
    assert!(!graph.recent_writes.lock().unwrap().contains_key(&path));

    fs::write(&path, b"- base again\n").unwrap();
    graph.warm_cache();
    let generation = graph.cache_generation();
    PROJECTION_POST_PUBLISH_REPLACEMENT.with(|replacement| {
        *replacement.borrow_mut() = Some(b"- changed after publish\n".to_vec());
    });
    assert!(graph
        .write_projection_exact(
            "pages/Projection.md",
            Some(b"- base again\n"),
            b"- target again\n",
        )
        .is_err());
    assert_eq!(fs::read(&path).unwrap(), b"- base again\n");
    assert!(projection_recovery_paths(&dir.join("pages"))
        .iter()
        .any(|evidence| {
            evidence
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".projection.withdrawn"))
                && fs::read(evidence).unwrap() == b"- changed after publish\n"
        }));
    assert_eq!(graph.cache_generation(), generation);
    assert_eq!(
        graph.disk_revs.read().unwrap().get(&path).cloned(),
        Some(content_rev("- base again\n"))
    );
    assert!(!graph.recent_writes.lock().unwrap().contains_key(&path));
    assert!(graph.sync_file(&path).is_none());
    assert_eq!(graph.cache_generation(), generation);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn projection_boundary_race_is_rejected_before_displacement() {
    let dir = scratch("projection-boundary-race-evidence");
    let path = dir.join("pages/Projection.md");
    fs::write(&path, b"- base\n").unwrap();
    let graph = Graph::open(&dir);

    PROJECTION_PUBLICATION_RACE_REPLACEMENT.with(|replacement| {
        *replacement.borrow_mut() = Some(b"- displaced external\n".to_vec());
    });
    PROJECTION_AFTER_RETIRE_REPLACEMENT.with(|replacement| {
        *replacement.borrow_mut() = Some(b"- newer external\n".to_vec());
    });
    let result =
        graph.write_projection_exact("pages/Projection.md", Some(b"- base\n"), b"- target\n");

    assert!(result.is_err(), "a raced publication returned write proof");
    assert_eq!(fs::read(&path).unwrap(), b"- displaced external\n");
    assert!(projection_recovery_bytes(&dir.join("pages")).is_empty());
    PROJECTION_AFTER_RETIRE_REPLACEMENT.with(|replacement| {
        assert_eq!(
            replacement.borrow_mut().take(),
            Some(b"- newer external\n".to_vec())
        );
    });
    assert!(!graph.recent_writes.lock().unwrap().contains_key(&path));

    let _ = fs::remove_dir_all(&dir);
}

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
        // model.rs `capture_managed_text_entries` symlink arm.
        (
            "precheck.symlink",
            Error::new(
                ErrorKind::InvalidInput,
                "managed text entry is a symlink or reparse point: pages/Note.md",
            ),
        ),
        // `capture_retained_graph_text_identity_with_limits` two-pass equality.
        (
            "precheck.interrupted",
            Error::new(
                ErrorKind::Interrupted,
                "managed inventory changed during retained identity capture",
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
            initial_shadow_limit_error("peak build memory"),
        ),
        // `save_page`, retained-identity arm -- the F4 class.
        (
            "identity.changed_since_load",
            Error::new(
                ErrorKind::AlreadyExists,
                "existing page identity changed since load",
            ),
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
/// can be driven directly. `initial_shadow_limit_error` and
/// `managed_text_inventory_limit_error` are the two the save path calls when a
/// bound is exceeded; both are `precheck.limit`, and neither may become a
/// conflict.
#[test]
fn direct_save_precheck_helpers_produce_their_own_codes() {
    for error in [
        initial_shadow_limit_error("entries"),
        managed_text_inventory_limit_error("bytes"),
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

/// The symlink skip is scoped to the save-time admission capture. Importing
/// a graph into managed storage still refuses, because an import that
/// silently leaves out a file the user considers part of their graph is a
/// different and worse failure than a refused import.
#[cfg(unix)]
#[test]
fn the_shadow_import_capture_still_refuses_a_symlink() {
    use std::os::unix::fs::symlink;

    let dir = scratch("symlink-import-refuses");
    fs::write(dir.join("pages/Target.md"), b"- before\n").unwrap();
    symlink(dir.join("pages/Target.md"), dir.join("pages/Alias.md")).unwrap();
    let graph = Graph::open(&dir);
    let permit = graph.admit_managed_text_writer().unwrap();
    let error = match collect_initial_shadow_managed_inventory(&graph, &permit, true) {
        Ok(_) => panic!("the import capture must still refuse a symlink in scope"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{error}");
    assert!(
        error.to_string().contains("symlink or reparse point"),
        "{error}"
    );
    let _ = fs::remove_dir_all(&dir);
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
        .with(|charge| charge.set(Some(INITIAL_SHADOW_LIMITS.peak_build_bytes)));
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
        Some(INITIAL_SHADOW_LIMITS.peak_build_bytes),
        "creation must not consume the retained shadow capture hook"
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
fn missing_target_creation_ignores_the_retained_shadow_peak_limit() {
    let dir = scratch("missing-target-retained-shadow-limit");
    fs::write(dir.join("pages/Existing.md"), b"- existing\n").unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();

    GRAPH_TEXT_FIRST_CAPTURE_CHARGE_OVERRIDE
        .with(|charge| charge.set(Some(INITIAL_SHADOW_LIMITS.peak_build_bytes)));
    let target = dir.join("pages/Noncolliding Missing Target.md");
    graph
        .save_page(
            &direct_save_bench_new_page("Noncolliding Missing Target"),
            None,
        )
        .expect("ordinary creation must not consult the retained shadow peak bound");
    assert!(target.is_file(), "the admitted creation must publish bytes");
    assert_eq!(
        GRAPH_TEXT_FIRST_CAPTURE_CHARGE_OVERRIDE.with(Cell::take),
        Some(INITIAL_SHADOW_LIMITS.peak_build_bytes),
        "ordinary creation consumed the retained shadow capture hook"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// The retained-shadow raw-byte ceiling remains a sensible per-file stream
/// bound, but is not a cumulative graph-size cap. Exercise the production
/// census decision with tiny real files instead of a 512 MiB allocation.
#[test]
fn direct_creation_census_accepts_aggregate_above_retained_byte_limit() {
    let dir = scratch("creation-census-aggregate-stream-bound");
    fs::write(dir.join("pages/First.md"), b"123456").unwrap();
    fs::write(dir.join("pages/Second.md"), b"abcdef").unwrap();
    let graph = Graph::open(&dir);
    let permit = graph.admit_retained_managed_text_writer().unwrap();
    let limits = InitialShadowLimits {
        raw_bytes: 6,
        ..INITIAL_SHADOW_LIMITS
    };

    let files = graph
        .capture_direct_creation_census_with_limits(&permit, limits)
        .expect("aggregate streamed bytes may exceed the retained-byte ceiling");
    assert_eq!(files.len(), 2);
    let aggregate = fs::metadata(dir.join("pages/First.md")).unwrap().len()
        + fs::metadata(dir.join("pages/Second.md")).unwrap().len();
    assert!(aggregate > limits.raw_bytes);

    fs::write(dir.join("pages/Oversized.md"), b"1234567").unwrap();
    let error = graph
        .capture_direct_creation_census_with_limits(&permit, limits)
        .expect_err("the per-file streamed resource bound must remain enforced");
    assert!(error.to_string().contains("file raw bytes"), "{error}");
    let _ = fs::remove_dir_all(&dir);
}

/// The whole validation/publication path is target-local: no graph census,
/// retained capture, complete-index build, or parse work.
#[test]
fn missing_target_creation_has_zero_graph_census_shadow_or_parse_work() {
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

#[cfg(unix)]
#[test]
fn unrelated_creation_does_not_mutate_existing_hardlinks() {
    let dir = scratch("creation-hardlink-refusal");
    fs::create_dir_all(dir.join("arbitrary")).unwrap();
    let incumbent = dir.join("pages/Owner.md");
    let alias = dir.join("arbitrary/Alias.md");
    fs::write(&incumbent, b"- incumbent\n").unwrap();
    fs::hard_link(&incumbent, &alias).unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let target = dir.join("pages/Fresh Hardlink Check.md");
    graph
        .save_page(&direct_save_bench_new_page("Fresh Hardlink Check"), None)
        .expect("an unrelated no-replace creation need not rewrite existing aliases");
    assert_eq!(fs::read(&incumbent).unwrap(), b"- incumbent\n");
    assert_eq!(fs::read(&alias).unwrap(), b"- incumbent\n");
    assert!(target.exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn creation_refuses_portable_leaf_and_ancestor_aliases_without_mutation() {
    for (label, configured, alias) in [
        ("case", "Pages", "pages"),
        ("nfc", "Caf\u{e9}Pages", "Cafe\u{301}Pages"),
    ] {
        let dir = scratch(&format!("creation-portable-ancestor-{label}"));
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq/config.edn"),
            format!("{{:pages-directory \"{configured}\"}}\n"),
        )
        .unwrap();
        fs::create_dir_all(dir.join(alias)).unwrap();
        let incumbent = dir.join(alias).join("Incumbent.md");
        fs::write(&incumbent, b"- incumbent\n").unwrap();
        let graph = Graph::open(&dir);
        graph.warm_cache();
        let target = dir.join(configured).join("Fresh.md");
        let error = graph
            .save_page(&direct_save_bench_new_page("Fresh"), None)
            .unwrap_err();
        assert_eq!(
            error.kind(),
            io::ErrorKind::AlreadyExists,
            "{label}: {error}"
        );
        assert_eq!(fs::read(&incumbent).unwrap(), b"- incumbent\n");
        assert!(!target.exists());
        assert!(!dir.join(configured).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    for (label, incumbent_name, requested) in [
        ("case", "leaf.md", "Leaf"),
        ("nfc", "Caf\u{e9}.md", "Cafe\u{301}"),
    ] {
        let dir = scratch(&format!("creation-portable-leaf-{label}"));
        let incumbent = dir.join("pages").join(incumbent_name);
        fs::write(&incumbent, b"- incumbent\n").unwrap();
        let graph = Graph::open(&dir);
        graph.warm_cache();
        let error = graph
            .save_page(&direct_save_bench_new_page(requested), None)
            .unwrap_err();
        assert_eq!(
            error.kind(),
            io::ErrorKind::AlreadyExists,
            "{label}: {error}"
        );
        assert_eq!(fs::read(&incumbent).unwrap(), b"- incumbent\n");
        let _ = fs::remove_dir_all(&dir);
    }
}

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
    MANAGED_WRITE_BEFORE_MUTATION.with(|hook| {
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
    MANAGED_WRITE_BEFORE_MUTATION.with(|hook| {
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
    MANAGED_WRITE_BEFORE_MUTATION.with(|hook| {
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
    MANAGED_WRITE_BEFORE_MUTATION.with(|hook| {
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
    INITIAL_SHADOW_REVALIDATION_RACE.with(|hook| {
        let other = dir.join("pages/Other.md");
        *hook.borrow_mut() = Some(Box::new(move || fs::write(&other, b"- other, pulled in\n")));
    });

    let mut page = graph.load_by_path("pages/Target.md").unwrap().unwrap();
    let base = page.rev.clone().expect("loaded page carries its revision");
    page.blocks[0].raw = "saved during sync activity".into();
    graph
        .save_page(&page, Some(&base))
        .expect("a sync client touching an unrelated file must not fail this save");
    INITIAL_SHADOW_REVALIDATION_RACE.with(|hook| {
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
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "Direct query benchmark projection did not converge"
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
    let result = graph.run_query_bounded(query, 20_000, 32 * 1024 * 1024);
    std::hint::black_box((result.total, result.exceeded));
    started.elapsed()
}

fn direct_query_bench_report(
    class: &str,
    phase: &str,
    samples: &mut [Duration],
    pages: usize,
    blocks: usize,
    indexed_reads: u64,
) {
    samples.sort();
    let ms = |duration: Duration| duration.as_secs_f64() * 1_000.0;
    println!(
        "b4_query class={class} phase={phase} median_ms={:.6} p95_ms={:.6} max_ms={:.6} rounds={} pages={pages} blocks={blocks} indexed_reads={indexed_reads}",
        ms(samples[samples.len() / 2]),
        ms(samples[samples.len() * 95 / 100]),
        ms(samples[samples.len() - 1]),
        samples.len(),
    );
}

fn direct_query_bench_cache_keys(graph: &Graph) -> std::collections::BTreeSet<String> {
    graph
        .derived_cache
        .read()
        .unwrap()
        .as_ref()
        .map(|cache| cache.results.keys().cloned().collect())
        .unwrap_or_default()
}

fn direct_query_bench_prime_invalidation(graph: &Graph) -> std::collections::BTreeSet<String> {
    *graph.derived_cache.write().unwrap() = None;
    for query in [
        "(task TODO)",
        "\"b4-unrelated-before\"",
        "\"b4-never-present\"",
        "(page-ref \"B4 Measurement Target\")",
    ] {
        std::hint::black_box(graph.run_query_bounded(query, 20_000, 32 * 1024 * 1024));
    }
    let keys = direct_query_bench_cache_keys(graph);
    assert_eq!(keys.len(), 4, "the invalidation probe must seed four memos");
    keys
}

fn direct_query_bench_report_invalidation(
    edit: &str,
    before: &std::collections::BTreeSet<String>,
    after: &std::collections::BTreeSet<String>,
    generation_before: u64,
    generation_after: u64,
) {
    let retained = before.intersection(after).count();
    let evicted = before.difference(after).count();
    println!(
        "b4_invalidation edit={edit} before={} retained={retained} evicted={evicted} cache_gen_before={generation_before} cache_gen_after={generation_after}",
        before.len(),
    );
}

/// B4 step 0 measurement. This release-only ignored benchmark compares memo
/// hits with invalidated evaluation for one representative of each sequencing
/// class, measures projection readiness immediately after a Direct delta, and
/// records scoped memo retention. Point it at a copied graph with
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
        std::hint::black_box(graph.run_query_bounded(query, 20_000, 32 * 1024 * 1024));
        let mut memo = (0..rounds)
            .map(|_| direct_query_bench_sample(&graph, query))
            .collect::<Vec<_>>();
        direct_query_bench_report(class, "memo", &mut memo, pages, blocks, 0);

        let indexed_before = graph.direct_projection_indexed_reads_test();
        let mut invalidated = Vec::with_capacity(rounds);
        for sample in 0..rounds {
            serial += 1;
            direct_query_bench_edit(&graph, serial);
            wait_for_direct_query_projection(&graph);
            *graph.derived_cache.write().unwrap() = None;
            graph.reset_direct_projection_candidate_probe_test();
            let fallback_before = graph.direct_projection_fallback_reads_test();
            let candidate_before = graph.direct_projection_indexed_reads_test();
            let elapsed = direct_query_bench_sample(&graph, query);
            let candidate_queries_completed = graph
                .direct_projection_indexed_reads_test()
                .saturating_sub(candidate_before);
            let fallback_reads = graph
                .direct_projection_fallback_reads_test()
                .saturating_sub(fallback_before);
            let full_graph_evaluations = crate::query::full_graph_query_evaluations();
            let evaluated_pages = graph
                .direct_projection_candidate_evaluated_paths_test()
                .len();
            println!(
                "b4_query_sample class={class} run={} sample={} candidateQueriesCompleted={candidate_queries_completed} fallbackReads={fallback_reads} fullGraphEvaluations={full_graph_evaluations} evaluatedPages={evaluated_pages} medianMs={:.6}",
                std::env::var("TINE_B4_QUERY_BENCH_RUN").unwrap_or_else(|_| "1".into()),
                sample + 1,
                elapsed.as_secs_f64() * 1_000.0,
            );
            invalidated.push(elapsed);
        }
        let indexed_reads = graph
            .direct_projection_indexed_reads_test()
            .saturating_sub(indexed_before);
        direct_query_bench_report(
            class,
            "invalidated_ready",
            &mut invalidated,
            pages,
            blocks,
            indexed_reads,
        );
    }

    let mut ready_hits = 0_usize;
    let mut ready_misses = 0_usize;
    let indexed_before = graph.direct_projection_indexed_reads_test();
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
        let candidate_before = graph.direct_projection_indexed_reads_test();
        let fallback_before = graph.direct_projection_fallback_reads_test();
        let actual = graph.run_query_bounded("(task TODO)", 20_000, 32 * 1024 * 1024);
        let oracle_equal = (actual.total, actual.exceeded) == (oracle.total, oracle.exceeded)
            && serde_json::to_vec(actual.groups.as_ref()).unwrap()
                == serde_json::to_vec(&oracle.groups).unwrap();
        println!(
            "b4_readiness save={}-{} generation={generation} immediate_ready={immediate_ready} ready_latency_ms={ready_latency_ms:.6} terminal_event=worker_apply_complete candidate_reads={} fallback_reads={} oracle_equal={oracle_equal}",
            std::env::var("TINE_B4_QUERY_BENCH_RUN").unwrap_or_else(|_| "1".into()),
            save + 1,
            graph.direct_projection_indexed_reads_test().saturating_sub(candidate_before),
            graph.direct_projection_fallback_reads_test().saturating_sub(fallback_before),
        );
    }
    let indexed_reads = graph
        .direct_projection_indexed_reads_test()
        .saturating_sub(indexed_before);
    direct_query_bench_report(
        "sparse_task",
        "data_rev_immediate",
        &mut immediate,
        pages,
        blocks,
        indexed_reads,
    );
    println!(
        "b4_projection_hit_rate samples={} ready_hits={ready_hits} ready_misses={ready_misses} indexed_reads={indexed_reads}",
        ready_hits + ready_misses,
    );

    let before = direct_query_bench_prime_invalidation(&graph);
    let generation_before = graph.cache_generation();
    let mut unrelated = graph
        .load_by_path("pages/B4 Measurement Unrelated.md")
        .unwrap()
        .unwrap();
    unrelated.blocks[0].raw = "b4-unrelated-after".into();
    graph
        .save_page(&unrelated, unrelated.rev.as_deref())
        .unwrap();
    let after = direct_query_bench_cache_keys(&graph);
    direct_query_bench_report_invalidation(
        "content_only",
        &before,
        &after,
        generation_before,
        graph.cache_generation(),
    );
    assert_eq!((after.len(), before.difference(&after).count()), (2, 2));
    wait_for_direct_query_projection(&graph);

    let before = direct_query_bench_prime_invalidation(&graph);
    let generation_before = graph.cache_generation();
    let mut unrelated = graph
        .load_by_path("pages/B4 Measurement Unrelated.md")
        .unwrap()
        .unwrap();
    unrelated.pre_block = Some("alias:: B4 Measurement Alias\n".into());
    graph
        .save_page(&unrelated, unrelated.rev.as_deref())
        .unwrap();
    let after = direct_query_bench_cache_keys(&graph);
    direct_query_bench_report_invalidation(
        "alias_change",
        &before,
        &after,
        generation_before,
        graph.cache_generation(),
    );
    assert!(after.is_empty());
    wait_for_direct_query_projection(&graph);

    let before = direct_query_bench_prime_invalidation(&graph);
    let generation_before = graph.cache_generation();
    let mut new_page = direct_save_bench_new_page("B4 Measurement New Page");
    new_page.blocks[0].id = Uuid::from_u128(0xb400_0000_0000_0000_0000_0000_0000_0001).to_string();
    graph.save_page(&new_page, None).unwrap();
    let after = direct_query_bench_cache_keys(&graph);
    direct_query_bench_report_invalidation(
        "page_set_change",
        &before,
        &after,
        generation_before,
        graph.cache_generation(),
    );
    assert!(after.is_empty());
    wait_for_direct_query_projection(&graph);

    let before = direct_query_bench_prime_invalidation(&graph);
    graph.derived_cache.write().unwrap().as_mut().unwrap().today -= 1;
    let generation_before = graph.cache_generation();
    let mut unrelated = graph
        .load_by_path("pages/B4 Measurement Unrelated.md")
        .unwrap()
        .unwrap();
    unrelated.blocks[0].raw = "b4-unrelated-day-rollover".into();
    graph
        .save_page(&unrelated, unrelated.rev.as_deref())
        .unwrap();
    let after = direct_query_bench_cache_keys(&graph);
    direct_query_bench_report_invalidation(
        "day_rollover_simulated",
        &before,
        &after,
        generation_before,
        graph.cache_generation(),
    );
    assert!(after.is_empty());
    wait_for_direct_query_projection(&graph);

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
fn projection_exact_updates_warm_cache_once_and_suppresses_watcher_echo() {
    let dir = scratch("projection-exact-cache");
    let path = dir.join("pages/Projection.md");
    fs::write(&path, b"- before\n").unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let generation = graph.cache_generation();

    let proof = graph
        .write_projection_exact("pages/Projection.md", Some(b"- before\n"), b"- after\n")
        .unwrap();
    assert_eq!(proof.bytes(), b"- after\n");
    assert_eq!(graph.cache_generation(), generation + 1);
    assert_eq!(
        graph
            .load_by_path("pages/Projection.md")
            .unwrap()
            .unwrap()
            .blocks[0]
            .raw,
        "after"
    );
    assert_eq!(
        graph.disk_revs.read().unwrap().get(&path).cloned(),
        Some(content_rev("- after\n"))
    );
    assert!(graph.sync_file(&path).is_none());
    assert!(!graph.recent_writes.lock().unwrap().contains_key(&path));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn projection_sync_failure_requires_recovery_and_stale_write_remains_conflict() {
    let dir = scratch("projection-exact-durability");
    let path = dir.join("pages/Projection.md");
    fs::write(&path, b"- base\n").unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let generation = graph.cache_generation();
    let revisions = graph.disk_revs.read().unwrap().clone();

    FAIL_NEXT_PROJECTION_DIRECTORY_SYNC.with(|fail| fail.set(true));
    assert!(graph
        .write_projection_exact("pages/Projection.md", Some(b"- base\n"), b"- target\n")
        .is_err());
    assert_eq!(fs::read(&path).unwrap(), b"- target\n");
    assert_eq!(graph.cache_generation(), generation);
    assert_eq!(
        *graph.disk_revs.read().unwrap(),
        revisions,
        "failed publication must not install baseline authority"
    );
    assert_eq!(
        graph
            .load_by_path("pages/Projection.md")
            .unwrap()
            .unwrap()
            .blocks[0]
            .raw,
        "target"
    );
    assert_eq!(graph.cache_generation(), generation);
    assert!(!graph.recent_writes.lock().unwrap().contains_key(&path));

    assert!(graph
        .write_projection_exact("pages/Projection.md", Some(b"- base\n"), b"- target\n")
        .is_err());
    FAIL_NEXT_PROJECTION_DIRECTORY_SYNC.with(|fail| fail.set(true));
    assert!(graph
        .write_projection_exact("pages/Projection.md", Some(b"- target\n"), b"- target\n")
        .is_err());
    FAIL_NEXT_PROJECTION_DIRECTORY_SYNC.with(|fail| {
        assert!(
            fail.replace(false),
            "an already-visible target must not be synced or accepted by ordinary write"
        );
    });

    FAIL_NEXT_PROJECTION_DIRECTORY_SYNC.with(|fail| fail.set(true));
    assert!(graph
        .recover_projection_exact("pages/Projection.md", b"- target\n")
        .is_err());
    let proof = graph
        .recover_projection_exact("pages/Projection.md", b"- target\n")
        .unwrap();
    assert_eq!(proof.path(), "pages/Projection.md");
    assert_eq!(proof.bytes(), b"- target\n");
    assert_eq!(
        graph.cache_generation(),
        generation + 1,
        "only successful exact recovery may publish cache authority"
    );
    assert!(graph.sync_file(&path).is_none());
    assert!(graph
        .recover_projection_exact("pages/Projection.md", b"- wrong\n")
        .is_err());
    assert_eq!(fs::read(&path).unwrap(), b"- target\n");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn projection_exact_creates_and_recovers_nested_parent_chain() {
    let dir = scratch("projection-exact-nested-parent");
    let path = dir.join("pages/nested/deeper/Projection.md");
    let graph = Graph::open(&dir);

    let proof = graph
        .write_projection_exact("pages/nested/deeper/Projection.md", None, b"- nested\n")
        .unwrap();
    assert_eq!(proof.bytes(), b"- nested\n");
    assert_eq!(fs::read(&path).unwrap(), b"- nested\n");
    assert_eq!(
        graph
            .recover_projection_exact("pages/nested/deeper/Projection.md", b"- nested\n")
            .unwrap()
            .bytes(),
        b"- nested\n"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn projection_input_absence_and_publication_cover_nested_unicode_layouts() {
    let dir = scratch("projection-input-nested-unicode");
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq/config.edn"),
        "{:pages-directory \"archive/pages\"\n\
              :journals-directory \"archive/journals\"}\n",
    )
    .unwrap();
    let graph = Graph::open(&dir);
    let cases = [
        (
            "archive/pages/題材/深い/ノート.md",
            b"- configured Unicode page\n".as_slice(),
        ),
        (
            "archive/pagesish/自由/Elsewhere.md",
            b"- graph-wide nonstandard page\n".as_slice(),
        ),
    ];

    for (relative, expected) in cases {
        let path = ManagedPath::parse(relative).unwrap();
        assert_eq!(graph.read_projection_input(&path).unwrap(), None);
        assert!(
            !dir.join(relative).parent().unwrap().exists(),
            "input capture synthesized missing parents for {relative}"
        );

        let proof = graph
            .write_projection_exact(relative, None, expected)
            .unwrap();
        assert_eq!(proof.path(), relative);
        assert_eq!(fs::read(dir.join(relative)).unwrap(), expected);
        assert_eq!(
            graph.read_projection_input(&path).unwrap().as_deref(),
            Some(expected)
        );
    }

    let independent = Graph::open(&dir);
    for relative in cases.map(|(relative, _)| relative) {
        assert!(
            independent
                .list_pages()
                .iter()
                .any(|entry| entry.rel_path == relative),
            "independent graph scan did not observe {relative}"
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

#[test]
fn projection_retry_resumes_after_synced_partial_parent_chain() {
    let dir = scratch("projection-partial-parent-retry");
    let relative = "pages/created/synced/deep/Projection.md";
    let path = dir.join(relative);
    let graph = Graph::open(&dir);

    FAIL_AFTER_PROJECTION_PARENT_SYNC.with(|fail| fail.set(true));
    assert!(graph
        .write_projection_exact(relative, None, b"- retry target\n")
        .is_err());
    assert!(dir.join("pages/created").is_dir());
    assert!(!dir.join("pages/created/synced").exists());
    assert!(!path.exists());

    let proof = graph
        .write_projection_exact(relative, None, b"- retry target\n")
        .unwrap();
    assert_eq!(proof.bytes(), b"- retry target\n");
    assert_eq!(fs::read(&path).unwrap(), b"- retry target\n");

    let _ = fs::remove_dir_all(&dir);
}

/// Count the directory barriers one projection write initiates.
///
/// `BarrierSession` is a per-thread attribution channel, and
/// `write_projection_exact` runs entirely on the calling thread, so this
/// sees exactly this write's barriers even while other tests run.
fn projection_write_directory_barriers(graph: &Graph, relative: &str, bytes: &[u8]) -> u64 {
    let session = crate::durability_counters::BarrierSession::begin();
    graph
        .write_projection_exact(relative, None, bytes)
        .expect("the projection write under measurement must succeed");
    let counted = session
        .counts()
        .get(crate::durability_counters::Barrier::Directory);
    crate::durability_counters::BarrierSession::detach_current_thread();
    counted
}

#[test]
fn a_same_directory_move_takes_one_directory_barrier() {
    use crate::durability_counters::{Barrier, BarrierSession};

    let dir = scratch("same-leaf-move-barrier");
    let graph = Graph::open(&dir);
    graph
        .write_projection_exact("pages/Before.md", None, b"- moving\n")
        .unwrap();

    let turn = ProjectionTurnBarrierScope::begin().unwrap();
    let session = BarrierSession::begin();
    graph
        .remove_projection_exact("pages/Before.md", b"- moving\n")
        .unwrap();
    graph
        .write_projection_exact("pages/After.md", None, b"- moving\n")
        .unwrap();
    let before_finish = session.counts().get(Barrier::Directory);
    turn.finish().unwrap();
    let directories = session.counts().get(Barrier::Directory);
    BarrierSession::detach_current_thread();

    assert_eq!(
        before_finish, 0,
        "managed reconstructible leaf barriers must remain deferred until turn finish"
    );
    assert_eq!(
        directories.saturating_sub(before_finish),
        1,
        "the remove and create must close with one shared reconstructible leaf barrier"
    );
    assert!(!dir.join("pages/Before.md").exists());
    assert_eq!(fs::read(dir.join("pages/After.md")).unwrap(), b"- moving\n");
    let _ = fs::remove_dir_all(&dir);
}

/// **The chain-flush invariant** (`docs/storage-sync-contract.md` §2.10a-i).
///
/// A projection operation changes the entry list of exactly one directory —
/// the chain leaf — so it takes its directory barrier there and nowhere
/// else. Path depth is therefore free. Before the 2026-08-26 cut the write
/// path walked the chain leaf-to-root on every write, rename and preflight,
/// so a namespaced page silently paid a device round trip per path
/// component; this assertion fails on that code.
#[test]
fn a_projection_operation_flushes_one_directory_whatever_its_depth() {
    let dir = scratch("projection-chain-flush-depth");
    fs::create_dir_all(dir.join("pages/one/two/three")).unwrap();
    let graph = Graph::open(&dir);

    let shallow = projection_write_directory_barriers(&graph, "pages/Shallow.md", b"- shallow\n");
    let deep =
        projection_write_directory_barriers(&graph, "pages/one/two/three/Deep.md", b"- deep\n");

    assert!(
        shallow > 0,
        "a projection write must still take its own directory barrier"
    );
    assert_eq!(
            deep, shallow,
            "chain depth must not cost directory barriers: the three-deep page took {deep}              and the one-deep page took {shallow}. Only the leaf's entry list changed, and              an ancestor entry that is already durable cannot be un-durabled by any in-scope              failure."
        );
    assert_eq!(
        fs::read(dir.join("pages/one/two/three/Deep.md")).unwrap(),
        b"- deep\n"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// The other half of the invariant: an ancestor Tine *creates* still gets a
/// barrier — exactly one, in the parent that now holds its name, taken by
/// `create_projection_chain_component` at the moment of creation rather
/// than by a chain walk afterwards.
#[test]
fn a_created_projection_ancestor_costs_exactly_one_extra_barrier() {
    let dir = scratch("projection-created-ancestor-barrier");
    let graph = Graph::open(&dir);

    let existing =
        projection_write_directory_barriers(&graph, "pages/Existing.md", b"- existing\n");
    let one_new = projection_write_directory_barriers(&graph, "pages/alpha/One.md", b"- one\n");
    let two_new =
        projection_write_directory_barriers(&graph, "pages/beta/gamma/Two.md", b"- two\n");

    assert_eq!(
            one_new,
            existing + 1,
            "creating one ancestor must cost exactly one barrier more than writing into an              existing chain ({one_new} vs {existing})"
        );
    assert_eq!(
            two_new,
            existing + 2,
            "creating two ancestors must cost exactly two barriers more ({two_new} vs              {existing})"
        );
    assert_eq!(
        fs::read(dir.join("pages/beta/gamma/Two.md")).unwrap(),
        b"- two\n"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// The second reachable post-crash state of the same crash point that
/// `projection_retry_resumes_after_synced_partial_parent_chain` covers.
///
/// `mkdir` and its parent's barrier are not atomic, so a crash between them
/// leaves the ancestor either absent (that test) or present — which on the
/// next boot is indistinguishable from an ancestor that was always there.
/// Retry must converge from this state too, and must not re-flush the
/// surviving ancestor to do it.
#[test]
fn projection_retry_converges_when_a_created_ancestor_survived_the_crash() {
    let dir = scratch("projection-surviving-ancestor-retry");
    let relative = "pages/created/synced/deep/Projection.md";
    let path = dir.join(relative);
    let graph = Graph::open(&dir);

    // The state a crash between `mkdir` and its barrier can leave behind.
    fs::create_dir_all(path.parent().unwrap()).unwrap();

    let barriers = projection_write_directory_barriers(&graph, relative, b"- retry target\n");
    assert_eq!(fs::read(&path).unwrap(), b"- retry target\n");

    let flat = scratch("projection-surviving-ancestor-baseline");
    let baseline = projection_write_directory_barriers(
        &Graph::open(&flat),
        "pages/Projection.md",
        b"- retry target\n",
    );
    assert_eq!(
            barriers, baseline,
            "a surviving ancestor is an ordinary existing directory and must cost nothing              ({barriers} vs {baseline})"
        );

    let _ = fs::remove_dir_all(&flat);
    let _ = fs::remove_dir_all(&dir);
}

/// The chain-*creation* half of the anonymized-corpus acceptance gate for
/// the 2026-08-26 chain-flush cut. Its sibling,
/// `sync_runtime::tests::managed_nested_projection_lands_on_a_real_graph_copy`,
/// drives the managed save and cross-page move on the same corpus.
///
/// Synthetic fixtures are generated from our own model of a graph, so this
/// runs the case the model keeps getting wrong at real scale: a page whose
/// projection path needs a directory that does not exist yet — the first
/// journal of a month, or a namespace page creating `pages/foo/`.
///
/// ```text
/// TINE_MS_AUDIT_GRAPH_COPY=/path/to/graph/copy \
///   cargo test -p tine-core --lib \
///   projection_creates_a_nested_parent_on_a_real_graph_copy -- --ignored --nocapture
/// ```
#[test]
#[ignore = "manual acceptance gate: chain creation on a real graph copy"]
fn projection_creates_a_nested_parent_on_a_real_graph_copy() {
    let source = PathBuf::from(
        std::env::var("TINE_MS_AUDIT_GRAPH_COPY")
            .expect("set TINE_MS_AUDIT_GRAPH_COPY to a disposable graph copy"),
    );
    assert!(
        fs::symlink_metadata(&source)
            .expect("the graph copy must be readable")
            .file_type()
            .is_dir(),
        "the graph copy must name a real directory, not a symlink"
    );

    let dir = scratch("projection-real-graph-chain-creation");
    let _ = fs::remove_dir_all(&dir);
    copy_real_graph_tree(&source, &dir);
    assert!(dir.join("pages").is_dir() && dir.join("journals").is_dir());
    let graph = Graph::open(&dir);

    let flat = projection_write_directory_barriers(
        &graph,
        "pages/Chain Flush Gate.md",
        b"- flat acceptance\n",
    );
    let created = projection_write_directory_barriers(
        &graph,
        "pages/Chain Flush Gate/Nested.md",
        b"- created acceptance\n",
    );

    assert_eq!(
        created,
        flat + 1,
        "creating one parent on the real corpus must cost exactly one barrier more \
             ({created} vs {flat})"
    );
    assert_eq!(
        fs::read(dir.join("pages/Chain Flush Gate/Nested.md")).unwrap(),
        b"- created acceptance\n"
    );
    eprintln!(
        "real-graph projection: flat={flat} dir_fsync, creating-one-parent={created} dir_fsync"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// Copy a disposable real-graph tree without following symlinks.
fn copy_real_graph_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    let mut pending = vec![(from.to_path_buf(), to.to_path_buf())];
    while let Some((source, destination)) = pending.pop() {
        for entry in fs::read_dir(&source).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            let target = destination.join(entry.file_name());
            if kind.is_dir() {
                fs::create_dir_all(&target).unwrap();
                pending.push((entry.path(), target));
            } else if kind.is_file() {
                fs::copy(entry.path(), &target).unwrap();
            }
        }
    }
}

/// A cross-directory move syncs BOTH the source and the destination chain
/// (`managed_move_noreplace_validated`). Each of those is one barrier on
/// the directory whose entry list actually changed — the source loses a
/// name, the destination gains one — so the depth of either chain is free.
///
/// This drives the Direct Files path deliberately: `rename_file_to_page`
/// is a real user operation on a Direct graph, and the chain helpers are
/// shared by both storage modes.
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

#[test]
fn missing_nested_tombstone_completes_without_creating_parents() {
    let dir = scratch("projection-missing-nested-tombstone");
    let relative = "pages/deleted/deep/Gone.md";
    let path = ManagedPath::parse(relative).unwrap();
    let graph = Graph::open(&dir);

    assert_eq!(graph.read_projection_input(&path).unwrap(), None);
    let proof = graph.confirm_removed_projection_exact(relative).unwrap();
    assert_eq!(proof.path(), relative);
    assert!(proof.bytes().is_empty());
    assert!(!dir.join(relative).parent().unwrap().exists());
    assert!(!dir.join(relative).exists());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn missing_nested_tombstone_refuses_parent_and_target_appearance_before_proof() {
    let dir = scratch("projection-missing-tombstone-race");
    let relative = "pages/deleted/deep/Gone.md";
    let target = dir.join(relative);
    let graph = Graph::open(&dir);

    PROJECTION_POST_PUBLISH_COLLISION.with(|hook| {
        let target = target.clone();
        *hook.borrow_mut() = Some(Box::new(move || {
            fs::create_dir_all(target.parent().unwrap())?;
            fs::write(target, b"- appeared externally\n")
        }));
    });
    assert!(graph.confirm_removed_projection_exact(relative).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"- appeared externally\n");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn projection_missing_capture_rejects_regular_file_intermediate() {
    let dir = scratch("projection-regular-intermediate");
    let blocker = dir.join("pages/blocker");
    fs::write(&blocker, b"retained ordinary file").unwrap();
    let relative = "pages/blocker/deep/Projection.md";
    let graph = Graph::open(&dir);

    assert!(graph
        .read_projection_input(&ManagedPath::parse(relative).unwrap())
        .is_err());
    assert!(graph
        .write_projection_exact(relative, None, b"- target\n")
        .is_err());
    assert_eq!(fs::read(&blocker).unwrap(), b"retained ordinary file");
    assert!(!dir.join(relative).exists());

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn projection_missing_capture_rejects_symlink_intermediate_without_escape() {
    use std::os::unix::fs::symlink;

    let dir = scratch("projection-symlink-intermediate");
    let outside = scratch("projection-symlink-intermediate-outside");
    symlink(&outside, dir.join("pages/linked")).unwrap();
    let relative = "pages/linked/deep/Projection.md";
    let graph = Graph::open(&dir);

    assert!(graph
        .read_projection_input(&ManagedPath::parse(relative).unwrap())
        .is_err());
    assert!(graph
        .write_projection_exact(relative, None, b"- target\n")
        .is_err());
    assert!(!outside.join("deep/Projection.md").exists());
    assert!(dir.join("pages/linked").symlink_metadata().is_ok());

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&outside);
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
        .read_projection_input(&ManagedPath::parse(relative).unwrap())
        .is_err());
    assert!(graph
        .write_projection_exact(relative, None, b"- target\n")
        .is_err());
    assert!(!outside.join("deep/Projection.md").exists());
    assert!(dir.join("pages/linked").symlink_metadata().is_ok());

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&outside);
}

#[test]
fn projection_exact_accepts_nested_configured_pages_and_journals_roots() {
    let dir = scratch("projection-configured-nested-roots");
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq/config.edn"),
        "{:pages-directory \"archive/pages\"\n\
              :journals-directory \"archive/journals\"}\n",
    )
    .unwrap();
    let graph = Graph::open(&dir);

    let page = graph
        .write_projection_exact(
            "archive/pages/topic/Projection.md",
            None,
            b"- nested page\n",
        )
        .unwrap();
    let journal = graph
        .write_projection_exact(
            "archive/journals/2026/07/23.org",
            None,
            b"* nested journal\n",
        )
        .unwrap();
    assert_eq!(page.path(), "archive/pages/topic/Projection.md");
    assert_eq!(journal.path(), "archive/journals/2026/07/23.org");
    assert_eq!(
        fs::read(dir.join("archive/pages/topic/Projection.md")).unwrap(),
        b"- nested page\n"
    );
    assert_eq!(
        fs::read(dir.join("archive/journals/2026/07/23.org")).unwrap(),
        b"* nested journal\n"
    );
    // No configured root owns these, but OG walks the whole graph directory
    // and rewrites a page wherever it already lives, so they are ordinary
    // graph text addressed at their exact spelling — never relocated under
    // `archive/pages`.
    for outside in ["archive/pagesish/Sibling.md", "pages/Default named.md"] {
        let projected = graph
            .write_projection_exact(outside, None, b"- outside\n")
            .unwrap();
        assert_eq!(projected.path(), outside);
        assert_eq!(fs::read(dir.join(outside)).unwrap(), b"- outside\n");
    }
    assert!(!dir.join("archive/pages/Sibling.md").exists());
    assert!(!dir.join("archive/pages/Default named.md").exists());

    // Containers outside the graph-text scope stay unaddressable.
    for refused in [
        "assets/Wrong.md",
        "logseq/bak/pages/Wrong.md",
        ".hidden/Wrong.md",
    ] {
        assert!(
            graph
                .write_projection_exact(refused, None, b"- wrong\n")
                .is_err(),
            "accepted {refused}"
        );
        assert!(!dir.join(refused).exists());
    }

    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn raw_managed_read_rejects_parent_retarget_and_file_replacement_after_open() {
    use std::os::unix::fs::symlink;

    let replacement_root = scratch("raw-managed-file-replacement");
    let replacement_path = replacement_root.join("pages/page.md");
    fs::write(&replacement_path, b"- original\n").unwrap();
    let replacement_graph = Graph::open(&replacement_root);
    let retired = replacement_root.join("pages/page.retired.md");
    MANAGED_INVENTORY_READ_RACE.with(|hook| {
        let replacement_path = replacement_path.clone();
        let retired = retired.clone();
        *hook.borrow_mut() = Some(Box::new(move || {
            fs::rename(&replacement_path, &retired)?;
            fs::write(&replacement_path, b"- replacement\n")
        }));
    });
    assert!(replacement_graph
        .read_raw_managed_text(&ManagedPath::parse("pages/page.md").unwrap())
        .is_err());
    assert_eq!(fs::read(&replacement_path).unwrap(), b"- replacement\n");

    let retarget_root = scratch("raw-managed-parent-retarget");
    let outside = scratch("raw-managed-parent-retarget-outside");
    fs::write(retarget_root.join("pages/page.md"), b"- retained\n").unwrap();
    fs::write(outside.join("page.md"), b"- outside\n").unwrap();
    let retarget_graph = Graph::open(&retarget_root);
    let moved = retarget_root.join("pages-retained");
    MANAGED_INVENTORY_READ_RACE.with(|hook| {
        let parent = retarget_root.join("pages");
        let outside = outside.clone();
        let moved = moved.clone();
        *hook.borrow_mut() = Some(Box::new(move || {
            fs::rename(&parent, &moved)?;
            symlink(&outside, &parent)
        }));
    });
    assert!(retarget_graph
        .read_raw_managed_text(&ManagedPath::parse("pages/page.md").unwrap())
        .is_err());
    assert_eq!(fs::read(outside.join("page.md")).unwrap(), b"- outside\n");

    let _ = fs::remove_dir_all(&replacement_root);
    let _ = fs::remove_file(retarget_root.join("pages"));
    let _ = fs::remove_dir_all(&retarget_root);
    let _ = fs::remove_dir_all(&outside);
}

#[test]
fn live_admission_snapshot_defers_late_enumeration_changes_to_the_fenced_feed() {
    let root = scratch("initial-shadow-race");
    fs::create_dir_all(root.join("pages/nested")).unwrap();
    fs::write(root.join("pages/nested/a.md"), b"- first\n").unwrap();
    let graph = Graph::open(&root);
    INITIAL_SHADOW_REVALIDATION_RACE.with(|hook| {
        let nested = root.join("pages/nested");
        *hook.borrow_mut() = Some(Box::new(move || {
            fs::remove_file(nested.join("a.md"))?;
            fs::write(nested.join("a.md"), b"- replaced\n")?;
            fs::write(nested.join("inserted.md"), b"- inserted\n")
        }));
    });
    let inventory = graph.initial_shadow_raw_managed_text_inventory().unwrap();
    assert_eq!(
        inventory,
        vec![(
            ManagedPath::parse("pages/nested/a.md").unwrap(),
            b"- first\n".to_vec(),
        )]
    );
    assert_eq!(
        fs::read(root.join("pages/nested/a.md")).unwrap(),
        b"- replaced\n"
    );
    assert_eq!(
        fs::read(root.join("pages/nested/inserted.md")).unwrap(),
        b"- inserted\n"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn initial_shadow_rejects_ambient_root_identity_replacement() {
    let root = scratch("initial-shadow-root-replacement");
    fs::write(root.join("Root.md"), b"- retained\n").unwrap();
    let retired = root.with_file_name(format!(
        "{}-retired",
        root.file_name().unwrap().to_string_lossy()
    ));
    let _ = fs::remove_dir_all(&retired);
    let graph = Graph::open(&root);
    INITIAL_SHADOW_REVALIDATION_RACE.with(|hook| {
        let root = root.clone();
        let retired = retired.clone();
        *hook.borrow_mut() = Some(Box::new(move || {
            fs::rename(&root, &retired)?;
            fs::create_dir_all(root.join("pages"))?;
            fs::create_dir_all(root.join("journals"))?;
            fs::write(root.join("Root.md"), b"- retained\n")
        }));
    });
    assert!(graph.initial_shadow_raw_managed_text_inventory().is_err());

    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&retired);
}

#[test]
fn initial_shadow_discovers_graph_wide_mixed_case_text_paths() {
    let root = scratch("initial-shadow-graph-wide");
    fs::create_dir_all(root.join("archive/client/deep")).unwrap();
    fs::write(root.join("Root.MD"), b"- root\n").unwrap();
    fs::write(
        root.join("archive/client/deep/Plan.Markdown"),
        b"- markdown\n",
    )
    .unwrap();
    fs::write(root.join("archive/client/deep/25-07-2026.ORG"), b"* org\n").unwrap();

    let graph = Graph::open(&root);
    reset_graph_text_admission_test_counters();
    let inventory = graph.initial_shadow_raw_managed_text_inventory().unwrap();
    assert_eq!(
        inventory
            .iter()
            .map(|(path, _)| path.as_str())
            .collect::<Vec<_>>(),
        vec![
            "Root.MD",
            "archive/client/deep/25-07-2026.ORG",
            "archive/client/deep/Plan.Markdown",
        ]
    );
    let counters = graph_text_admission_test_counters();
    assert_eq!(counters.builder_enumerations, 6);
    assert_eq!(counters.point_query_attempts, 0);
    assert_eq!(counters.parser_invocations, 3);
    assert_eq!(counters.index_map_insertions, 15);
    assert_eq!(counters.event_map_key_reads, 0);
    assert_eq!(counters.event_map_key_writes, 0);
    assert_eq!(counters.event_reverse_members, 0);
    assert!(counters.persistent_node_allocations > 0);
    assert_eq!(counters.persistent_payload_members, 6);

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn admission_snapshot_uses_parser_owned_semantics_and_preserves_creation_roots() {
    let root = scratch("admission-semantic");
    fs::create_dir_all(root.join("logseq")).unwrap();
    fs::write(
        root.join("logseq/config.edn"),
        "{:pages-directory \"create/pages\"\n\
              :journals-directory \"create/journals\"\n\
              :journal/file-name-format \"dd-MM-yyyy\"\n\
              :journal/page-title-format \"yyyy-MM-dd\"}\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("archive")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    fs::write(
        root.join("archive/Not-A-Date.Markdown"),
        b"title:: 25-07-2026\n\n- journal by title\n",
    )
    .unwrap();
    fs::write(root.join("journals/Plan.ORG"), b"* ordinary page\n").unwrap();

    let graph = Graph::open(&root);
    graph.initial_shadow_raw_managed_text_inventory().unwrap();
    let index = graph.guarded_graph_text_identity_index().unwrap();
    let journal = index
        .files_by_exact_path
        .get(&ManagedPath::parse("archive/Not-A-Date.Markdown").unwrap())
        .unwrap();
    assert_eq!(journal.semantic.name, "2026-07-25");
    assert_eq!(journal.semantic.kind, PageKind::Journal);
    assert_eq!(journal.format, Format::Md);
    let page = index
        .files_by_exact_path
        .get(&ManagedPath::parse("journals/Plan.ORG").unwrap())
        .unwrap();
    assert_eq!(page.semantic.name, "Plan");
    assert_eq!(page.semantic.kind, PageKind::Page);
    assert_eq!(page.format, Format::Org);

    fs::create_dir_all(root.join("create/pages")).unwrap();
    fs::create_dir_all(root.join("create/journals")).unwrap();
    let permit = graph.admit_retained_managed_text_writer().unwrap();
    assert_eq!(
        graph
            .managed_path_for(&permit, "New Page", PageKind::Page)
            .unwrap(),
        root.join("create/pages/New Page.md")
    );
    assert_eq!(
        graph
            .managed_path_for(&permit, "2026-07-25", PageKind::Journal)
            .unwrap(),
        root.join("create/journals/25-07-2026.md")
    );

    let _ = fs::remove_dir_all(&root);
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
    let path = ManagedPath::parse("content/pages/25-07-2026.md").unwrap();

    let configured = graph.managed_entry_for_managed_path(&path).unwrap();
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
fn managed_entry_decoder_uses_og_filename_semantics_outside_configured_roots() {
    let root = scratch("managed-entry-nonstandard-layout");
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
    let nested = ManagedPath::parse("archive/2024/client notes/Ünicode Page.md").unwrap();
    let entry = graph.managed_entry_for_managed_path(&nested).unwrap();
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
    let journal = ManagedPath::parse("archive/2024/25-07-2026.org").unwrap();
    let entry = graph.managed_entry_for_managed_path(&journal).unwrap();
    assert_eq!(entry.kind, PageKind::Journal);
    assert_eq!(entry.name, "2026-07-25");
    assert!(entry.date_key.is_some());
    assert_eq!(entry.rel_path, "archive/2024/25-07-2026.org");

    // A graph-root file is equally ordinary graph text for OG.
    let top = ManagedPath::parse("Top Level.md").unwrap();
    let entry = graph.managed_entry_for_managed_path(&top).unwrap();
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
        let path = ManagedPath::parse(relative).unwrap();
        let entry = graph.managed_entry_for_managed_path(&path).unwrap();
        assert_eq!(entry.kind, PageKind::Page, "{relative}");
        assert_eq!(entry.name, expected_name, "{relative}");
        assert_eq!(entry.rel_path, relative, "{relative}");
    }

    // Containers OG itself ignores, hidden paths, provider conflict copies,
    // and spellings the guarded sparse writer cannot project stay refused.
    for refused in [
        "assets/note.md",
        "publish/note.md",
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
                .managed_entry_for_managed_path(&ManagedPath::parse(refused).unwrap())
                .is_err(),
            "accepted {refused}"
        );
    }
    for invalid in ["archive/note.txt", "archive/../escape.md"] {
        assert!(ManagedPath::parse(invalid).is_err(), "accepted {invalid}");
    }

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn admission_snapshot_excludes_reserved_paths_and_failed_builds_poison() {
    let root = scratch("admission-exclusions");
    fs::create_dir_all(root.join("logseq")).unwrap();
    fs::write(root.join("logseq/config.edn"), "{:hidden [\"private\"]}\n").unwrap();
    for relative in [
        ".hidden/x.md",
        "assets/x.md",
        "publish/x.org",
        "node_modules/x.md",
        ".tine-sync/x.md",
        "logseq/.recycle/x.md",
        "private/x.md",
    ] {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"- excluded\n").unwrap();
    }
    fs::write(
        root.join("Dropbox (conflicted copy 2026-07-25).md"),
        b"- conflict\n",
    )
    .unwrap();
    fs::write(root.join("Visible.md"), b"- visible\n").unwrap();
    let graph = Graph::open(&root);
    graph.initial_shadow_raw_managed_text_inventory().unwrap();
    let index = graph.guarded_graph_text_identity_index().unwrap();
    assert_eq!(index.files_by_exact_path.iter().count(), 1);
    assert!(index
        .files_by_exact_path
        .contains_key(&ManagedPath::parse("Visible.md").unwrap()));

    let invalid = scratch("admission-invalid-utf8");
    fs::write(invalid.join("Bad.md"), [0xff, 0xfe]).unwrap();
    let invalid_graph = Graph::open(&invalid);
    assert!(invalid_graph
        .initial_shadow_raw_managed_text_inventory()
        .is_err());

    let parse = scratch("admission-parse-failure");
    fs::write(parse.join("Bad.md"), b"- valid bytes\n").unwrap();
    let parse_graph = Graph::open(&parse);
    GRAPH_TEXT_PARSE_FAILURE.with(|failure| failure.set(true));
    assert!(parse_graph
        .initial_shadow_raw_managed_text_inventory()
        .is_err());

    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&invalid);
    let _ = fs::remove_dir_all(&parse);
}

#[cfg(unix)]
#[test]
fn admission_snapshot_retains_all_collision_group_evidence() {
    let portable = scratch("admission-portable-groups");
    fs::write(portable.join("Caf\u{e9}.md"), b"- composed\n").unwrap();
    fs::write(
        portable.join("Cafe\u{301}.MD"),
        b"- decomposed and mixed case\n",
    )
    .unwrap();
    let graph = Graph::open(&portable);
    graph.initial_shadow_raw_managed_text_inventory().unwrap();
    let index = graph.guarded_graph_text_identity_index().unwrap();
    assert_eq!(
        index
            .paths_by_portable_key
            .get(&ManagedPath::parse("Caf\u{e9}.md").unwrap().portable_key())
            .unwrap()
            .len(),
        2
    );

    let semantic = scratch("admission-semantic-groups");
    fs::create_dir_all(semantic.join("a")).unwrap();
    fs::create_dir_all(semantic.join("b")).unwrap();
    fs::write(semantic.join("a/One.md"), b"title:: Shared\n\n- one\n").unwrap();
    fs::write(semantic.join("b/Two.org"), b"#+title: Shared\n* two\n").unwrap();
    let graph = Graph::open(&semantic);
    graph.initial_shadow_raw_managed_text_inventory().unwrap();
    let index = graph.guarded_graph_text_identity_index().unwrap();
    assert_eq!(
        index
            .paths_by_semantic_key
            .get(&(0, crate::refs::page_key("Shared")))
            .unwrap()
            .len(),
        2
    );

    let _ = fs::remove_dir_all(&portable);
    let _ = fs::remove_dir_all(&semantic);
}

#[test]
fn graph_text_exact_path_authority_preserves_root_nested_and_markdown_spelling() {
    let root = scratch("admission-target");
    let graph = Graph::open(&root);
    let root_target = graph.graph_text_exact_path("Root.MarkDown", true).unwrap();
    assert!(root_target.parent_components.is_empty());
    assert_eq!(
        root_target.managed_path.as_ref().unwrap().as_str(),
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
        nested.managed_path.as_ref().unwrap().as_str(),
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
        nested.relative_path,
        "archive/2024/client notes/Ünicode Page.md"
    );
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
    let source = include_str!("model.rs");
    let shape = source
        .split_once("    fn ensure_projection_target_shape(")
        .expect("projection target-shape function")
        .1
        .split_once("\n    fn ensure_projection_parent_binding(")
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
        let parent = graph.projection_parent(&target, false).unwrap();
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
fn admission_large_same_semantic_key_build_is_linear_in_collision_members() {
    const FILES: usize = 2048;
    let root = scratch("admission-large-same-semantic-key");
    for index in 0..FILES {
        fs::write(
            root.join(format!("Page-{index:04}.md")),
            b"title:: Shared\n\n- body\n",
        )
        .unwrap();
    }

    let graph = Graph::open(&root);
    reset_graph_text_admission_test_counters();
    graph.initial_shadow_raw_managed_text_inventory().unwrap();
    let counters = graph_text_admission_test_counters();
    assert_eq!(
        counters.persistent_payload_members,
        FILES * 2,
        "each input joins one portable and one semantic group exactly once"
    );
    assert!(
        counters.persistent_node_allocations < FILES * 256,
        "persistent sealing must stay logarithmic, not clone growing groups: {counters:?}"
    );
    let index = graph.guarded_graph_text_identity_index().unwrap();
    let first = ManagedPath::parse("Page-0000.md").unwrap();
    let semantic_key =
        graph_text_semantic_key(&index.files_by_exact_path.get(&first).unwrap().semantic);
    assert_eq!(
        index
            .paths_by_semantic_key
            .get(&semantic_key)
            .unwrap()
            .len(),
        FILES
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn admission_semantic_accounting_admits_large_ordinary_text_and_rejects_overlong_title() {
    let root = scratch("admission-realistic-semantic-accounting");
    let graph = Graph::open(&root);
    let path = ManagedPath::parse("Ordinary.md").unwrap();
    let observed =
        graph_text_observed_semantic_name_upper_bound(&graph, &path, "- ordinary body\n")
            .unwrap()
            .semantic_name_bytes;
    let one_record =
        graph_text_file_record_worst_case_upper_bound(&graph, path.as_str().len() as u64, observed)
            .unwrap();
    let realistic_raw_corpus = 480_u64 * 1024 * 1024;
    assert!(realistic_raw_corpus < INITIAL_SHADOW_LIMITS.raw_bytes);
    assert!(
        checked_mul_bytes(one_record, 4).unwrap() < INITIAL_SHADOW_LIMITS.permanent_index_bytes,
        "ordinary titles must not be charged as four 120 MiB document bodies"
    );

    let overlong = "T".repeat(MAX_GRAPH_TEXT_SEMANTIC_NAME_BYTES as usize + 1);
    let content = format!("title:: {overlong}\n\n- body\n");
    assert!(graph_text_observed_semantic_name_upper_bound(&graph, &path, &content).is_err());

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn admission_journal_title_format_expansion_rejects_before_render_or_parse() {
    let root = scratch("admission-huge-journal-title-format");
    fs::create_dir_all(root.join("logseq")).unwrap();
    let huge_format = "X".repeat(
        (MAX_GRAPH_TEXT_SEMANTIC_NAME_BYTES / MAX_JOURNAL_TITLE_BYTES_PER_FORMAT_BYTE) as usize + 1,
    );
    fs::write(
        root.join("logseq/config.edn"),
        format!("{{:journal/page-title-format \"{huge_format}\"}}\n"),
    )
    .unwrap();
    fs::write(root.join("2026_07_25.md"), b"- short journal\n").unwrap();

    let graph = Graph::open(&root);
    reset_graph_text_admission_test_counters();
    assert!(graph.initial_shadow_raw_managed_text_inventory().is_err());
    assert_eq!(
        graph_text_admission_test_counters().parser_invocations,
        0,
        "format input and rendered bound must reject before title rendering or parsing"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn admission_normal_journal_title_format_is_budgeted_and_reconciled() {
    let root = scratch("admission-normal-journal-title-format");
    fs::create_dir_all(root.join("logseq")).unwrap();
    let title_format = "EEEE, dd-MM-yyyy";
    fs::write(
        root.join("logseq/config.edn"),
        format!("{{:journal/page-title-format \"{title_format}\"}}\n"),
    )
    .unwrap();
    fs::write(root.join("2026_07_25.md"), b"- short journal\n").unwrap();

    let graph = Graph::open(&root);
    let format_budget = graph_text_journal_title_format_budget(&graph).unwrap();
    assert_eq!(format_budget.input_bytes, title_format.len() as u64);
    assert_eq!(
        format_budget.rendered_bytes,
        title_format.len() as u64 * MAX_JOURNAL_TITLE_BYTES_PER_FORMAT_BYTE
    );
    graph.initial_shadow_raw_managed_text_inventory().unwrap();
    let index = graph.guarded_graph_text_identity_index().unwrap();
    let record = index
        .files_by_exact_path
        .get(&ManagedPath::parse("2026_07_25.md").unwrap())
        .unwrap();
    assert_eq!(record.semantic.name, "Saturday, 25-07-2026");
    assert!(
        record.semantic.name.capacity() as u64
            <= owned_string_len_upper_bound(format_budget.rendered_bytes).unwrap()
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn admission_complete_validator_checks_exact_reverse_keys_and_path_kinds() {
    let root = scratch("admission-total-invariant-checker");
    fs::write(root.join("One.md"), b"title:: One\n").unwrap();
    fs::write(root.join("Two.md"), b"title:: Two\n").unwrap();
    let graph = Graph::open(&root);
    graph.initial_shadow_raw_managed_text_inventory().unwrap();
    let index = graph.guarded_graph_text_identity_index().unwrap();
    let one = ManagedPath::parse("One.md").unwrap();
    let two = ManagedPath::parse("Two.md").unwrap();

    let mut wrong_semantic = (*index).clone();
    persistent_set_insert(
        &mut wrong_semantic.paths_by_semantic_key,
        graph_text_semantic_key(
            &wrong_semantic
                .files_by_exact_path
                .get(&two)
                .unwrap()
                .semantic,
        ),
        one.clone(),
    );
    assert!(validate_graph_text_admission_index(&wrong_semantic).is_err());

    let mut wrong_resource = (*index).clone();
    let two_resource = wrong_resource
        .files_by_exact_path
        .get(&two)
        .unwrap()
        .file_resource_id;
    persistent_set_insert(
        &mut wrong_resource.paths_by_file_resource,
        two_resource,
        "One.md".to_owned(),
    );
    assert!(validate_graph_text_admission_index(&wrong_resource).is_err());

    let mut missing_kind = (*index).clone();
    missing_kind
        .file_is_graph_text_by_exact_relative
        .remove(&"One.md".to_owned());
    assert!(validate_graph_text_admission_index(&missing_kind).is_err());

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn admission_memory_preflights_capture_maps_parser_and_overflow() {
    let capture_root = scratch("admission-capture-bound");
    for index in 0..8 {
        fs::write(
            capture_root.join(format!("Page-{index}.md")),
            vec![b'x'; 8 * 1024],
        )
        .unwrap();
    }
    let graph = Graph::open(&capture_root);
    reset_graph_text_admission_test_counters();
    assert_eq!(
        graph
            .initial_shadow_raw_managed_text_inventory_with_limits(INITIAL_SHADOW_LIMITS)
            .unwrap()
            .len(),
        8
    );
    assert_eq!(
        graph_text_admission_test_counters().builder_enumerations,
        3,
        "live admission must enumerate each graph root once"
    );

    let resource_root = scratch("admission-resource-map-preflight");
    fs::write(
        resource_root.join("retained-resource-with-a-long-name.bin"),
        b"x",
    )
    .unwrap();
    let graph = Graph::open(&resource_root);
    let permit = graph.admit_retained_managed_text_writer().unwrap();
    let capture = collect_initial_shadow_managed_inventory(&graph, &permit, true).unwrap();
    let permanent = graph_text_initial_permanent_upper_bound(&graph, &capture, true).unwrap();
    assert!(permanent > 0);
    reset_graph_text_admission_test_counters();
    assert!(graph
        .initial_shadow_raw_managed_text_inventory_with_limits(InitialShadowLimits {
            permanent_index_bytes: permanent - 1,
            ..INITIAL_SHADOW_LIMITS
        })
        .is_err());
    assert_eq!(
        graph_text_admission_test_counters().index_map_insertions,
        0,
        "resource maps must be rejected before their first permanent insertion"
    );

    let parser_root = scratch("admission-parser-peak-preflight");
    let title = "T".repeat(32 * 1024);
    let parser_content = format!("title:: {title}\n\n- body\n");
    fs::write(parser_root.join("Page.md"), &parser_content).unwrap();
    let graph = Graph::open(&parser_root);
    let permit = graph.admit_retained_managed_text_writer().unwrap();
    let capture = collect_initial_shadow_managed_inventory(&graph, &permit, true).unwrap();
    let permanent = graph_text_initial_permanent_upper_bound(&graph, &capture, true).unwrap();
    let obsolete_parse_peak = managed_page_build_upper_bound(&parser_content).unwrap();
    let parser_limit = checked_add_bytes(
        checked_add_bytes(capture.peak_build_charge, permanent).unwrap(),
        obsolete_parse_peak - 1,
    )
    .unwrap();
    reset_graph_text_admission_test_counters();
    assert!(graph
        .initial_shadow_raw_managed_text_inventory_with_limits(InitialShadowLimits {
            peak_build_bytes: parser_limit,
            ..INITIAL_SHADOW_LIMITS
        })
        .is_ok());
    let counters = graph_text_admission_test_counters();
    assert_eq!(counters.parser_invocations, 1);
    assert!(counters.index_map_insertions > 0);

    let overflow_root = scratch("admission-capture-charge-overflow");
    let graph = Graph::open(&overflow_root);
    GRAPH_TEXT_FIRST_CAPTURE_CHARGE_OVERRIDE.with(|charge| charge.set(Some(u64::MAX)));
    reset_graph_text_admission_test_counters();
    assert!(graph.initial_shadow_raw_managed_text_inventory().is_err());
    assert_eq!(
        graph_text_admission_test_counters().builder_enumerations,
        3,
        "capture-charge overflow must fail after one graph-wide census"
    );

    let _ = fs::remove_dir_all(&capture_root);
    let _ = fs::remove_dir_all(&resource_root);
    let _ = fs::remove_dir_all(&parser_root);
    let _ = fs::remove_dir_all(&overflow_root);
}

#[test]
fn initial_shadow_reads_each_enumerated_file_through_its_retained_binding() {
    let root = scratch("initial-shadow-open-race");
    fs::write(root.join("pages/a.md"), b"- first\n").unwrap();
    let graph = Graph::open(&root);
    let target = root.join("pages/a.md");
    let retired = root.join("pages/a.retired.md");
    MANAGED_INVENTORY_READ_RACE.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            fs::rename(&target, &retired)?;
            fs::write(&target, b"- replacement\n")
        }));
    });
    assert!(graph.initial_shadow_raw_managed_text_inventory().is_err());

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn initial_shadow_uses_configured_nested_roots_and_longest_root_identity() {
    let root = scratch("initial-shadow-configured-nested");
    fs::create_dir_all(root.join("logseq")).unwrap();
    fs::write(
        root.join("logseq/config.edn"),
        "{:pages-directory \"content/pages\"\n\
              :journals-directory \"content/journals\"\n\
              :journal/file-name-format \"dd-MM-yyyy\"\n\
              :journal/page-title-format \"yyyy-MM-dd\"}\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("content/pages/arbitrary/deeper")).unwrap();
    fs::create_dir_all(root.join("content/journals/archive/deeper")).unwrap();
    fs::write(
        root.join("content/pages/arbitrary/deeper/Project%2FPlan.md"),
        b"- page\n",
    )
    .unwrap();
    fs::write(
        root.join("content/journals/archive/deeper/25-07-2026.org"),
        b"* journal\n",
    )
    .unwrap();

    let graph = Graph::open(&root);
    let inventory = graph.initial_shadow_raw_managed_text_inventory().unwrap();
    assert_eq!(
        inventory
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice()))
            .collect::<Vec<_>>(),
        vec![
            (
                "content/journals/archive/deeper/25-07-2026.org",
                b"* journal\n".as_slice(),
            ),
            (
                "content/pages/arbitrary/deeper/Project%2FPlan.md",
                b"- page\n".as_slice(),
            ),
        ]
    );
    let journal = graph
        .managed_entry_for_managed_path(&inventory[0].0)
        .unwrap();
    assert_eq!(journal.kind, PageKind::Journal);
    assert_eq!(journal.name, "2026-07-25");
    let page = graph
        .managed_entry_for_managed_path(&inventory[1].0)
        .unwrap();
    assert_eq!(page.kind, PageKind::Page);
    assert_eq!(page.name, "Project/Plan");

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn initial_shadow_accepts_markdown_in_a_configured_nested_page_root() {
    let root = scratch("initial-shadow-configured-markdown");
    fs::create_dir_all(root.join("logseq")).unwrap();
    fs::write(
        root.join("logseq/config.edn"),
        "{:pages-directory \"content/pages\"}\n",
    )
    .unwrap();
    let path = root.join("content/pages/arbitrary/deeper/Project%2FPlan.markdown");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"- markdown page\n").unwrap();

    let graph = Graph::open(&root);
    let inventory = graph.initial_shadow_raw_managed_text_inventory().unwrap();
    assert_eq!(inventory.len(), 1);
    assert_eq!(
        inventory[0].0.as_str(),
        "content/pages/arbitrary/deeper/Project%2FPlan.markdown"
    );
    assert_eq!(inventory[0].1, b"- markdown page\n");
    let page = graph
        .managed_entry_for_managed_path(&inventory[0].0)
        .unwrap();
    assert_eq!(page.kind, PageKind::Page);
    assert_eq!(page.name, "Project/Plan");

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn initial_shadow_handles_overlapping_roots_in_both_directions() {
    for (tag, pages, journals, expected) in [
        (
            "outer-pages",
            "content",
            "content/journals",
            vec![
                ("content/journals/archive.org", PageKind::Page),
                ("content/project.md", PageKind::Page),
            ],
        ),
        (
            "outer-journals",
            "content/pages",
            "content",
            vec![
                ("content/archive.org", PageKind::Page),
                ("content/pages/project.md", PageKind::Page),
            ],
        ),
    ] {
        let root = scratch(&format!("initial-shadow-overlap-{tag}"));
        fs::create_dir_all(root.join("logseq")).unwrap();
        fs::write(
            root.join("logseq/config.edn"),
            format!("{{:pages-directory \"{pages}\"\n:journals-directory \"{journals}\"}}\n"),
        )
        .unwrap();
        for (path, _) in &expected {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(
                &path,
                if path.extension().and_then(|value| value.to_str()) == Some("org") {
                    b"* entry\n".as_slice()
                } else {
                    b"- entry\n".as_slice()
                },
            )
            .unwrap();
        }
        let graph = Graph::open(&root);
        let inventory = graph.initial_shadow_raw_managed_text_inventory().unwrap();
        assert_eq!(inventory.len(), 2);
        for (path, kind) in expected {
            let (managed, _) = inventory
                .iter()
                .find(|(managed, _)| managed.as_str() == path)
                .unwrap();
            assert_eq!(
                graph.managed_entry_for_managed_path(managed).unwrap().kind,
                kind
            );
        }
        let _ = fs::remove_dir_all(&root);
    }
}

#[test]
fn live_admission_ignores_equal_creation_roots_and_defers_directory_retarget() {
    let equal = scratch("initial-shadow-equal-roots");
    fs::create_dir_all(equal.join("logseq")).unwrap();
    fs::write(
        equal.join("logseq/config.edn"),
        "{:pages-directory \"content\" :journals-directory \"content\"}\n",
    )
    .unwrap();
    fs::create_dir_all(equal.join("content")).unwrap();
    assert!(Graph::open(&equal)
        .initial_shadow_raw_managed_text_inventory()
        .unwrap()
        .is_empty());

    let retarget = scratch("initial-shadow-configured-retarget");
    fs::create_dir_all(retarget.join("logseq")).unwrap();
    fs::write(
        retarget.join("logseq/config.edn"),
        "{:pages-directory \"content/pages\" :journals-directory \"content/journals\"}\n",
    )
    .unwrap();
    fs::create_dir_all(retarget.join("content/pages")).unwrap();
    fs::write(retarget.join("content/pages/a.md"), b"- same\n").unwrap();
    let graph = Graph::open(&retarget);
    INITIAL_SHADOW_REVALIDATION_RACE.with(|hook| {
        let pages = retarget.join("content/pages");
        let retired = retarget.join("content/pages-retired");
        *hook.borrow_mut() = Some(Box::new(move || {
            fs::rename(&pages, retired)?;
            fs::create_dir_all(&pages)?;
            fs::write(pages.join("a.md"), b"- same\n")
        }));
    });
    assert_eq!(
        graph.initial_shadow_raw_managed_text_inventory().unwrap(),
        vec![(
            ManagedPath::parse("content/pages/a.md").unwrap(),
            b"- same\n".to_vec(),
        )]
    );

    let _ = fs::remove_dir_all(&equal);
    let _ = fs::remove_dir_all(&retarget);
}

#[cfg(unix)]
#[test]
fn initial_shadow_rejects_a_configured_root_symlink() {
    use std::os::unix::fs::symlink;

    let root = scratch("initial-shadow-configured-symlink");
    let outside = scratch("initial-shadow-configured-symlink-outside");
    fs::create_dir_all(root.join("logseq")).unwrap();
    fs::write(
        root.join("logseq/config.edn"),
        "{:pages-directory \"content/pages\" :journals-directory \"content/journals\"}\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("content")).unwrap();
    symlink(outside.join("pages"), root.join("content/pages")).unwrap();
    let graph = Graph::open(&root);
    assert!(graph.initial_shadow_raw_managed_text_inventory().is_err());

    let _ = fs::remove_file(root.join("content/pages"));
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&outside);
}

#[test]
fn initial_shadow_validation_pass_retains_metadata_not_second_raw_inventory() {
    let root = scratch("initial-shadow-peak");
    fs::write(root.join("pages/a.md"), vec![b'a'; 4096]).unwrap();
    fs::write(root.join("pages/b.md"), vec![b'b'; 8192]).unwrap();
    let graph = Graph::open(&root);
    let permit = graph.admit_retained_managed_text_writer().unwrap();

    let first = collect_initial_shadow_managed_inventory(&graph, &permit, true).unwrap();
    let validation = collect_initial_shadow_managed_inventory(&graph, &permit, false).unwrap();
    assert!(first.entries.iter().all(|entry| entry.bytes.is_some()));
    assert!(validation.entries.iter().all(|entry| entry.bytes.is_none()));
    assert_eq!(
        first
            .entries
            .iter()
            .map(|entry| (
                entry.path.clone(),
                entry.description,
                entry.file_resource_id
            ))
            .collect::<Vec<_>>(),
        validation
            .entries
            .iter()
            .map(|entry| (
                entry.path.clone(),
                entry.description,
                entry.file_resource_id
            ))
            .collect::<Vec<_>>()
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn initial_shadow_iterative_limits_count_empty_depth_and_nonmanaged_entries() {
    let deep = scratch("initial-shadow-deep-limit");
    fs::create_dir_all(deep.join("pages/a/b/c/d")).unwrap();
    let graph = Graph::open(&deep);
    let limits = InitialShadowLimits {
        directory_depth: 3,
        ..INITIAL_SHADOW_LIMITS
    };
    let permit = graph.admit_retained_managed_text_writer().unwrap();
    assert!(
        collect_initial_shadow_managed_inventory_with_limits(&graph, &permit, true, limits, 0,)
            .is_err()
    );

    let many = scratch("initial-shadow-all-entry-limit");
    for index in 0..4 {
        fs::write(
            many.join("pages").join(format!(".projection-{index}.tmp")),
            b"",
        )
        .unwrap();
    }
    let graph = Graph::open(&many);
    let limits = InitialShadowLimits {
        all_entries: 3,
        ..INITIAL_SHADOW_LIMITS
    };
    let permit = graph.admit_retained_managed_text_writer().unwrap();
    assert!(
        collect_initial_shadow_managed_inventory_with_limits(&graph, &permit, true, limits, 0,)
            .is_err()
    );

    let pending = scratch("initial-shadow-pending-directory-limit");
    fs::write(pending.join("pages/a.md"), b"").unwrap();
    let graph = Graph::open(&pending);
    let limits = InitialShadowLimits {
        pending_directories: 0,
        ..INITIAL_SHADOW_LIMITS
    };
    let permit = graph.admit_retained_managed_text_writer().unwrap();
    assert!(
        collect_initial_shadow_managed_inventory_with_limits(&graph, &permit, true, limits, 0,)
            .is_err()
    );

    let _ = fs::remove_dir_all(&deep);
    let _ = fs::remove_dir_all(&many);
    let _ = fs::remove_dir_all(&pending);
}

#[cfg(unix)]
#[test]
fn initial_shadow_rejects_file_aliases_but_retains_portable_collisions() {
    let aliases = scratch("initial-shadow-hardlink");
    fs::write(aliases.join("pages/a.md"), b"- a\n").unwrap();
    fs::hard_link(aliases.join("pages/a.md"), aliases.join("pages/alias.tmp")).unwrap();
    let graph = Graph::open(&aliases);
    assert!(graph.initial_shadow_raw_managed_text_inventory().is_err());

    let portable = scratch("initial-shadow-portable");
    fs::write(portable.join("pages/Foo.md"), b"- upper\n").unwrap();
    fs::write(portable.join("pages/foo.md"), b"- lower\n").unwrap();
    let graph = Graph::open(&portable);
    graph.initial_shadow_raw_managed_text_inventory().unwrap();
    let index = graph.guarded_graph_text_identity_index().unwrap();
    assert_eq!(
        index
            .paths_by_portable_key
            .get(&ManagedPath::parse("pages/Foo.md").unwrap().portable_key())
            .unwrap()
            .len(),
        2
    );

    let _ = fs::remove_dir_all(&aliases);
    let _ = fs::remove_dir_all(&portable);
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

#[cfg(unix)]
#[test]
fn projection_exact_parent_retarget_cannot_redirect_absent_publication() {
    let dir = scratch("projection-retarget-absent");
    let outside = scratch("projection-retarget-absent-outside");
    let moved = dir.join("pages-retained");
    fs::write(outside.join("Projection.md"), b"- outside\n").unwrap();
    let graph = Graph::open(&dir);

    PROJECTION_PARENT_RETARGET.with(|retarget| {
        *retarget.borrow_mut() = Some(ProjectionParentRetarget {
            parent: dir.join("pages"),
            moved: moved.clone(),
            outside: outside.clone(),
        });
    });
    assert!(graph
        .write_projection_exact("pages/Projection.md", None, b"- target\n")
        .is_err());
    assert_eq!(
        fs::read(outside.join("Projection.md")).unwrap(),
        b"- outside\n"
    );
    assert!(!moved.join("Projection.md").exists());
    assert!(!graph
        .recent_writes
        .lock()
        .unwrap()
        .contains_key(&dir.join("pages/Projection.md")));

    fs::remove_file(dir.join("pages")).unwrap();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&outside);
}

#[cfg(unix)]
#[test]
fn projection_exact_parent_retarget_cannot_redirect_replacement() {
    let dir = scratch("projection-retarget-base");
    let outside = scratch("projection-retarget-base-outside");
    let moved = dir.join("pages-retained");
    fs::write(dir.join("pages/Projection.md"), b"- base\n").unwrap();
    fs::write(outside.join("Projection.md"), b"- outside\n").unwrap();
    let graph = Graph::open(&dir);

    PROJECTION_PARENT_RETARGET.with(|retarget| {
        *retarget.borrow_mut() = Some(ProjectionParentRetarget {
            parent: dir.join("pages"),
            moved: moved.clone(),
            outside: outside.clone(),
        });
    });
    assert!(graph
        .write_projection_exact("pages/Projection.md", Some(b"- base\n"), b"- target\n")
        .is_err());
    assert_eq!(
        fs::read(outside.join("Projection.md")).unwrap(),
        b"- outside\n"
    );
    assert_eq!(fs::read(moved.join("Projection.md")).unwrap(), b"- base\n");

    fs::remove_file(dir.join("pages")).unwrap();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&outside);
}

#[cfg(unix)]
#[test]
fn projection_exact_rejects_unsafe_symlink_special_twin_and_non_utf8_paths() {
    use std::os::unix::fs::symlink;

    let dir = scratch("projection-exact-paths");
    let graph = Graph::open(&dir);
    for invalid in [
        "/tmp/Projection.md",
        "pages/../Projection.md",
        "assets/Projection.md",
        "pages/Projection.md:stream.md",
        "pages/CON.md",
        "pages/NUL.md",
    ] {
        assert!(
            graph
                .write_projection_exact(invalid, None, b"- target\n")
                .is_err(),
            "unsafe path {invalid:?} was accepted"
        );
    }
    assert!(graph
        .write_projection_exact("pages/Projection.md", None, &[0xff])
        .is_err());
    assert!(graph
        .write_projection_exact("pages/Projection.md", Some(&[0xff]), b"- target\n")
        .is_err());
    fs::write(dir.join("pages/NonUtf8.md"), [0xff]).unwrap();
    assert!(graph
        .write_projection_exact("pages/NonUtf8.md", Some(&[0xff]), b"- target\n")
        .is_err());
    assert!(graph
        .recover_projection_exact("pages/NonUtf8.md", &[0xff])
        .is_err());

    let real = dir.join("pages/Real.md");
    let linked = dir.join("pages/Linked.md");
    fs::write(&real, b"- real\n").unwrap();
    symlink(&real, &linked).unwrap();
    assert!(graph
        .write_projection_exact("pages/Linked.md", Some(b"- real\n"), b"- target\n")
        .is_err());

    symlink(dir.join("journals"), dir.join("pages/LinkedParent")).unwrap();
    assert!(graph
        .write_projection_exact("pages/LinkedParent/Projection.md", None, b"- target\n")
        .is_err());
    assert!(!dir.join("journals/Projection.md").exists());

    fs::create_dir_all(dir.join("pages/Special.md")).unwrap();
    assert!(graph
        .write_projection_exact("pages/Special.md", None, b"- target\n")
        .is_err());

    let special_sibling_target = dir.join("pages/SpecialSibling.md");
    let special_sibling = dir.join("pages/SpecialSibling.org");
    fs::write(&special_sibling_target, b"- markdown\n").unwrap();
    fs::create_dir_all(&special_sibling).unwrap();
    assert!(graph
        .write_projection_exact(
            "pages/SpecialSibling.md",
            Some(b"- markdown\n"),
            b"- target\n",
        )
        .is_err());
    assert_eq!(fs::read(&special_sibling_target).unwrap(), b"- markdown\n");
    assert!(special_sibling.is_dir());

    let twin_md = dir.join("pages/Twin.md");
    let twin_org = dir.join("pages/Twin.org");
    fs::write(&twin_md, b"- markdown\n").unwrap();
    fs::write(&twin_org, b"* org\n").unwrap();
    let proof = graph
        .write_projection_exact("pages/Twin.md", Some(b"- markdown\n"), b"- target\n")
        .unwrap();
    assert_eq!(proof.bytes(), b"- target\n");
    assert_eq!(fs::read(&twin_md).unwrap(), b"- target\n");
    assert_eq!(fs::read(&twin_org).unwrap(), b"* org\n");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn projection_uses_editor_corruption_firewalls_before_mutation() {
    let dir = scratch("projection-shared-firewalls");
    let graph = Graph::open(&dir);

    let org_path = dir.join("pages/Unsafe.org");
    let unsafe_org = "* a\n*** c\n";
    fs::write(&org_path, unsafe_org).unwrap();
    let error = graph
        .write_projection_exact(
            "pages/Unsafe.org",
            Some(unsafe_org.as_bytes()),
            b"* replacement\n",
        )
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(fs::read(&org_path).unwrap(), unsafe_org.as_bytes());

    let header_path = dir.join("pages/Header.md");
    let header_base = "A:: header\nB:: retained\n\n- body\n";
    fs::write(&header_path, header_base).unwrap();
    let error = graph
        .write_projection_exact(
            "pages/Header.md",
            Some(header_base.as_bytes()),
            b"A:: header\n\n- B:: retained\n- body\n",
        )
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(fs::read(&header_path).unwrap(), header_base.as_bytes());

    let crlf_path = dir.join("pages/Crlf.md");
    fs::write(&crlf_path, b"- before\r\n").unwrap();
    let proof = graph
        .write_projection_exact("pages/Crlf.md", Some(b"- before\r\n"), b"- after\r\n")
        .unwrap();
    assert_eq!(proof.bytes(), b"- after\r\n");
    assert_eq!(fs::read(&crlf_path).unwrap(), b"- after\r\n");

    let _ = fs::remove_dir_all(&dir);
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
    let graph = Graph::open(&dir);

    let conflict = graph
        .write_projection_exact("pages/Foo.md", None, target)
        .unwrap_err();
    assert_eq!(conflict.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(&old).unwrap(), base);

    graph.remove_projection_exact("pages/foo.md", base).unwrap();
    graph
        .write_projection_exact("pages/Foo.md", None, target)
        .unwrap();
    assert_eq!(fs::read(&new).unwrap(), target);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn projection_reserved_name_collision_fails_no_replace() {
    let dir = scratch("projection-reserved-collision");
    let path = dir.join("pages/Collision.md");
    fs::write(&path, b"- base\n").unwrap();
    let graph = Graph::open(&dir);
    let reservation = ProjectionAttemptReservation::for_test("pages/Collision.md");
    let reserved = dir.join("pages").join(reservation.recovery_filename());
    fs::write(&reserved, b"- forged\n").unwrap();
    let write = graph.admit_retained_managed_text_writer().unwrap();
    let document = parse_doc(&path, "- target\n");
    let guarded_layout = GuardedProjectionLayout::canonical_for_test(&document);

    assert!(graph
        .write_page_projection_with_attempts(
            &write,
            "pages/Collision.md",
            Some(b"- base\n"),
            b"- target\n",
            &guarded_layout,
            &reservation,
            std::slice::from_ref(&reservation),
            None,
        )
        .is_err());
    assert_eq!(fs::read(&path).unwrap(), b"- base\n");
    assert_eq!(fs::read(&reserved).unwrap(), b"- forged\n");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn projection_collision_probes_are_bounded_with_ten_thousand_siblings() {
    let dir = scratch("projection-exact-probe-count");
    let pages = dir.join("pages");
    for index in 0..10_000 {
        fs::write(
            pages.join(format!("unrelated-{index}.md")),
            b"- unrelated\n",
        )
        .unwrap();
    }
    TEST_PROJECTION_ATTEMPTS.with(|catalog| catalog.borrow_mut().clear());
    PROJECTION_EXACT_OPEN_COUNT.with(|count| count.set(0));
    let graph = Graph::open(&dir);
    graph
        .write_projection_exact("pages/Constant.md", None, b"- target\n")
        .unwrap();
    let with_siblings = PROJECTION_EXACT_OPEN_COUNT.with(std::cell::Cell::get);

    let empty = scratch("projection-exact-probe-control");
    TEST_PROJECTION_ATTEMPTS.with(|catalog| catalog.borrow_mut().clear());
    PROJECTION_EXACT_OPEN_COUNT.with(|count| count.set(0));
    let graph = Graph::open(&empty);
    graph
        .write_projection_exact("pages/Constant.md", None, b"- target\n")
        .unwrap();
    let without_siblings = PROJECTION_EXACT_OPEN_COUNT.with(std::cell::Cell::get);
    assert!(
            with_siblings <= without_siblings + 16,
            "exact-path collision authority must not scan unrelated siblings: with_siblings={with_siblings}, without_siblings={without_siblings}"
        );

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty);
}

#[test]
fn sparse_giant_projection_file_is_rejected_before_allocation_and_untouched() {
    let dir = scratch("projection-giant-evidence");
    let path = dir.join("pages/Giant.md");
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    file.set_len(MAX_PROJECTION_EVIDENCE_BYTES + 1).unwrap();
    drop(file);
    let graph = Graph::open(&dir);

    let error = graph
        .write_projection_exact("pages/Giant.md", None, b"- target\n")
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        fs::metadata(&path).unwrap().len(),
        MAX_PROJECTION_EVIDENCE_BYTES + 1
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn production_projection_has_no_alternate_graph_writer_entrypoint() {
    let source = include_str!("model.rs");
    let forbidden = ["pub(crate) fn write_projection", "_exact"].concat();
    assert!(!source.contains(&forbidden));
    assert!(source.contains("pub(crate) fn write_page_projection"));
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
/// in `model.rs`; the storage boundary stays for app-private authorities only.
#[test]
fn direct_files_graph_text_publication_uses_the_graph_tree_noreplace_rename() {
    const RULE: &str = "GH #466 / I-16: Direct Files graph-text name transitions use \
        move_graph_text_exact_no_replace (the graph tree's renameat2(RENAME_NOREPLACE) \
        family), never tine-storage's DurableDirectoryPublication, whose Android arm is a \
        hard link that shared storage refuses; imitate move_graph_text_exact_no_replace";
    let source = include_str!("model.rs");
    let create = source
        .split_once("    fn managed_atomic_create_with_proof(")
        .expect("Direct Files create path")
        .1
        .split_once("\n    fn managed_atomic_write_with_conflict(")
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
        .split_once("    fn managed_atomic_write_validated(")
        .expect("Direct Files validated write path")
        .1
        .split_once("\n    /// Replace an existing editor target")
        .expect("Direct Files bounded replacement")
        .0;
    assert!(write.contains("self.managed_atomic_replace_bound("));
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
        .split_once("    fn managed_atomic_replace_bound(")
        .expect("Direct Files bounded replacement")
        .1
        .split_once("\n    fn managed_move_noreplace(")
        .expect("next projection method")
        .0;
    // The retire/publish closure, the recovery set-aside, and the restore.
    assert!(
        replace.matches("move_graph_text_exact_no_replace(").count() >= 3,
        "{RULE}"
    );
    assert!(!replace.contains("DurableDirectoryPublication"), "{RULE}");
    assert!(!replace.contains(".move_exact_no_replace("), "{RULE}");
    assert!(
        !replace.contains("EditorPublicationAuthority::DirectFile => rename_projection_noreplace"),
        "the Direct arm must carry the exact-byte protocol, not the bare rename"
    );
    assert!(
        replace.contains("EditorPublicationAuthority::ReconstructibleManagedProjection => {"),
        "the managed projection arm keeps its own capability-fallback rename"
    );
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
        AtomicReplaceOutcome::ExternalChanged(found) => assert_eq!(found, b"external"),
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
            MANAGED_TEXT_INVENTORY_LIMITS_OVERRIDE.with(|limits| {
                *limits.borrow_mut() = None;
            });
        }
    }

    let dir = scratch("editor-recovery-walk-bound");
    let _reset = LimitsReset;
    MANAGED_TEXT_INVENTORY_LIMITS_OVERRIDE.with(|limits| {
        *limits.borrow_mut() = Some(ManagedTextInventoryLimits {
            all_entries: 0,
            ..MANAGED_TEXT_INVENTORY_LIMITS
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

#[cfg(unix)]
#[test]
fn editor_recovery_sweep_refuses_a_multi_link_artifact() {
    let dir = scratch("editor-recovery-hardlink");
    let artifact = dir.join("pages").join(".Note.md.4242.1.editor-recovery");
    fs::write(&artifact, b"- linked bytes\n").unwrap();
    fs::hard_link(&artifact, dir.join("linked-copy")).unwrap();

    let refused = match Graph::open_checked(&dir) {
        Ok(_) => panic!("checked open accepted a multi-link W1 claimant"),
        Err(error) => error,
    };

    assert_eq!(refused.kind(), io::ErrorKind::AlreadyExists, "{refused}");
    assert!(!dir.join("pages/Note.md").exists());
    assert_eq!(fs::read(&artifact).unwrap(), b"- linked bytes\n");
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

#[cfg(unix)]
#[test]
fn checked_open_rejects_managed_output_symlink_escapes() {
    use std::os::unix::fs::symlink;
    for managed in ["assets", "logseq", "publish"] {
        let dir = scratch(&format!("checked-open-{managed}-symlink"));
        let outside = std::env::temp_dir().join(format!(
            "tine-checked-open-{managed}-outside-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&outside);
        fs::create_dir_all(&outside).unwrap();
        let managed_path = dir.join(managed);
        let _ = fs::remove_dir_all(&managed_path);
        symlink(&outside, &managed_path).unwrap();

        assert!(
            Graph::open_checked(&dir).is_err(),
            "accepted escaped {managed} directory"
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
fn checked_open_rejects_managed_directories_aliased_inside_graph() {
    use std::os::unix::fs::symlink;
    let dir = scratch("checked-open-managed-alias");
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
    let g = Graph::open(&dir);
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
        !g.search("nestedsentinel", 10).is_empty(),
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

    let g = Graph::open(&dir);
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
fn managed_text_twin_refusal_includes_markdown_extension_variant() {
    let dir = scratch("managed-markdown-twin");
    fs::write(dir.join("pages/Twin.md"), "- md body\n").unwrap();
    fs::write(dir.join("pages/Twin.markdown"), "- markdown body\n").unwrap();
    let graph = Graph::open(&dir);
    let write = graph.admit_managed_text_writer().unwrap();

    assert!(graph
        .managed_has_twin(&write, "Twin", PageKind::Page)
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

#[cfg(any(unix, windows))]
#[test]
fn handoff_mint_and_writer_admission_race_at_the_gate() {
    let dir = scratch("handoff-admission-race");
    let graph = Arc::new(Graph::open(&dir));
    let (workspace_id, endpoint) = handoff_binding(&graph, 90_900);
    let gate = Arc::clone(managed_write_gate(&graph));
    gate.set_admission_race_barrier(Some(Arc::new(std::sync::Barrier::new(2))));

    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let (writer_release_tx, writer_release_rx) = std::sync::mpsc::channel();
    let (mint_release_tx, mint_release_rx) = std::sync::mpsc::channel();
    let writer = std::thread::spawn({
        let graph = Arc::clone(&graph);
        let result_tx = result_tx.clone();
        move || match graph.admit_managed_text_writer() {
            Ok(permit) => {
                result_tx.send(("writer", Ok(()))).unwrap();
                writer_release_rx.recv().unwrap();
                drop(permit);
            }
            Err(error) => {
                result_tx.send(("writer", Err(error.kind()))).unwrap();
                writer_release_rx.recv().unwrap();
            }
        }
    });
    let mint = std::thread::spawn({
        let graph = Arc::clone(&graph);
        move || match graph.mint_handoff_safe(workspace_id, endpoint) {
            Ok(handoff) => {
                result_tx.send(("handoff", Ok(()))).unwrap();
                mint_release_rx.recv().unwrap();
                drop(handoff);
            }
            Err(error) => {
                result_tx.send(("handoff", Err(error.kind()))).unwrap();
                mint_release_rx.recv().unwrap();
            }
        }
    });

    let first = result_rx.recv().unwrap();
    let second = result_rx.recv().unwrap();
    gate.set_admission_race_barrier(None);
    let outcomes = [first, second];
    assert_eq!(
        outcomes.iter().filter(|(_, result)| result.is_ok()).count(),
        1,
        "the barrier placed both operations inside the admission race"
    );
    for (_, result) in outcomes {
        if let Err(kind) = result {
            assert_eq!(kind, io::ErrorKind::WouldBlock);
        }
    }

    writer_release_tx.send(()).unwrap();
    mint_release_tx.send(()).unwrap();
    writer.join().unwrap();
    mint.join().unwrap();
    assert!(graph
        .create_markdown_page_if_absent("after admission race", "- admitted\n")
        .unwrap());
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(any(unix, windows))]
#[test]
fn handoff_blocks_journal_migration_but_not_auxiliary_conflict_trash() {
    let dir = scratch("handoff-omitted-entrypoints");
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq").join("config.edn"),
        "{:preferred-format \"Org\"\n :journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
    )
    .unwrap();
    let title_named = "Thursday, 25-06-2026.org";
    fs::write(dir.join("journals").join(title_named), "* migrate me\n").unwrap();
    let conflict = "Foo.sync-conflict-20260705-141233-A2B2C3D.md";
    fs::write(dir.join("pages").join(conflict), "- trash me\n").unwrap();

    let graph = Arc::new(Graph::open(&dir));
    let (workspace_id, endpoint) = handoff_binding(&graph, 90_950);
    let handoff = graph.mint_handoff_safe(workspace_id, endpoint).unwrap();
    let start = Arc::new(std::sync::Barrier::new(3));
    let migration = std::thread::spawn({
        let graph = Arc::clone(&graph);
        let start = Arc::clone(&start);
        move || {
            start.wait();
            graph.migrate_journal_filenames_checked()
        }
    });
    let trash = std::thread::spawn({
        let graph = Arc::clone(&graph);
        let start = Arc::clone(&start);
        move || {
            start.wait();
            graph.trash_sync_conflict(&format!("pages/{conflict}"))
        }
    });

    start.wait();
    assert_handoff_blocked(migration.join().unwrap());
    trash.join().unwrap().unwrap();
    assert!(dir.join("journals").join(title_named).exists());
    assert!(!dir.join("pages").join(conflict).exists());
    assert_eq!(
        trash_stats(&trash_root(&dir)).conflicts,
        1,
        "the conflict copy remains recoverable while graph-text authority is retired"
    );

    drop(handoff);
    assert_eq!(graph.migrate_journal_filenames_checked().unwrap(), 1);
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(any(unix, windows))]
#[test]
fn handoff_resource_gate_blocks_reopen_but_not_an_independent_graph() {
    let dir = scratch("handoff-reopen-gate");
    let other_dir = scratch("handoff-independent-gate");
    let graph = Arc::new(Graph::open(&dir));
    let (workspace_id, endpoint) = handoff_binding(&graph, 91_025);
    let handoff = graph.mint_handoff_safe(workspace_id, endpoint).unwrap();
    let reopened = Arc::new(Graph::open(&dir));
    assert!(Arc::ptr_eq(
        managed_write_gate(&graph),
        managed_write_gate(&reopened)
    ));
    assert!(handoff
        .verify_binding(&reopened, workspace_id, endpoint)
        .is_err());
    let independent = Arc::new(Graph::open(&other_dir));
    let start = Arc::new(std::sync::Barrier::new(3));
    let blocked_writer = std::thread::spawn({
        let graph = Arc::clone(&reopened);
        let start = Arc::clone(&start);
        move || {
            start.wait();
            graph.create_markdown_page_if_absent("same resource", "- blocked\n")
        }
    });
    let independent_writer = std::thread::spawn({
        let graph = Arc::clone(&independent);
        let start = Arc::clone(&start);
        move || {
            start.wait();
            graph.create_markdown_page_if_absent("other resource", "- admitted\n")
        }
    });

    start.wait();
    assert_handoff_blocked(blocked_writer.join().unwrap());
    assert!(independent_writer.join().unwrap().unwrap());
    drop(handoff);
    assert!(reopened
        .create_markdown_page_if_absent("same resource released", "- admitted\n")
        .unwrap());
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&other_dir);
}

#[cfg(unix)]
#[test]
fn handoff_resource_gate_blocks_a_reopened_symlink_alias() {
    use std::os::unix::fs::symlink;

    let dir = scratch("handoff-symlink-resource");
    let alias = dir.with_file_name("tine-handoff-symlink-resource-alias");
    let _ = fs::remove_file(&alias);
    symlink(&dir, &alias).unwrap();
    let graph = Graph::open(&dir);
    let reopened = Graph::open(&alias);
    assert!(Arc::ptr_eq(
        managed_write_gate(&graph),
        managed_write_gate(&reopened)
    ));
    let (workspace_id, endpoint) = handoff_binding(&graph, 91_035);
    let handoff = graph.mint_handoff_safe(workspace_id, endpoint).unwrap();

    assert_handoff_blocked(
        reopened.create_markdown_page_if_absent("symlink alias blocked", "- no\n"),
    );
    assert!(!dir.join("pages").join("symlink alias blocked.md").exists());
    assert!(handoff
        .verify_binding(&reopened, workspace_id, endpoint)
        .is_err());

    drop(handoff);
    assert!(reopened
        .create_markdown_page_if_absent("symlink alias released", "- yes\n")
        .unwrap());
    let _ = fs::remove_file(&alias);
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(any(unix, windows))]
#[test]
fn managed_writer_identity_failure_never_mints_an_independent_gate() {
    let dir = scratch("handoff-identity-acquisition-failure");
    let graph = Graph::open(&dir);
    let (workspace_id, endpoint) = handoff_binding(&graph, 91_040);
    let handoff = graph.mint_handoff_safe(workspace_id, endpoint).unwrap();
    MANAGED_WRITE_IDENTITY_ACQUISITION.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(|| {
            Err(io::Error::other(
                "injected transient graph identity acquisition failure",
            ))
        }));
    });
    let identity_failed = Graph::open(&dir);

    assert!(identity_failed.managed_write_binding().is_err());
    assert!(identity_failed
        .create_markdown_page_if_absent("identity bypass", "- no\n")
        .is_err());
    assert!(!dir.join("pages").join("identity bypass.md").exists());

    drop(handoff);
    assert!(identity_failed
        .create_markdown_page_if_absent("still fail closed", "- no\n")
        .is_err());
    assert!(graph
        .create_markdown_page_if_absent("original binding released", "- yes\n")
        .unwrap());
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn stale_graph_writes_retained_resource_not_replacement_reserved_by_another_gate() {
    let dir = scratch("handoff-stale-root");
    let moved = dir.with_file_name("tine-handoff-stale-root-moved");
    let _ = fs::remove_dir_all(&moved);
    let stale = Graph::open(&dir);
    fs::rename(&dir, &moved).unwrap();
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("journals")).unwrap();
    let replacement = Graph::open(&dir);
    let (workspace_id, endpoint) = handoff_binding(&replacement, 91_045);
    let handoff = replacement
        .mint_handoff_safe(workspace_id, endpoint)
        .unwrap();

    assert!(stale
        .create_markdown_page_if_absent("retained writer", "- retained\n")
        .unwrap());
    assert!(!dir.join("pages").join("retained writer.md").exists());
    assert_eq!(
        fs::read(moved.join("pages").join("retained writer.md")).unwrap(),
        b"- retained\n"
    );
    assert_handoff_blocked(
        replacement.create_markdown_page_if_absent("replacement blocked", "- no\n"),
    );

    drop(handoff);
    assert!(replacement
        .create_markdown_page_if_absent("replacement released", "- yes\n")
        .unwrap());
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&moved);
}

#[cfg(unix)]
#[test]
fn root_replacement_after_admission_writes_retained_resource() {
    let dir = scratch("handoff-admission-root-race");
    let moved = dir.with_file_name("tine-handoff-admission-root-race-moved");
    let _ = fs::remove_dir_all(&moved);
    let graph = Graph::open(&dir);
    MANAGED_WRITE_AFTER_ADMISSION.with(|hook| {
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
    let dir = scratch("handoff-root-replacement-page-lock");
    let moved = dir.with_file_name("tine-handoff-root-replacement-page-lock-moved");
    let _ = fs::remove_dir_all(&moved);
    let graph = Arc::new(Graph::open(&dir));
    let target = dir.join("pages").join("page lock retained.md");
    let lock = graph.page_lock(&target);
    let guard = lock.lock().unwrap();
    let (admitted_tx, admitted_rx) = std::sync::mpsc::channel();
    let writer = std::thread::spawn({
        let graph = Arc::clone(&graph);
        move || {
            MANAGED_WRITE_AFTER_IDENTITY_CHECK.with(|hook| {
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

#[cfg(unix)]
#[test]
fn replacement_after_final_check_cannot_redirect_mutation_into_reserved_root() {
    let dir = scratch("handoff-capability-final-window");
    let moved = dir.with_file_name("tine-handoff-capability-final-window-moved");
    let _ = fs::remove_dir_all(&moved);
    let graph = Graph::open(&dir);
    MANAGED_WRITE_BEFORE_MUTATION.with(|hook| {
        let dir = dir.clone();
        let moved = moved.clone();
        *hook.borrow_mut() = Some(Box::new(move || {
            fs::rename(&dir, &moved)?;
            fs::create_dir_all(dir.join("pages"))?;
            fs::create_dir_all(dir.join("journals"))?;
            fs::write(dir.join("pages").join("replacement sentinel.md"), "- B\n")?;
            let replacement = Graph::open(&dir);
            let (workspace_id, endpoint) = handoff_binding(&replacement, 91_047);
            let handoff = replacement.mint_handoff_safe(workspace_id, endpoint)?;
            MANAGED_WRITE_REPLACEMENT_HANDOFF.with(|held| {
                *held.borrow_mut() = Some(handoff);
            });
            Ok(())
        }));
    });

    assert!(graph
        .create_markdown_page_if_absent("final window", "- retained A\n")
        .unwrap());
    assert_eq!(
        fs::read(moved.join("pages").join("final window.md")).unwrap(),
        b"- retained A\n"
    );
    assert!(!dir.join("pages").join("final window.md").exists());
    assert_eq!(
        fs::read(dir.join("pages").join("replacement sentinel.md")).unwrap(),
        b"- B\n"
    );
    let replacement = Graph::open(&dir);
    assert_handoff_blocked(replacement.create_markdown_page_if_absent("reserved B", "- blocked\n"));
    MANAGED_WRITE_REPLACEMENT_HANDOFF.with(|held| drop(held.borrow_mut().take()));
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&moved);
}

#[test]
fn retained_content_budget_failed_reservation_is_atomic_and_retryable() {
    let budget = RetainedContentBudget::new(ManagedTextInventoryLimits {
        retained_content_bytes: 10,
        ..MANAGED_TEXT_INVENTORY_LIMITS
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
    let budget = RetainedContentBudget::new(ManagedTextInventoryLimits {
        retained_content_bytes: 64,
        ..MANAGED_TEXT_INVENTORY_LIMITS
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
    let peak = last_managed_content_budget_peak();

    let accepted = scratch("budget-rename-b");
    populate(&accepted);
    set_managed_content_budget_limit(peak);
    Graph::open(&accepted)
        .rename_page("Project", "Archive")
        .unwrap();
    clear_managed_content_budget_limit();
    assert!(accepted.join("pages/Archive.md").exists());
    assert!(fs::read_to_string(accepted.join("pages/Refs.md"))
        .unwrap()
        .contains("[[Archive]]"));

    let rejected = scratch("budget-rename-c");
    populate(&rejected);
    set_managed_content_budget_limit(peak - 1);
    let graph = Graph::open(&rejected);
    let error = graph.rename_page("Project", "Archive").unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        fs::read_to_string(rejected.join("pages/Project.md")).unwrap(),
        "- [[Project/Child]]\n"
    );
    assert!(!rejected.join("pages/Archive.md").exists());
    assert!(graph.cache.read().unwrap().is_none());
    assert!(graph.recent_writes.lock().unwrap().is_empty());

    set_managed_content_budget_limit(peak);
    graph.rename_page("Project", "Archive").unwrap();
    clear_managed_content_budget_limit();
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
    let small_peak = last_managed_content_budget_peak();

    let many = scratch("rename-container-many-a");
    populate(&many, 32);
    Graph::open(&many)
        .rename_page("Project", "Archive")
        .unwrap();
    let many_peak = last_managed_content_budget_peak();
    assert!(many_peak > small_peak);

    let rejected = scratch("rename-container-many-b");
    populate(&rejected, 32);
    let before = regular_file_tree(&rejected.join("pages"));
    let graph = Graph::open(&rejected);
    set_managed_content_budget_limit(many_peak - 1);
    assert_eq!(
        graph.rename_page("Project", "Archive").unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(regular_file_tree(&rejected.join("pages")), before);
    assert!(!rejected.join("pages/Archive.md").exists());
    assert!(graph.recent_writes.lock().unwrap().is_empty());
    set_managed_content_budget_limit(many_peak);
    graph.rename_page("Project", "Archive").unwrap();
    clear_managed_content_budget_limit();
    assert!(rejected.join("pages/Archive.md").exists());
    assert!(rejected.join("pages/Archive%2FTiny031.md").exists());

    let _ = fs::remove_dir_all(&small);
    let _ = fs::remove_dir_all(&many);
    let _ = fs::remove_dir_all(&rejected);
}

fn assert_exact_budget_cache_unchanged(
    graph: &Graph,
    before: &Option<Arc<Vec<(PageEntry, Arc<Document>)>>>,
) {
    let after = graph.cache.read().unwrap();
    match (&*after, before) {
        (None, None) => {}
        (Some(after), Some(before)) => assert!(Arc::ptr_eq(after, before)),
        _ => panic!("managed cache changed across rejected admission"),
    }
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

    let accepted = nested_document(MAX_MANAGED_BLOCK_DEPTH, Some(&deepest_reference));
    let accepted_dto = page_dto_checked(&entry, &accepted).unwrap();
    let mut accepted_walk = BlockDtoWalk::new(&accepted_dto.blocks);
    let mut accepted_count = 0_usize;
    while accepted_walk.next().unwrap().is_some() {
        accepted_count += 1;
    }
    assert_eq!(accepted_count, MAX_MANAGED_BLOCK_DEPTH);

    let accepted_block = block_to_dto(&accepted.roots[0]).unwrap();
    let mut accepted_block_walk = BlockDtoWalk::new(std::slice::from_ref(&accepted_block));
    let mut accepted_block_count = 0_usize;
    while accepted_block_walk.next().unwrap().is_some() {
        accepted_block_count += 1;
    }
    assert_eq!(accepted_block_count, MAX_MANAGED_BLOCK_DEPTH);

    let accepted_snapshot = Graph::from_page_snapshot(
        &dir,
        vec![
            (entry.clone(), Arc::new(accepted.clone())),
            (target_entry.clone(), Arc::new(target_doc.clone())),
        ],
    );
    let target_names = vec![crate::refs::page_key("Deep target")];
    let accepted_candidates =
        accepted_snapshot.reference_candidate_pages(&target_names, ReferenceKind::Explicit);
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
        MAX_MANAGED_BLOCK_DEPTH,
    );

    let rejected = nested_document(MAX_MANAGED_BLOCK_DEPTH + 1, Some(&deepest_reference));
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
        let candidates =
            rejected_snapshot.reference_candidate_pages(&target_names, ReferenceKind::Explicit);
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
    assert_eq!(accepted_markdown_count, MAX_MANAGED_BLOCK_DEPTH);
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
fn rename_inventory_and_referrer_reads_stay_on_retained_a_after_reserved_b_replacement() {
    let dir = scratch("handoff-rename-selection-retained");
    let moved = dir.with_file_name("tine-handoff-rename-selection-retained-moved");
    let _ = fs::remove_dir_all(&moved);
    fs::create_dir_all(dir.join("pages/client/deep")).unwrap();
    fs::create_dir_all(dir.join("journals/archive/2026")).unwrap();
    fs::write(dir.join("pages/Old.md"), "- old A\n").unwrap();
    fs::write(
        dir.join("pages/client/deep/Ref.md"),
        "- A-only nested [[Old]]\n",
    )
    .unwrap();
    fs::write(
        dir.join("journals/archive/2026/07_24.md"),
        "- A-only journal [[Old]]\n",
    )
    .unwrap();
    let graph = Graph::open(&dir);
    MANAGED_WRITE_AFTER_ADMISSION.with(|hook| {
        let dir = dir.clone();
        let moved = moved.clone();
        *hook.borrow_mut() = Some(Box::new(move || {
            fs::rename(&dir, &moved)?;
            fs::create_dir_all(dir.join("pages"))?;
            fs::create_dir_all(dir.join("journals"))?;
            fs::write(dir.join("pages/Old.md"), "- old B\n")?;
            fs::write(dir.join("pages/B-only.md"), "- B sentinel\n")?;
            hold_replacement_handoff(&dir, 91_047_100)
        }));
    });

    graph.rename_page_expected("Old", "New", None).unwrap();

    assert!(!moved.join("pages/Old.md").exists());
    assert_eq!(fs::read(moved.join("pages/New.md")).unwrap(), b"- old A\n");
    assert_eq!(
        fs::read(moved.join("pages/client/deep/Ref.md")).unwrap(),
        b"- A-only nested [[New]]\n"
    );
    assert_eq!(
        fs::read(moved.join("journals/archive/2026/07_24.md")).unwrap(),
        b"- A-only journal [[New]]\n"
    );
    assert_eq!(fs::read(dir.join("pages/Old.md")).unwrap(), b"- old B\n");
    assert!(!dir.join("pages/New.md").exists());
    assert!(!dir.join("pages/client/deep/Ref.md").exists());
    assert_eq!(
        fs::read(dir.join("pages/B-only.md")).unwrap(),
        b"- B sentinel\n"
    );
    let replacement = Graph::open(&dir);
    assert_handoff_blocked(replacement.rename_page("Old", "Blocked"));

    release_replacement_handoff();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&moved);
}
#[cfg(unix)]
#[test]
fn journal_migration_selection_stays_on_nested_retained_a_after_reserved_b_replacement() {
    let dir = scratch("handoff-journal-selection-retained");
    let moved = dir.with_file_name("tine-handoff-journal-selection-retained-moved");
    let _ = fs::remove_dir_all(&moved);
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq/config.edn"),
        "{:preferred-format \"Org\"\n :journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
    )
    .unwrap();
    fs::create_dir_all(dir.join("journals/imported/deep")).unwrap();
    let title_named = "Thursday, 25-06-2026.org";
    fs::write(
        dir.join("journals/imported/deep").join(title_named),
        "* journal A\n",
    )
    .unwrap();
    let graph = Graph::open(&dir);
    MANAGED_WRITE_AFTER_ADMISSION.with(|hook| {
        let dir = dir.clone();
        let moved = moved.clone();
        *hook.borrow_mut() = Some(Box::new(move || {
            fs::rename(&dir, &moved)?;
            fs::create_dir_all(dir.join("pages"))?;
            fs::create_dir_all(dir.join("journals"))?;
            fs::write(dir.join("journals").join(title_named), "* title-named B\n")?;
            fs::write(dir.join("journals/2026_06_25.org"), "* canonical B\n")?;
            hold_replacement_handoff(&dir, 91_047_200)
        }));
    });

    assert_eq!(graph.migrate_journal_filenames_checked().unwrap(), 1);

    assert!(!moved
        .join("journals/imported/deep")
        .join(title_named)
        .exists());
    assert_eq!(
        fs::read(moved.join("journals/2026_06_25.org")).unwrap(),
        b"* journal A\n"
    );
    assert_eq!(
        fs::read(dir.join("journals").join(title_named)).unwrap(),
        b"* title-named B\n"
    );
    assert_eq!(
        fs::read(dir.join("journals/2026_06_25.org")).unwrap(),
        b"* canonical B\n"
    );

    release_replacement_handoff();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&moved);
}

#[cfg(unix)]
#[test]
fn save_force_save_and_delete_selection_stay_on_retained_a_after_reserved_b_replacement() {
    let dir = scratch("handoff-save-delete-selection-retained");
    let moved = dir.with_file_name("tine-handoff-save-delete-selection-retained-moved");
    let _ = fs::remove_dir_all(&moved);
    fs::create_dir_all(dir.join("pages/archive/deep")).unwrap();
    fs::write(dir.join("pages/Target.org"), "* target A\n").unwrap();
    fs::write(dir.join("pages/archive/deep/Victim.md"), "- victim A\n").unwrap();
    let graph = Graph::open(&dir);
    let mut page = graph.load_named("Target", PageKind::Page).unwrap().unwrap();
    let base_rev = page.rev.clone().unwrap();
    as_editor(&graph, &mut page);
    page.path.clear();
    page.blocks[0].raw = "saved A".to_owned();
    MANAGED_WRITE_AFTER_ADMISSION.with(|hook| {
        let dir = dir.clone();
        let moved = moved.clone();
        *hook.borrow_mut() = Some(Box::new(move || {
            fs::rename(&dir, &moved)?;
            fs::create_dir_all(dir.join("pages/other/deep"))?;
            fs::create_dir_all(dir.join("journals"))?;
            fs::write(dir.join("pages/Target.md"), "- target B\n")?;
            fs::write(dir.join("pages/other/deep/Victim.md"), "- victim B\n")?;
            hold_replacement_handoff(&dir, 91_047_300)
        }));
    });

    let saved_rev = graph.save_page(&page, Some(&base_rev)).unwrap();
    page.blocks[0].raw = "forced A".to_owned();
    fs::write(moved.join("pages/Target.org"), "* external A\n").unwrap();
    let shown = graph.save_page(&page, Some(&saved_rev)).unwrap_err();
    graph
        .force_save_page_at_revision(&page, Some(&saved_rev), gh254_shown(&shown))
        .unwrap();
    graph
        .delete_page_expected("Victim", PageKind::Page, None)
        .unwrap();

    assert_eq!(
        fs::read_to_string(moved.join("pages/Target.org")).unwrap(),
        "* forced A\n"
    );
    assert!(!moved.join("pages/Target.md").exists());
    assert!(!moved.join("pages/archive/deep/Victim.md").exists());
    assert_eq!(
        fs::read(dir.join("pages/Target.md")).unwrap(),
        b"- target B\n"
    );
    assert_eq!(
        fs::read(dir.join("pages/other/deep/Victim.md")).unwrap(),
        b"- victim B\n"
    );

    release_replacement_handoff();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&moved);
}

#[cfg(unix)]
#[test]
fn hls_selection_migration_and_legacy_cleanup_stay_on_retained_a_under_reserved_b() {
    let dir = scratch("handoff-hls-selection-retained");
    let moved = dir.with_file_name("tine-handoff-hls-selection-retained-moved");
    let _ = fs::remove_dir_all(&moved);
    let pdf = "My Paper.pdf";
    let legacy_key = crate::pdf::legacy_asset_key(pdf);
    let new_key = crate::pdf::asset_key(pdf);
    let highlight = mkhl("11111111-1111-1111-1111-111111111111", 3, Some("legacy"));
    let mut legacy_page =
        crate::pdf::hls_page_document(pdf, "Paper", std::slice::from_ref(&highlight));
    legacy_page.roots[0]
        .children
        .push(DocBlock::new("A-only private note"));
    fs::write(
        dir.join("pages")
            .join(format!("{}.md", crate::pdf::hls_page_name(&legacy_key))),
        doc::serialize(&legacy_page),
    )
    .unwrap();
    let graph = Graph::open(&dir);
    MANAGED_WRITE_AFTER_ADMISSION.with(|hook| {
        let dir = dir.clone();
        let moved = moved.clone();
        let b_page = format!("{}.md", crate::pdf::hls_page_name(&new_key));
        *hook.borrow_mut() = Some(Box::new(move || {
            fs::rename(&dir, &moved)?;
            fs::create_dir_all(dir.join("pages"))?;
            fs::create_dir_all(dir.join("journals"))?;
            fs::create_dir_all(dir.join("assets"))?;
            fs::write(dir.join("pages").join(&b_page), "- B-only hls page\n")?;
            fs::write(dir.join("assets/my_paper.pdf"), b"B collision input")?;
            hold_replacement_handoff(&dir, 91_047_500)
        }));
    });

    graph
        .write_highlights(
            pdf,
            "Paper",
            std::slice::from_ref(&highlight),
            std::slice::from_ref(&highlight.id),
        )
        .unwrap();

    let a_new = moved
        .join("pages")
        .join(format!("{}.md", crate::pdf::hls_page_name(&new_key)));
    assert!(fs::read_to_string(&a_new)
        .unwrap()
        .contains("A-only private note"));
    assert!(!moved
        .join("pages")
        .join(format!("{}.md", crate::pdf::hls_page_name(&legacy_key)))
        .exists());
    assert_eq!(
        fs::read(
            dir.join("pages")
                .join(format!("{}.md", crate::pdf::hls_page_name(&new_key)))
        )
        .unwrap(),
        b"- B-only hls page\n"
    );
    assert_eq!(
        fs::read(dir.join("assets/my_paper.pdf")).unwrap(),
        b"B collision input"
    );

    release_replacement_handoff();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&moved);
}

#[cfg(unix)]
#[test]
fn root_replacement_during_rename_rollback_restores_retained_a_and_leaves_b_untouched() {
    let dir = scratch("handoff-capability-rollback-window");
    let moved = dir.with_file_name("tine-handoff-capability-rollback-window-moved");
    let _ = fs::remove_dir_all(&moved);
    let old = dir.join("pages").join("Old.md");
    let reference = dir.join("pages").join("Reference.md");
    fs::write(&old, "- old A\n").unwrap();
    fs::write(&reference, "- [[Old]] on A\n").unwrap();
    let graph = Graph::open(&dir);
    FAIL_NEXT_RENAME_SOURCE_REMOVE.with(|flag| flag.set(true));
    MANAGED_WRITE_DURING_ROLLBACK.with(|hook| {
        let dir = dir.clone();
        let moved = moved.clone();
        *hook.borrow_mut() = Some(Box::new(move || {
            fs::rename(&dir, &moved)?;
            fs::create_dir_all(dir.join("pages"))?;
            fs::create_dir_all(dir.join("journals"))?;
            fs::write(dir.join("pages").join("Old.md"), "- old B\n")?;
            fs::write(dir.join("pages").join("New.md"), "- new B\n")?;
            fs::write(dir.join("pages").join("Reference.md"), "- [[Old]] on B\n")?;
            let replacement = Graph::open(&dir);
            let (workspace_id, endpoint) = handoff_binding(&replacement, 91_048);
            let handoff = replacement.mint_handoff_safe(workspace_id, endpoint)?;
            MANAGED_WRITE_REPLACEMENT_HANDOFF.with(|held| {
                *held.borrow_mut() = Some(handoff);
            });
            Ok(())
        }));
    });

    assert!(graph.rename_page("Old", "New").is_err());
    assert_eq!(
        fs::read(moved.join("pages").join("Old.md")).unwrap(),
        b"- old A\n"
    );
    assert!(!moved.join("pages").join("New.md").exists());
    assert_eq!(
        fs::read(moved.join("pages").join("Reference.md")).unwrap(),
        b"- [[Old]] on A\n"
    );
    assert_eq!(
        fs::read(dir.join("pages").join("Old.md")).unwrap(),
        b"- old B\n"
    );
    assert_eq!(
        fs::read(dir.join("pages").join("New.md")).unwrap(),
        b"- new B\n"
    );
    assert_eq!(
        fs::read(dir.join("pages").join("Reference.md")).unwrap(),
        b"- [[Old]] on B\n"
    );
    let replacement = Graph::open(&dir);
    assert_handoff_blocked(
        replacement.create_markdown_page_if_absent("reserved rollback B", "- blocked\n"),
    );
    MANAGED_WRITE_REPLACEMENT_HANDOFF.with(|held| drop(held.borrow_mut().take()));
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&moved);
}

#[cfg(unix)]
#[test]
fn handoff_resource_gate_survives_moved_root_reopen() {
    let dir = scratch("handoff-moved-reopen-gate");
    let moved = dir.with_file_name("tine-handoff-moved-reopen-gate");
    let _ = fs::remove_dir_all(&moved);
    let graph = Arc::new(Graph::open(&dir));
    let (workspace_id, endpoint) = handoff_binding(&graph, 91_050);
    let handoff = graph.mint_handoff_safe(workspace_id, endpoint).unwrap();

    fs::rename(&dir, &moved).unwrap();
    let reopened = Arc::new(Graph::open(&moved));
    assert!(Arc::ptr_eq(
        managed_write_gate(&graph),
        managed_write_gate(&reopened)
    ));
    handoff
        .verify_binding(&graph, workspace_id, endpoint)
        .unwrap();
    assert!(handoff
        .verify_binding(&reopened, workspace_id, endpoint)
        .is_err());

    let start = Arc::new(std::sync::Barrier::new(2));
    let writer = std::thread::spawn({
        let graph = Arc::clone(&reopened);
        let start = Arc::clone(&start);
        move || {
            start.wait();
            graph.create_markdown_page_if_absent("moved reopen", "- blocked\n")
        }
    });
    start.wait();
    assert_handoff_blocked(writer.join().unwrap());
    drop(handoff);
    assert!(reopened
        .create_markdown_page_if_absent("moved reopen released", "- admitted\n")
        .unwrap());
    let _ = fs::remove_dir_all(&moved);
}

#[cfg(any(unix, windows))]
#[test]
fn handoff_blocks_both_projection_recovery_entrypoints_before_the_page_lock() {
    let dir = scratch("handoff-projection-recovery");
    let graph = Graph::open(&dir);
    let path = "pages/recovery.md";
    let target = b"- retained target\n";
    graph.write_projection_exact(path, None, target).unwrap();
    let (workspace_id, endpoint) = handoff_binding(&graph, 91_060);
    let handoff = graph.mint_handoff_safe(workspace_id, endpoint).unwrap();

    assert_handoff_blocked(graph.recover_projection_exact(path, target));
    assert_handoff_blocked(graph.recover_removed_projection_exact(path, target));
    assert_eq!(fs::read(dir.join(path)).unwrap(), target);

    drop(handoff);
    graph.recover_projection_exact(path, target).unwrap();
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(any(unix, windows))]
#[test]
fn handoff_blocks_production_authority_consuming_projection_recovery_entrypoints() {
    let dir = scratch("handoff-production-projection-recovery");
    let receipts = dir.with_file_name("tine-handoff-production-projection-receipts");
    let _ = fs::remove_dir_all(&receipts);
    fs::create_dir_all(&receipts).unwrap();
    let graph = Graph::open(&dir);
    let workspace_id = WorkspaceId::from_uuid(Uuid::from_u128(91_061));
    let store =
        crate::oplog::projection_store::ProjectionReceiptStore::open(&receipts, workspace_id)
            .unwrap();

    let present_path = "pages/production-recovery.md";
    let present_target = b"- retained target\n";
    fs::write(dir.join(present_path), present_target).unwrap();
    let present_intent = crate::oplog::ProjectionIntent::new(
        workspace_id,
        crate::oplog::PageId::from_uuid(Uuid::from_u128(91_062)),
        ManagedPath::parse(present_path).unwrap(),
        crate::oplog::FrontierV2::default(),
        Vec::new(),
        crate::oplog::ProjectionPrecondition::Absent,
        crate::oplog::ProjectionTargetKind::Present,
        BlobDescription::of(present_target),
        Vec::new(),
    )
    .unwrap();
    store.publish_intent(&present_intent, None).unwrap();
    store.reserve_attempt(&present_intent).unwrap();
    let mut present_authority = store.begin_mutation(&present_intent, None).unwrap();
    let (_, endpoint) = handoff_binding(&graph, 91_063);
    let handoff = graph.mint_handoff_safe(workspace_id, endpoint).unwrap();
    assert_handoff_blocked(graph.recover_page_projection(
        present_path,
        None,
        present_target,
        &mut present_authority,
    ));
    drop(handoff);
    graph
        .recover_page_projection(present_path, None, present_target, &mut present_authority)
        .unwrap();

    let removed_path = "pages/production-removed-recovery.md";
    let removed_base = b"- retained base\n";
    fs::write(dir.join(removed_path), removed_base).unwrap();
    let removed_intent = crate::oplog::ProjectionIntent::new(
        workspace_id,
        crate::oplog::PageId::from_uuid(Uuid::from_u128(91_064)),
        ManagedPath::parse(removed_path).unwrap(),
        crate::oplog::FrontierV2::default(),
        Vec::new(),
        crate::oplog::ProjectionPrecondition::Base(BlobDescription::of(removed_base)),
        crate::oplog::ProjectionTargetKind::Absent,
        BlobDescription::of(&[]),
        Vec::new(),
    )
    .unwrap();
    store
        .publish_intent(&removed_intent, Some(removed_base))
        .unwrap();
    let removed_reservation = store.reserve_attempt(&removed_intent).unwrap();
    fs::rename(
        dir.join(removed_path),
        dir.join("pages")
            .join(removed_reservation.recovery_filename()),
    )
    .unwrap();
    let mut removed_authority = store.begin_mutation(&removed_intent, None).unwrap();
    let handoff = graph.mint_handoff_safe(workspace_id, endpoint).unwrap();
    assert_handoff_blocked(graph.recover_removed_page_projection(
        removed_path,
        removed_base,
        &mut removed_authority,
    ));
    drop(handoff);
    graph
        .recover_removed_page_projection(removed_path, removed_base, &mut removed_authority)
        .unwrap();

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&receipts);
}

#[cfg(any(unix, windows))]
#[test]
fn handoff_blocks_pdf_view_state_before_its_shared_page_lock() {
    let dir = scratch("handoff-pdf-view-state");
    let graph = Graph::open(&dir);
    graph.open_pdf("paper.pdf", "Paper").unwrap();
    let sidecar = dir
        .join("assets")
        .join(format!("{}.edn", crate::pdf::asset_key("paper.pdf")));
    let baseline = fs::read(&sidecar).unwrap();
    let (workspace_id, endpoint) = handoff_binding(&graph, 91_070);
    let handoff = graph.mint_handoff_safe(workspace_id, endpoint).unwrap();

    assert_handoff_blocked(graph.write_pdf_view_state("paper.pdf", 8, 1.75));
    assert_eq!(fs::read(&sidecar).unwrap(), baseline);

    drop(handoff);
    graph.write_pdf_view_state("paper.pdf", 8, 1.75).unwrap();
    assert_ne!(fs::read(&sidecar).unwrap(), baseline);
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(any(unix, windows))]
#[test]
fn handoff_mint_error_releases_a_writer_waiting_at_the_reservation() {
    let dir = scratch("handoff-mint-error-race");
    let graph = Arc::new(Graph::open(&dir));
    let (workspace_id, endpoint) = handoff_binding(&graph, 91_075);
    let rendezvous = Arc::new(std::sync::Barrier::new(2));
    let (writer_tx, writer_rx) = std::sync::mpsc::channel();
    let writer = std::thread::spawn({
        let graph = Arc::clone(&graph);
        let rendezvous = Arc::clone(&rendezvous);
        move || {
            rendezvous.wait();
            writer_tx
                .send(graph.create_markdown_page_if_absent("mint error blocked", "- blocked\n"))
        }
    });
    HANDOFF_MINT_AFTER_RESERVATION.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            rendezvous.wait();
            assert_handoff_blocked(writer_rx.recv().unwrap());
            Err(io::Error::other("injected handoff mint failure"))
        }));
    });

    assert!(graph.mint_handoff_safe(workspace_id, endpoint).is_err());
    writer.join().unwrap().unwrap();
    assert!(graph
        .create_markdown_page_if_absent("mint error released", "- admitted\n")
        .unwrap());
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(any(unix, windows))]
#[test]
fn handoff_cancel_and_drop_release_waiting_writers() {
    let dir = scratch("handoff-cancel-drop-race");
    let graph = Arc::new(Graph::open(&dir));
    let (workspace_id, endpoint) = handoff_binding(&graph, 91_080);
    assert_handoff_release_admits_waiting_writer(
        Arc::clone(&graph),
        graph.mint_handoff_safe(workspace_id, endpoint).unwrap(),
        "cancel",
        |handoff| handoff.cancel(),
    );
    assert_handoff_release_admits_waiting_writer(
        Arc::clone(&graph),
        graph.mint_handoff_safe(workspace_id, endpoint).unwrap(),
        "drop",
        drop,
    );
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(any(unix, windows))]
#[test]
fn handoff_transfer_keeps_the_reservation_during_a_writer_race() {
    let dir = scratch("handoff-transfer-race");
    let graph = Arc::new(Graph::open(&dir));
    let (workspace_id, endpoint) = handoff_binding(&graph, 91_090);
    let handoff = graph.mint_handoff_safe(workspace_id, endpoint).unwrap();
    let rendezvous = Arc::new(std::sync::Barrier::new(2));
    let (writer_tx, writer_rx) = std::sync::mpsc::channel();
    let writer = std::thread::spawn({
        let graph = Arc::clone(&graph);
        let rendezvous = Arc::clone(&rendezvous);
        move || {
            rendezvous.wait();
            writer_tx.send(graph.create_markdown_page_if_absent("transfer blocked", "- blocked\n"))
        }
    });
    HANDOFF_TRANSFER_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            rendezvous.wait();
            assert_handoff_blocked(writer_rx.recv().unwrap());
        }));
    });

    let guard = handoff.into_publisher_guard();
    guard
        .verify_binding(&graph, workspace_id, endpoint)
        .unwrap();
    writer.join().unwrap().unwrap();
    drop(guard);
    assert!(graph
        .create_markdown_page_if_absent("transfer released", "- admitted\n")
        .unwrap());
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(any(unix, windows))]
#[test]
fn handoff_seal_closes_the_inventory_to_writer_race() {
    let dir = scratch("handoff-inventory-race");
    let graph = Graph::open(&dir);
    let (workspace_id, endpoint) = handoff_binding(&graph, 91_000);

    // Capturing inventory alone is intentionally not a writer reservation:
    // this is the pre-handoff race the sealed capability must close.
    let inventory = graph.initial_shadow_raw_managed_text_inventory().unwrap();
    assert!(inventory.is_empty());
    assert!(graph
        .create_markdown_page_if_absent("before seal", "- visible change\n")
        .unwrap());

    let handoff = graph.mint_handoff_safe(workspace_id, endpoint).unwrap();
    assert_handoff_blocked(graph.create_markdown_page_if_absent("blocked", "- no\n"));
    assert_handoff_blocked(graph.save_page(
        &markdown_page_dto("blocked save", "blocked save", "- no\n").unwrap(),
        None,
    ));
    drop(handoff);
    assert!(graph
        .create_markdown_page_if_absent("after release", "- admitted\n")
        .unwrap());
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(any(unix, windows))]
#[test]
fn handoff_rejects_every_managed_page_and_journal_writer_entrypoint() {
    let dir = scratch("handoff-writer-entrypoints");
    let graph = Graph::open(&dir);
    let (workspace_id, endpoint) = handoff_binding(&graph, 91_100);
    let handoff = graph.mint_handoff_safe(workspace_id, endpoint).unwrap();
    let page = markdown_page_dto("blocked page", "blocked page", "- no\n").unwrap();
    let mut journal = markdown_page_dto("January 1st, 2026", "journal", "- no\n").unwrap();
    journal.kind = PageKind::Journal;

    assert_handoff_blocked(graph.create_markdown_page_if_absent("create", "- no\n"));
    assert_handoff_blocked(graph.save_page(&page, None));
    assert_handoff_blocked(graph.force_save_page_at_revision(
        &page,
        None,
        ConflictOverride {
            observation_epoch: 0,
        },
    ));
    assert_handoff_blocked(graph.save_page(&journal, None));
    assert_handoff_blocked(graph.force_save_page_at_revision(
        &journal,
        None,
        ConflictOverride {
            observation_epoch: 0,
        },
    ));
    assert_handoff_blocked(graph.rename_page_expected("old", "new", None));
    assert_handoff_blocked(graph.delete_page_expected("page", PageKind::Page, None));
    assert_handoff_blocked(graph.delete_page_expected(
        "January 1st, 2026",
        PageKind::Journal,
        None,
    ));
    assert_handoff_blocked(graph.resolve_sync_conflict(
        "pages/a.md",
        "pages/b.md",
        &Default::default(),
        "",
        "",
        None,
        "mine",
    ));
    assert_handoff_blocked(graph.merge_pages("pages/a.md", "pages/b.md"));
    assert_handoff_blocked(graph.rename_file_to_page("journals/a.md", "rescued"));
    assert_handoff_blocked(graph.trash_journal_file("a.md"));
    assert_handoff_blocked(graph.open_pdf("blocked.pdf", "blocked"));
    assert_handoff_blocked(graph.write_pdf_view_state("blocked.pdf", 1, 1.0));
    assert_handoff_blocked(graph.write_highlights("blocked.pdf", "blocked", &[], &[]));
    assert_handoff_blocked(graph.write_projection_exact("pages/projection.md", None, b"- no\n"));
    assert_handoff_blocked(graph.recover_projection_exact("pages/projection.md", b"- no\n"));
    assert_handoff_blocked(
        graph.recover_removed_projection_exact("pages/projection.md", b"- no\n"),
    );
    assert_handoff_blocked(graph.sync_file_checked(&dir.join("pages").join("watch.md")));
    assert_handoff_blocked(graph.sync_deleted_file(&dir.join("pages").join("deleted.md")));

    drop(handoff);
    assert!(graph
        .create_markdown_page_if_absent("writer released", "- yes\n")
        .unwrap());
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(any(unix, windows))]
#[test]
fn handoff_binding_rejects_reopened_copied_wrong_workspace_endpoint_and_resource() {
    let dir = scratch("handoff-binding");
    let graph = Graph::open(&dir);
    let (workspace_id, endpoint) = handoff_binding(&graph, 91_200);
    let handoff = graph.mint_handoff_safe(workspace_id, endpoint).unwrap();

    handoff
        .verify_binding(&graph, workspace_id, endpoint)
        .unwrap();
    assert_eq!(handoff.binding().workspace_id(), workspace_id);
    assert_eq!(handoff.binding().endpoint(), endpoint);
    assert_eq!(
        handoff.binding().graph_resource_id(),
        endpoint.graph_resource_id()
    );
    assert!(handoff
        .verify_binding(
            &graph,
            WorkspaceId::from_uuid(Uuid::from_u128(91_299)),
            endpoint,
        )
        .is_err());
    let wrong_endpoint = ProjectionEndpointBinding::enroll_graph(
        &graph,
        crate::oplog::ProjectionEndpointId::from_uuid(Uuid::from_u128(91_203)),
        crate::oplog::DeviceId::from_uuid(Uuid::from_u128(91_204)),
    )
    .unwrap();
    assert!(handoff
        .verify_binding(&graph, workspace_id, wrong_endpoint)
        .is_err());

    let reopened = Graph::open(&dir);
    assert!(handoff
        .verify_binding(&reopened, workspace_id, endpoint)
        .is_err());

    let copied_root = dir.with_file_name("tine-handoff-binding-copy");
    let _ = fs::remove_dir_all(&copied_root);
    fs::create_dir_all(copied_root.join("pages")).unwrap();
    fs::write(copied_root.join("pages").join("copied.md"), "- copied\n").unwrap();
    let copied = Graph::open(&copied_root);
    copied.warm_cache();
    assert_ne!(
        copied.canonical_resource_id().unwrap(),
        endpoint.graph_resource_id()
    );
    assert!(handoff
        .verify_binding(&copied, workspace_id, endpoint)
        .is_err());
    assert!(copied
        .create_markdown_page_if_absent("unrelated graph remains writable", "- yes\n")
        .unwrap());

    let foreign_endpoint = ProjectionEndpointBinding::enroll_graph(
        &copied,
        crate::oplog::ProjectionEndpointId::from_uuid(Uuid::from_u128(91_205)),
        crate::oplog::DeviceId::from_uuid(Uuid::from_u128(91_206)),
    )
    .unwrap();
    assert!(graph
        .mint_handoff_safe(workspace_id, foreign_endpoint)
        .is_err());
    drop(handoff);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&copied_root);
}

#[cfg(any(unix, windows))]
#[test]
fn handoff_raii_mint_failure_and_transfer_never_leave_an_unlocked_interval() {
    let dir = scratch("handoff-raii");
    let graph = Graph::open(&dir);
    let (workspace_id, endpoint) = handoff_binding(&graph, 91_300);

    let active_writer = graph.admit_managed_text_writer().unwrap();
    assert_handoff_blocked(graph.mint_handoff_safe(workspace_id, endpoint));
    drop(active_writer);

    HANDOFF_MINT_AFTER_RESERVATION.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(|| {
            Err(io::Error::new(
                io::ErrorKind::Other,
                "injected handoff mint failure",
            ))
        }));
    });
    assert!(graph.mint_handoff_safe(workspace_id, endpoint).is_err());
    assert!(graph
        .create_markdown_page_if_absent("released after mint error", "- yes\n")
        .unwrap());

    let cancelled = graph.mint_handoff_safe(workspace_id, endpoint).unwrap();
    cancelled.cancel();
    assert!(graph
        .create_markdown_page_if_absent("released after cancel", "- yes\n")
        .unwrap());

    let dropped = graph.mint_handoff_safe(workspace_id, endpoint).unwrap();
    drop(dropped);
    assert!(graph
        .create_markdown_page_if_absent("released after drop", "- yes\n")
        .unwrap());

    let handoff = graph.mint_handoff_safe(workspace_id, endpoint).unwrap();
    let gate = Arc::clone(managed_write_gate(&graph));
    HANDOFF_TRANSFER_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            assert_handoff_blocked(gate.admit_writer());
        }));
    });
    let guard = handoff.into_publisher_guard();
    guard
        .verify_binding(&graph, workspace_id, endpoint)
        .unwrap();
    assert_handoff_blocked(graph.create_markdown_page_if_absent("still held", "- no\n"));
    drop(guard);
    assert!(graph
        .create_markdown_page_if_absent("released after consume", "- yes\n")
        .unwrap());
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn handoff_retained_resource_identity_survives_supported_move_but_not_replacement() {
    let dir = scratch("handoff-resource-move");
    let moved = dir.with_file_name("tine-handoff-resource-moved");
    let _ = fs::remove_dir_all(&moved);
    let graph = Graph::open(&dir);
    let (workspace_id, endpoint) = handoff_binding(&graph, 91_400);
    let handoff = graph.mint_handoff_safe(workspace_id, endpoint).unwrap();

    fs::rename(&dir, &moved).unwrap();
    fs::create_dir_all(&dir).unwrap();
    handoff
        .verify_binding(&graph, workspace_id, endpoint)
        .unwrap();
    let replacement = Graph::open(&dir);
    assert_ne!(
        replacement.canonical_resource_id().unwrap(),
        endpoint.graph_resource_id()
    );
    assert!(handoff
        .verify_binding(&replacement, workspace_id, endpoint)
        .is_err());

    drop(handoff);
    graph
        .write_projection_exact("pages/retained-capability.md", None, b"- exact resource\n")
        .unwrap();
    assert_eq!(
        fs::read(moved.join("pages").join("retained-capability.md")).unwrap(),
        b"- exact resource\n"
    );
    assert!(!dir.join("pages").join("retained-capability.md").exists());
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&moved);
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
    graph
        .write_projection_exact("pages/still-bound.md", None, b"- retained\n")
        .unwrap();
    assert_eq!(
        fs::read(dir.join("pages/still-bound.md")).unwrap(),
        b"- retained\n"
    );

    drop(graph);
    crate::test_support::remove_dir_all(&dir);
}

fn bootstrap_capture_entries(capture: &BootstrapSourceCapture) -> Vec<BootstrapSourceEntry> {
    let mut cursor = capture.entries_cursor().unwrap();
    let mut entries = Vec::new();
    while let Some(entry) = cursor.next().unwrap() {
        entries.push(entry);
    }
    entries
}

#[derive(Debug, Eq, PartialEq)]
struct BootstrapCapturedSource {
    path: String,
    kind: ManagedTextKind,
    logical_name: String,
    chunk_lengths: Vec<u64>,
    bytes: Vec<u8>,
}

fn bootstrap_capture_sources(capture: &BootstrapSourceCapture) -> Vec<BootstrapCapturedSource> {
    let entries = bootstrap_capture_entries(capture);
    let mut chunks = capture.chunks_cursor().unwrap();
    let mut sources = Vec::with_capacity(entries.len());
    for entry in entries {
        let mut bytes = Vec::new();
        let mut chunk_lengths = Vec::with_capacity(entry.chunk_count() as usize);
        for ordinal in 0..entry.chunk_count() {
            let chunk = chunks.next().unwrap().unwrap();
            assert_eq!(chunk.path(), entry.path());
            assert_eq!(chunk.ordinal(), ordinal);
            let mut reader = capture.open_chunk(&chunk).unwrap();
            let mut chunk_bytes = Vec::new();
            reader.read_to_end(&mut chunk_bytes).unwrap();
            reader.finish().unwrap();
            chunk_lengths.push(chunk_bytes.len() as u64);
            bytes.extend_from_slice(&chunk_bytes);
        }
        assert_eq!(bytes.len() as u64, entry.description().byte_length());
        sources.push(BootstrapCapturedSource {
            path: entry.path().as_str().to_owned(),
            kind: entry.kind(),
            logical_name: entry.logical_name().to_owned(),
            chunk_lengths,
            bytes,
        });
    }
    assert!(chunks.next().unwrap().is_none());
    sources
}

#[test]
fn inactive_bootstrap_capture_preserves_exact_nested_unicode_org_and_semantic_kinds() {
    let root = scratch("bootstrap-source-paths");
    fs::create_dir_all(root.join("logseq")).unwrap();
    fs::write(
        root.join("logseq/config.edn"),
        r#"{:pages-directory "notes"
                :journals-directory "diary"
                :file/name-format :triple-lowbar
                :journal/file-name-format "dd-MM-yyyy"
                :journal/page-title-format "yyyy-MM-dd"}"#,
    )
    .unwrap();
    fs::create_dir_all(root.join("notes/arbitrary/nested")).unwrap();
    fs::create_dir_all(root.join("notes/深い")).unwrap();
    fs::create_dir_all(root.join("diary/nested")).unwrap();
    fs::write(
        root.join("Root.md"),
        "title:: Root logical name\n\n- root\n",
    )
    .unwrap();
    fs::write(
        root.join("notes/arbitrary/nested/Markdown.md"),
        "title:: Markdown title ☕\n\n- semantic page\n",
    )
    .unwrap();
    fs::write(
        root.join("notes/深い/Déjà___計画.markdown"),
        "- filename-derived page\n",
    )
    .unwrap();
    fs::write(
        root.join("diary/nested/25-07-2026.org"),
        "#+TITLE: Org title ��\n\n* semantic journal\n",
    )
    .unwrap();
    fs::write(root.join("diary/nested/26-07-2026.md"), "- journal\n").unwrap();
    let graph = Graph::open(&root);
    let capture_scratch = bootstrap_capture_scratch("paths");
    let capture = graph
        .capture_inactive_bootstrap_sources(&capture_scratch)
        .unwrap();
    let entries = bootstrap_capture_entries(&capture);
    assert_eq!(
        entries
            .iter()
            .map(|entry| {
                (
                    entry.path().as_str().to_owned(),
                    entry.kind(),
                    entry.logical_name().to_owned(),
                )
            })
            .collect::<Vec<_>>(),
        vec![
            (
                "Root.md".to_owned(),
                ManagedTextKind::Page,
                "Root logical name".to_owned(),
            ),
            (
                "diary/nested/25-07-2026.org".to_owned(),
                ManagedTextKind::Page,
                "Org title ��".to_owned(),
            ),
            (
                "diary/nested/26-07-2026.md".to_owned(),
                ManagedTextKind::Journal,
                "2026-07-26".to_owned(),
            ),
            (
                "notes/arbitrary/nested/Markdown.md".to_owned(),
                ManagedTextKind::Page,
                "Markdown title ☕".to_owned(),
            ),
            (
                "notes/深い/Déjà___計画.markdown".to_owned(),
                ManagedTextKind::Page,
                "Déjà/計画".to_owned(),
            ),
        ]
    );
    assert_eq!(capture.instrumentation().parser_calls, 5);
    let final_proof = capture
        .verify_before_inactive_bootstrap_authoring(&graph)
        .unwrap();
    assert_eq!(final_proof.parser_calls, 0);
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&capture_scratch);
}

#[test]
fn portable_bootstrap_identity_ignores_local_filesystem_identity_but_not_content() {
    let first_root = scratch("portable-bootstrap-identity-first");
    let second_root = scratch("portable-bootstrap-identity-second");
    for root in [&first_root, &second_root] {
        fs::create_dir_all(root.join("logseq")).unwrap();
        fs::write(
            root.join("logseq/config.edn"),
            r#"{:pages-directory "notes" :journals-directory "diary"}"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("notes/層")).unwrap();
        fs::write(
            root.join("notes/層/計画.md"),
            "title:: Shared 計画\n\n- café\n",
        )
        .unwrap();
    }
    let first_scratch = bootstrap_capture_scratch("portable-identity-first");
    let second_scratch = bootstrap_capture_scratch("portable-identity-second");
    let changed_scratch = bootstrap_capture_scratch("portable-identity-changed");
    let first = Graph::open(&first_root)
        .capture_inactive_bootstrap_sources(&first_scratch)
        .unwrap();
    let second_graph = Graph::open(&second_root);
    let second = second_graph
        .capture_inactive_bootstrap_sources(&second_scratch)
        .unwrap();

    assert_ne!(
        first.capture_identity().unwrap(),
        second.capture_identity().unwrap()
    );
    assert_eq!(
        first.portable_capture_identity().unwrap(),
        second.portable_capture_identity().unwrap()
    );

    fs::write(
        second_root.join("notes/層/計画.md"),
        "title:: Shared 計画\n\n- changed café\n",
    )
    .unwrap();
    let changed = Graph::open(&second_root)
        .capture_inactive_bootstrap_sources(&changed_scratch)
        .unwrap();
    assert_ne!(
        first.portable_capture_identity().unwrap(),
        changed.portable_capture_identity().unwrap()
    );

    let _ = fs::remove_dir_all(&first_root);
    let _ = fs::remove_dir_all(&second_root);
    let _ = fs::remove_dir_all(&first_scratch);
    let _ = fs::remove_dir_all(&second_scratch);
    let _ = fs::remove_dir_all(&changed_scratch);
}

#[test]
fn bootstrap_source_regular_file_sync_uses_supported_handle_access() {
    let root = scratch("bootstrap-source-file-sync");
    let path = root.join("capture-artifact");
    fs::write(&path, b"durable bootstrap artifact").unwrap();

    sync_bootstrap_source_regular_file(&path).unwrap();

    assert_eq!(fs::read(&path).unwrap(), b"durable bootstrap artifact");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn inactive_bootstrap_capture_is_deterministic_and_chunks_zero_one_and_many_files() {
    let root = scratch("bootstrap-source-chunks");
    fs::write(root.join("pages/empty.md"), b"").unwrap();
    fs::write(root.join("pages/one.md"), b"- one\n").unwrap();
    let mut many = b"- ".to_vec();
    many.extend(std::iter::repeat_n(b'x', BOOTSTRAP_SOURCE_CHUNK_BYTES * 2));
    many.extend_from_slice(b"\n");
    fs::write(root.join("pages/many.org"), &many).unwrap();
    let first_scratch = bootstrap_capture_scratch("chunks-first");
    let second_scratch = bootstrap_capture_scratch("chunks-second");
    let graph = Graph::open(&root);
    let first = graph
        .capture_inactive_bootstrap_sources(&first_scratch)
        .unwrap();
    let second = graph
        .capture_inactive_bootstrap_sources(&second_scratch)
        .unwrap();
    let first_sources = bootstrap_capture_sources(&first);
    let second_sources = bootstrap_capture_sources(&second);
    assert_eq!(first_sources, second_sources);
    assert_eq!(
        first_sources,
        vec![
            BootstrapCapturedSource {
                path: "pages/empty.md".to_owned(),
                kind: ManagedTextKind::Page,
                logical_name: "empty".to_owned(),
                chunk_lengths: vec![],
                bytes: vec![],
            },
            BootstrapCapturedSource {
                path: "pages/many.org".to_owned(),
                kind: ManagedTextKind::Page,
                logical_name: "many".to_owned(),
                chunk_lengths: vec![
                    BOOTSTRAP_SOURCE_CHUNK_BYTES as u64,
                    BOOTSTRAP_SOURCE_CHUNK_BYTES as u64,
                    3,
                ],
                bytes: many,
            },
            BootstrapCapturedSource {
                path: "pages/one.md".to_owned(),
                kind: ManagedTextKind::Page,
                logical_name: "one".to_owned(),
                chunk_lengths: vec![6],
                bytes: b"- one\n".to_vec(),
            },
        ]
    );
    assert!(first.instrumentation().peak_owned_rows < 20_000);
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&first_scratch);
    let _ = fs::remove_dir_all(&second_scratch);
}

#[test]
fn inactive_bootstrap_capture_seals_one_pass_and_final_proof_rejects_later_mutations() {
    for mutation in ["modify", "add", "delete", "rename"] {
        let root = scratch(&format!("bootstrap-source-mutation-{mutation}"));
        let source = root.join("pages/one.md");
        let added = root.join("pages/added.md");
        let renamed = root.join("pages/renamed.md");
        fs::write(&source, b"title:: Before\n\n- body\n").unwrap();
        let scratch = bootstrap_capture_scratch(&format!("mutation-{mutation}"));
        let graph = Graph::open(&root);
        BOOTSTRAP_SOURCE_CAPTURE_AFTER_INITIAL_PASS.with(|hook| {
            *hook.borrow_mut() = Some(Box::new({
                let source = source.clone();
                let added = added.clone();
                let renamed = renamed.clone();
                move || match mutation {
                    "modify" => fs::write(source, b"title:: After\n\n- body\n"),
                    "add" => fs::write(added, b"- added externally\n"),
                    "delete" => fs::remove_file(source),
                    "rename" => fs::rename(source, renamed),
                    _ => unreachable!(),
                }
            }));
        });
        let capture = graph.capture_inactive_bootstrap_sources(&scratch).unwrap();
        assert_eq!(capture.instrumentation().passes, 1);
        assert!(
            capture
                .verify_before_inactive_bootstrap_authoring(&graph)
                .is_err(),
            "final source proof admitted an external {mutation} after capture"
        );
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&scratch);
    }

    let root = scratch("bootstrap-source-final-proof-mutation");
    let source = root.join("pages/one.md");
    fs::write(&source, b"- before final proof\n").unwrap();
    let scratch = bootstrap_capture_scratch("final-proof-mutation");
    let graph = Graph::open(&root);
    let capture = graph.capture_inactive_bootstrap_sources(&scratch).unwrap();
    BOOTSTRAP_SOURCE_CAPTURE_BEFORE_FINAL_PROOF.with(|hook| {
        *hook.borrow_mut() = Some(Box::new({
            let source = source.clone();
            move || fs::write(source, b"- changed immediately before final proof\n")
        }));
    });
    assert!(capture
        .verify_before_inactive_bootstrap_authoring(&graph)
        .is_err());
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&scratch);
}

/// A capture-change refusal has to say WHAT moved.
///
/// It reaches the user as a `Retryable` activation failure and nothing
/// else: on CI 32108957903 the whole report was `Retryable { durable_stage:
/// Absent, detail: "source capture changed before final inventory proof" }`
/// — not the path, not the field, not whether one row moved or a thousand.
/// Both directions are pinned. A file that merely APPEARED matters most:
/// it changes no source-file and no source-chunk count, so every other
/// check in the final proof is blind to it.
#[test]
fn final_proof_refusal_names_the_rows_that_changed() {
    for (mutation, expect) in [
        ("modify", "changed:"),
        ("add-source", "appeared:"),
        ("add-other", "appeared:"),
        ("delete", "gone:"),
    ] {
        let root = scratch(&format!("bootstrap-source-named-change-{mutation}"));
        let source = root.join("pages/one.md");
        let second = root.join("pages/two.md");
        fs::write(&source, b"- before the final proof\n").unwrap();
        fs::write(&second, b"- untouched\n").unwrap();
        let capture_scratch = bootstrap_capture_scratch(&format!("named-change-{mutation}"));
        let graph = Graph::open(&root);
        let capture = graph
            .capture_inactive_bootstrap_sources(&capture_scratch)
            .unwrap();
        BOOTSTRAP_SOURCE_CAPTURE_BEFORE_FINAL_PROOF.with(|hook| {
            *hook.borrow_mut() = Some(Box::new({
                let source = source.clone();
                let root = root.clone();
                move || match mutation {
                    "modify" => fs::write(source, b"- changed under the activation\n"),
                    "add-source" => fs::write(root.join("pages/three.md"), b"- new page\n"),
                    "add-other" => fs::write(root.join("pages/notes.txt"), b"not a page\n"),
                    "delete" => fs::remove_file(source),
                    _ => unreachable!(),
                }
            }));
        });
        let error = capture
            .verify_before_inactive_bootstrap_authoring(&graph)
            .expect_err("the final proof must refuse a graph that moved");
        let detail = error.to_string();
        if mutation == "add-other" {
            // The only mutation of the four that leaves both counts equal,
            // so the inventory report is the ONLY thing that localises it.
            assert!(
                detail.contains("source capture changed before final inventory proof"),
                "{mutation}: {detail}"
            );
            assert!(detail.contains("pages/notes.txt"), "{mutation}: {detail}");
            assert!(detail.contains("1 row(s) differ"), "{mutation}: {detail}");
        }
        if mutation == "modify" {
            assert!(detail.contains("pages/one.md"), "{mutation}: {detail}");
            assert!(detail.contains("content:"), "{mutation}: {detail}");
        }
        if detail.contains("inventory proof") {
            assert!(detail.contains(expect), "{mutation}: {detail}");
        } else {
            // A count mismatch is caught before the spools are compared, so
            // it reports the counts rather than the rows.
            assert!(
                detail.contains("source files") && detail.contains("source chunks"),
                "{mutation}: {detail}"
            );
        }
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&capture_scratch);
    }
}

/// A named row is worth nothing if the name cannot be told from its twin.
///
/// CI 32115065229 reported `changed: file pages/\u{17d} pilot notes
/// #pilot.md … -> file pages/\u{17d} pilot notes #pilot.md …` for a graph
/// holding TWO files whose names differ only by Unicode normalization
/// (`U+017D` against `Z` + `U+030C`). Both spellings print as the same
/// glyph sequence in every log, so the refusal could not say which file
/// moved, and the first reading of that evidence — content normalization —
/// was wrong. The report escapes non-ASCII; ASCII paths are untouched.
#[test]
fn a_capture_change_refusal_distinguishes_two_spellings_of_one_glyph() {
    let root = scratch("bootstrap-source-normalization-twins");
    let precomposed = root.join("pages/\u{17d} pilot notes.md");
    let decomposed = root.join("pages/Z\u{30c} pilot notes.md");
    fs::write(&precomposed, b"- precomposed\n").unwrap();
    fs::write(&decomposed, b"- decomposed\n").unwrap();
    let capture_scratch = bootstrap_capture_scratch("normalization-twins");
    let graph = Graph::open(&root);
    let capture = graph
        .capture_inactive_bootstrap_sources(&capture_scratch)
        .unwrap();
    BOOTSTRAP_SOURCE_CAPTURE_BEFORE_FINAL_PROOF.with(|hook| {
        *hook.borrow_mut() = Some(Box::new({
            let decomposed = decomposed.clone();
            move || fs::write(decomposed, b"- decomposed, changed under the activation\n")
        }));
    });
    let detail = capture
        .verify_before_inactive_bootstrap_authoring(&graph)
        .expect_err("the final proof must refuse a graph that moved")
        .to_string();
    assert!(detail.contains("1 row(s) differ"), "{detail}");
    assert!(
        detail.contains("pages/Z\\u{30c} pilot notes.md"),
        "the refusal must name the DECOMPOSED twin as the row that moved: {detail}"
    );
    assert!(
        !detail.contains("pages/\u{17d} pilot notes.md")
            && !detail.contains("pages/Z\u{30c} pilot notes.md"),
        "no raw glyph spelling may reach the refusal, or the twins read alike: {detail}"
    );
    // An ASCII path is still reported exactly as it is on disk.
    assert_eq!(
        bootstrap_source_change_report_path("pages/one.md"),
        "pages/one.md"
    );
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&capture_scratch);
}

#[test]
fn inactive_bootstrap_capture_rejects_bad_logical_name_frames() {
    let mut truncated = BootstrapSourceEncoder::new(3);
    truncated.string("pages/logical.md").unwrap();
    truncated.u8(managed_text_kind_tag(ManagedTextKind::Page));
    truncated.u32(4);
    truncated.bytes.extend_from_slice(b"na");
    assert!(decode_bootstrap_source_entry(&truncated.finish()).is_err());

    let mut non_utf8 = BootstrapSourceEncoder::new(3);
    non_utf8.string("pages/logical.md").unwrap();
    non_utf8.u8(managed_text_kind_tag(ManagedTextKind::Page));
    non_utf8.u32(1);
    non_utf8.bytes.push(0xff);
    assert!(decode_bootstrap_source_entry(&non_utf8.finish()).is_err());

    let mut oversized = BootstrapSourceEncoder::new(3);
    oversized.string("pages/logical.md").unwrap();
    oversized.u8(managed_text_kind_tag(ManagedTextKind::Page));
    oversized
        .string(&"x".repeat(BOOTSTRAP_SOURCE_MAX_LOGICAL_NAME_BYTES as usize + 1))
        .unwrap();
    assert!(decode_bootstrap_source_entry(&oversized.finish()).is_err());
}

#[test]
fn inactive_bootstrap_capture_exact_64_mib_sparse_file_is_accepted() {
    let root = scratch("bootstrap-source-64mib");
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join("pages/boundary.md"))
        .unwrap();
    file.set_len(BOOTSTRAP_SOURCE_MAX_FILE_BYTES).unwrap();
    drop(file);
    let scratch = bootstrap_capture_scratch("64mib");
    let graph = Graph::open(&root);
    let capture = graph.capture_inactive_bootstrap_sources(&scratch).unwrap();
    let entry = bootstrap_capture_entries(&capture).pop().unwrap();
    assert_eq!(
        entry.description().byte_length(),
        BOOTSTRAP_SOURCE_MAX_FILE_BYTES
    );
    assert_eq!(entry.chunk_count(), 64);
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&scratch);
}

#[test]
fn inactive_bootstrap_capture_ignores_residue_is_idempotent_and_rejects_conflicting_seal() {
    let root = scratch("bootstrap-source-seal");
    fs::write(root.join("pages/one.md"), b"- one\n").unwrap();
    let scratch = bootstrap_capture_scratch("seal");
    fs::create_dir(scratch.join(BOOTSTRAP_SOURCE_CAPTURE_DIRECTORY)).unwrap();
    fs::create_dir(
        scratch
            .join(BOOTSTRAP_SOURCE_CAPTURE_DIRECTORY)
            .join(".working-crash-residue"),
    )
    .unwrap();
    fs::write(
        scratch
            .join(BOOTSTRAP_SOURCE_CAPTURE_DIRECTORY)
            .join(".working-crash-residue/unsealed"),
        b"residue",
    )
    .unwrap();
    let graph = Graph::open(&root);
    BOOTSTRAP_SOURCE_CAPTURE_BEFORE_SEAL_RENAME.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(|| {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "injected pre-rename capture crash",
            ))
        }));
    });
    assert!(graph.capture_inactive_bootstrap_sources(&scratch).is_err());
    let first = graph.capture_inactive_bootstrap_sources(&scratch).unwrap();
    let second = graph.capture_inactive_bootstrap_sources(&scratch).unwrap();
    assert_eq!(
        bootstrap_capture_sources(&first),
        bootstrap_capture_sources(&second)
    );
    fs::write(
        first.sealed_directory.join(BOOTSTRAP_SOURCE_MANIFEST),
        b"conflict",
    )
    .unwrap();
    assert!(graph.capture_inactive_bootstrap_sources(&scratch).is_err());
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&scratch);
}

#[test]
fn inactive_bootstrap_capture_rejects_file_cap_before_streaming() {
    let root = scratch("bootstrap-source-over-file-cap");
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join("pages/too-large.md"))
        .unwrap();
    file.set_len(BOOTSTRAP_SOURCE_MAX_FILE_BYTES + 1).unwrap();
    drop(file);
    let scratch = bootstrap_capture_scratch("over-file-cap");
    assert!(Graph::open(&root)
        .capture_inactive_bootstrap_sources(&scratch)
        .is_err());
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&scratch);
}

#[cfg(unix)]
#[test]
fn inactive_bootstrap_capture_rejects_source_symlinks_and_hard_links() {
    use std::ffi::OsString;
    use std::os::unix::{ffi::OsStringExt, fs::symlink};

    let root = scratch("bootstrap-source-links");
    fs::write(root.join("pages/one.md"), b"- one\n").unwrap();
    fs::hard_link(root.join("pages/one.md"), root.join("pages/two.md")).unwrap();
    let scratch = bootstrap_capture_scratch("links");
    assert!(Graph::open(&root)
        .capture_inactive_bootstrap_sources(&scratch)
        .is_err());
    fs::remove_file(root.join("pages/two.md")).unwrap();
    symlink(root.join("pages/one.md"), root.join("pages/link.md")).unwrap();
    assert!(Graph::open(&root)
        .capture_inactive_bootstrap_sources(&scratch)
        .is_err());
    fs::remove_file(root.join("pages/link.md")).unwrap();
    fs::write(
        root.join("pages")
            .join(OsString::from_vec(b"non-utf8-\xff.md".to_vec())),
        b"- invalid path\n",
    )
    .unwrap();
    assert!(Graph::open(&root)
        .capture_inactive_bootstrap_sources(&scratch)
        .is_err());
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&scratch);
}

#[test]
fn inactive_bootstrap_capture_external_sort_is_buffer_bounded_without_real_files() {
    const SYNTHETIC_ROWS: u64 = 1_000_000;
    let root = scratch("bootstrap-source-streaming-scale");
    let working = root.join("synthetic-working");
    fs::create_dir(&working).unwrap();
    let paths = BootstrapSourcePassPaths::new(&working, "synthetic").unwrap();
    let mut writers = BootstrapSourcePassWriters::create(&paths).unwrap();
    let mut instrumentation = BootstrapSourceCaptureInstrumentation::default();
    for index in (0..SYNTHETIC_ROWS as u32).rev() {
        let path = ManagedPath::parse(format!("pages/{index:08}.md")).unwrap();
        write_bootstrap_source_entry(
            &mut writers.entries,
            &BootstrapSourceEntry {
                path,
                kind: ManagedTextKind::Page,
                logical_name: format!("Synthetic logical name {index:08}"),
                description: BlobDescription::from_parts([0; 32], 0),
                file_resource: ContentDigest::from_bytes([1; 32]),
                link_count: 1,
                chunk_count: 0,
                activation_page: BlobDescription::from_parts([2; 32], 0),
            },
            &mut instrumentation,
        )
        .unwrap();
    }
    writers.sync_to_stable_storage().unwrap();
    sort_bootstrap_source_spool(
        &paths,
        BootstrapSourceSpoolKind::Entries,
        &mut instrumentation,
    )
    .unwrap();
    assert!(instrumentation.peak_owned_buffer_bytes <= BOOTSTRAP_SOURCE_SORT_BUFFER_BYTES as u64);
    assert!(instrumentation.sort_runs > 1);
    assert!(instrumentation.peak_owned_rows < SYNTHETIC_ROWS);
    validate_bootstrap_source_sorted_entries(
        &paths.sorted(BootstrapSourceSpoolKind::Entries),
        SYNTHETIC_ROWS,
    )
    .unwrap();
    let _ = fs::remove_dir_all(&root);
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
    assert!(reopened
        .resolve_durable_live_save_conflict(&page, &capture.disk_rev, &decisions, "union",)
        .is_err());

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
    MANAGED_WRITE_BEFORE_MUTATION.with(|hook| {
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

/// Rename resets the frontend's whole working set: reference rewrites can
/// make every mounted page stale, not only the page whose file moved.  The
/// backend Graph remains the same object, so it must explicitly burn every
/// activation after success rather than relying on Graph destruction.
#[test]
fn gh254_successful_rename_burns_all_editor_activations_but_failure_does_not() {
    let root = scratch("gh254-inc3-rename-activation-lifecycle");
    let note_path = root.join("pages/Note.md");
    let other_path = root.join("pages/Other.md");
    fs::write(&note_path, "- [[Other]]\n").unwrap();
    fs::write(&other_path, "- other\n").unwrap();
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
    assert!(
        !graph.retire_editor_activation("pages/Other.md", other.activation),
        "a reference-rewritten satellite editor must be retired too"
    );
    let reopened = graph
        .activate_editor("pages/Other.md", ActivationIntent::Reuse, None)
        .unwrap();
    assert_ne!(
        reopened.activation, other.activation,
        "Reuse after the reset must mint for the new editor instance"
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
    MANAGED_WRITE_BEFORE_MUTATION.with(|hook| {
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
    MANAGED_WRITE_BEFORE_MUTATION.with(|hook| {
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
    MANAGED_WRITE_AFTER_RETIRE.with(|hook| {
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
    MANAGED_WRITE_BEFORE_MUTATION.with(|hook| {
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
        MANAGED_WRITE_AFTER_RETIRE.with(|hook| {
            let path = path.clone();
            *hook.borrow_mut() = Some(Box::new(move || fs::write(path, "- s6 transient\n")));
        });
        if restore_succeeds {
            MANAGED_WRITE_BEFORE_RESTORE.with(|hook| {
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
    MANAGED_WRITE_BEFORE_MUTATION.with(|hook| {
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
    MANAGED_WRITE_BEFORE_MUTATION.with(|hook| {
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
// guard walks `model.rs`'s call graph and fails on any such function.
#[test]
fn graph_text_writers_take_the_identity_gate_before_any_page_lock() {
    use std::collections::{HashMap, HashSet};

    let source = include_str!("model.rs");
    let lines: Vec<&str> = source.lines().collect();
    let is_fn_start = |line: &str| {
        line.starts_with("    fn ")
            || line.starts_with("    pub fn ")
            || line.starts_with("    pub(crate) fn ")
    };
    let mut starts: Vec<usize> = (0..lines.len())
        .filter(|i| is_fn_start(lines[*i]))
        .collect();
    starts.push(lines.len());
    assert!(
        starts.len() > 400,
        "the function scan found only {} candidates; the source shape changed",
        starts.len()
    );

    fn name_of(line: &str) -> &str {
        let rest = line
            .trim_start()
            .trim_start_matches("pub(crate) ")
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
        let (a, b) = (pair[0], pair[1]);
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
    for expected in [
        "save_page",
        "force_save_page_at_revision",
        "merge_pages",
        "write_page_projection_with_attempts",
    ] {
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
        let (a, b) = (pair[0], pair[1]);
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
            "{} (model.rs:{}) holds a page lock and then reaches the identity gate via {reaching:?}",
            name_of(lines[a]),
            a + 1
        ));
    }
    assert!(
        checked >= 25,
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

#[test]
fn a_key_page_save_advances_the_registry_generation_and_an_unrelated_save_does_not() {
    let dir = registry_graph("registry-key-page-save");
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let before = graph.property_registry_generation();

    // A page that is not a property key: its rows are covered by the page-cache
    // generation, and nothing about the key's declarations moved.
    let unrelated = graph
        .load_named("Unrelated", PageKind::Page)
        .unwrap()
        .unwrap();
    graph
        .save_page(&unrelated, unrelated.rev.as_deref())
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        graph.property_registry_generation(),
        before,
        "an unrelated save changes no row and no declaration"
    );

    // The key page. Its `tine.type::` is what the walk coerces by, so the
    // snapshot has to be rebuilt even though no property ROW changed.
    let mut key_page = graph.load_named("score", PageKind::Page).unwrap().unwrap();
    key_page.blocks[0].raw = "the key page, reworded".into();
    key_page.pre_block = Some("tine.type:: text".to_string());
    graph.save_page(&key_page, key_page.rev.as_deref()).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        graph.property_registry_generation() > before,
        "a declaration change advances the generation"
    );
    assert_eq!(
        graph
            .property_registry()
            .effective_type("score")
            .expect("the key is in the registry"),
        crate::query::ir::ObservedType::Text,
        "the declared type overrides the observed one (§6.3)"
    );
    let _ = fs::remove_dir_all(dir);
}

/// SPEC §5.9 guard (b): a declared-type change on a key page evicts every
/// cached typed query over that key.
///
/// The edit is deliberately a page the query does not participate in, so
/// per-page retention (`page_affects_query`) keeps the entry and only the
/// registry generation can evict it. The answer flips because `01` is the
/// number 1 under a number key and the text `01` under a text key.
#[test]
fn a_declared_type_change_evicts_the_cached_typed_query_it_retypes() {
    let dir = registry_graph("registry-declared-type-eviction");
    let graph = Graph::open(&dir);
    graph.warm_cache();

    let matched = |graph: &Graph| -> usize {
        graph
            .run_query_bounded("(property score 1)", usize::MAX, usize::MAX)
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
    std::thread::sleep(Duration::from_millis(300));

    assert_eq!(
        matched(&graph),
        0,
        "under a text key `01` is not `1` -- a served stale entry would still say 1"
    );
    let _ = fs::remove_dir_all(dir);
}

/// SPEC §5.9 guard (c), Direct Files half (E6): a query with NO property leaf
/// is still config-sensitive, because `journal_page_title_format` decides
/// whether a page in `pages/` is a journal day at all. The config is read at
/// open, so the guard is a reopen — and the digest is what keeps the two
/// answers apart in the cache.
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
        graph
            .run_query_bounded("(between -10y +10y)", usize::MAX, usize::MAX)
            .groups
            .iter()
            .map(|group| group.blocks.len())
            .sum()
    };

    let before = Graph::open(&dir);
    before.warm_cache();
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
    let after = Graph::open(&dir);
    after.warm_cache();
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

/// Two readers racing the rebuild never see a half-built table: every snapshot
/// handed out is internally complete, and one generation always means one set
/// of rows.
#[test]
fn concurrent_readers_never_observe_a_half_built_registry() {
    let dir = registry_graph("registry-concurrent-readers");
    let graph = Arc::new(Graph::open(&dir));
    graph.warm_cache();

    let observations = Arc::new(std::sync::Mutex::new(Vec::<(u64, Vec<String>)>::new()));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let graph = Arc::clone(&graph);
        let observations = Arc::clone(&observations);
        handles.push(std::thread::spawn(move || {
            for _ in 0..40 {
                let snapshot = graph.property_registry();
                let shape = snapshot
                    .rows()
                    .iter()
                    .map(|row| {
                        format!(
                            "{}:{:?}:{}",
                            row.normalized_name, row.observed_type, row.count_blocks
                        )
                    })
                    .collect::<Vec<_>>();
                // A snapshot is never empty-but-generation-advanced: the key is
                // in every published table this graph can produce.
                assert!(
                    shape.iter().any(|row| row.starts_with("score:")),
                    "a published snapshot always carries the graph's keys: {shape:?}"
                );
                observations
                    .lock()
                    .unwrap()
                    .push((snapshot.generation(), shape));
            }
        }));
    }
    for handle in handles {
        handle.join().unwrap();
    }

    let observations = observations.lock().unwrap();
    let mut by_generation: std::collections::BTreeMap<u64, Vec<String>> =
        std::collections::BTreeMap::new();
    for (generation, shape) in observations.iter() {
        match by_generation.get(generation) {
            Some(seen) => assert_eq!(
                seen, shape,
                "generation {generation} was published with two different tables"
            ),
            None => {
                by_generation.insert(*generation, shape.clone());
            }
        }
    }
    let _ = fs::remove_dir_all(dir);
}

/// O11: the registry built from the Direct Files document iterator and the one
/// built from the Managed per-page owner rows are the same table. The two
/// producers disagree only about opaque owner identity, which the aggregation
/// deliberately does not carry.
#[test]
fn every_implemented_row_source_builds_the_same_registry() {
    let dir = registry_graph("registry-source-identity");
    fs::write(
        dir.join("pages/More.md"),
        "tags:: alpha, beta\n\n- another row\n  score:: 7\n  note:: [[Book]], plain\n",
    )
    .unwrap();
    let graph = Graph::open(&dir);
    graph.warm_cache();
    let config = graph.config.parse_config();

    let (document_rows, document_pages) = crate::query::property_owner_rows(&graph);
    let from_documents = crate::query::registry::build_registry(
        document_rows.into_iter(),
        &|page_id| document_pages.get(page_id).cloned(),
        &config,
    )
    .expect("every document row names its own page");

    // The Managed producer's shape: one page DTO at a time, page properties
    // included, keyed by an opaque per-snapshot page id.
    let mut managed_rows = Vec::new();
    let mut managed_pages = std::collections::HashMap::new();
    graph.with_pages(|entries| {
        for (entry, _) in entries {
            let page = graph
                .load_named(&entry.name, entry.kind)
                .unwrap()
                .expect("a page the inventory listed loads");
            let page_id = format!("managed:{}", entry.rel_path);
            managed_pages.insert(
                page_id.clone(),
                crate::query::registry::PageMeta {
                    format: Format::from_path(Path::new(&entry.rel_path)).into(),
                    name: entry.name.clone(),
                },
            );
            managed_rows.extend(crate::query::application_page_property_owner_rows(
                &page, &page_id, true,
            ));
        }
    });
    let from_managed = crate::query::registry::build_registry(
        managed_rows.into_iter(),
        &|page_id| managed_pages.get(page_id).cloned(),
        &config,
    )
    .expect("every managed row names its own page");

    assert_eq!(
        from_documents.rows(),
        from_managed.rows(),
        "the two implemented row sources build one table"
    );
    assert!(
        from_documents
            .rows()
            .iter()
            .any(|row| row.normalized_name == "score"),
        "the fixture's key is actually in the table"
    );
    let _ = fs::remove_dir_all(dir);
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
