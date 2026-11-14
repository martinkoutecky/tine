use super::*;
use clap::CommandFactory;

#[test]
fn comment_exclusion_is_an_explicit_cli_export_choice() {
    let Some(Command::Export {
        format: ExportFormat::Static(args),
    }) = Cli::try_parse_from([
        "tine",
        "export",
        "static",
        "g",
        "--output",
        "/o",
        "--no-comments",
    ])
    .unwrap()
    .command
    else {
        panic!("static export");
    };
    assert!(args.no_comments);
    let Some(Command::Export {
        format: ExportFormat::Query(args),
    }) = Cli::try_parse_from([
        "tine",
        "export",
        "query",
        "g",
        "11111111-1111-4111-8111-111111111111",
        "--no-comments",
    ])
    .unwrap()
    .command
    else {
        panic!("query export");
    };
    assert!(query_export_args(&args).unwrap().no_comments);
}

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_owned()).collect()
}

#[test]
fn gh840_only_the_flag_sets_author_on_edit() {
    assert!(edit_author_is_a_flag([
        "tine",
        "edit",
        "A/1~abcdef",
        "x",
        "--author",
        "Ada"
    ]));
    assert!(!edit_author_is_a_flag(["tine", "edit", "A/1~abcdef", "x"]));
    assert!(!edit_author_is_a_flag([
        "tine", "add", "--page", "P", "--author", "Ada", "x"
    ]));
}

#[test]
fn page_current_parses_without_a_page_name() {
    Cli::try_parse_from(["tine", "page", "current"]).unwrap();
    Cli::try_parse_from(["tine", "page", "current", "--graph", "/g", "--json"]).unwrap();
    assert!(Cli::try_parse_from(["tine", "page", "current", "extra"]).is_err());
}

#[test]
fn cli_delete_confirmation_flags_parse() {
    for flag in ["--yes", "--force"] {
        let matches = Cli::command()
            .try_get_matches_from(["tine", "delete", "a/1~abcdef", flag])
            .unwrap();
        assert_eq!(matches.subcommand().unwrap().1.get_flag("yes"), true);
    }
    Cli::try_parse_from(["tine", "delete", "a/1~abcdef"]).unwrap();
}

#[test]
fn page_verbs_and_reference_helpers_parse_as_terminal_writes() {
    for form in [
        vec!["tine", "page", "create", "A/B"],
        vec!["tine", "page", "rename", "A", "a"],
        vec!["tine", "page", "delete", "A", "--yes", "--rev", "revision"],
        vec!["tine", "page", "delete", "A", "--force"],
        vec!["tine", "ref", "A/1~abcdef", "--json"],
        vec!["tine", "embed", "A/1~abcdef", "--author", "Ada"],
    ] {
        Cli::try_parse_from(&form).unwrap();
        assert_eq!(
            launch_request(&argv(&form), Path::new("/")),
            LaunchRequest::Focus
        );
    }
    assert!(Cli::try_parse_from(["tine", "page", "rename", "A"]).is_err());
    assert!(Cli::try_parse_from(["tine", "ref"]).is_err());
}

#[test]
fn rename_help_discloses_unsupported_revision_and_run_options() {
    let mut command = Cli::command();
    let rename = command
        .find_subcommand_mut("page")
        .unwrap()
        .find_subcommand_mut("rename")
        .unwrap();
    for option in ["rev", "run"] {
        let help = rename
            .get_arguments()
            .find(|arg| arg.get_id() == option)
            .unwrap()
            .get_help()
            .unwrap()
            .to_string();
        assert!(help.contains("Unsupported for graph-wide rename"));
        assert!(help.contains("refuses without changes"));
    }
}

#[test]
fn legacy_and_named_gui_launches_share_one_parser() {
    let cwd = Path::new("/graphs");
    for form in [
        &["tine", "notes"][..],
        &["tine", "open", "notes"],
        &["tine", "--debug", "notes"],
    ] {
        assert_eq!(
            launch_request(&argv(form), cwd),
            LaunchRequest::Open(PathBuf::from("/graphs/notes")),
            "{form:?}"
        );
    }
    assert_eq!(
        launch_request(&argv(&["tine", "open", "/abs/notes"]), cwd),
        LaunchRequest::Open(PathBuf::from("/abs/notes"))
    );
    for form in [
        &["tine", "capture"][..],
        &["tine", "--capture"],
        &["tine", "--debug", "capture"],
    ] {
        assert_eq!(
            launch_request(&argv(form), cwd),
            LaunchRequest::Capture,
            "{form:?}"
        );
    }
    assert_eq!(launch_request(&argv(&["tine"]), cwd), LaunchRequest::Focus);
    assert_eq!(
        launch_request(&argv(&["tine", "--bogus"]), cwd),
        LaunchRequest::Focus
    );
}

#[test]
fn a_graph_folder_named_like_a_command_is_reachable_with_open() {
    let cwd = Path::new("/graphs");
    assert_eq!(
        launch_request(&argv(&["tine", "open", "export"]), cwd),
        LaunchRequest::Open(PathBuf::from("/graphs/export"))
    );
}

#[test]
fn terminal_commands_never_request_a_gui_action() {
    for form in [
        &["tine", "doctor", "g"][..],
        &["tine", "export", "static", "g", "--output", "/o"],
        &["tine", "version"],
    ] {
        assert_eq!(
            launch_request(&argv(form), Path::new("/")),
            LaunchRequest::Focus,
            "{form:?}"
        );
    }
}

#[test]
fn add_accepts_each_target_and_stdin_and_rejects_multiple_targets() {
    for form in [
        vec!["tine", "add", "--under", "P/1~abcdef", "-"],
        vec!["tine", "add", "--after", "P/1~abcdef", "- root"],
        vec!["tine", "add", "--before", "P/1~abcdef", "text"],
        vec!["tine", "add", "--page", "P", "--author", "Ada", "text"],
        vec!["tine", "add", "--journal=2026-10-18", "text"],
        vec!["tine", "add", "text", "--journal"],
        vec!["tine", "add", "--journal", "text"],
    ] {
        assert!(
            matches!(
                Cli::try_parse_from(&form).unwrap().command,
                Some(Command::Add { .. })
            ),
            "{form:?}"
        );
    }
    assert!(Cli::try_parse_from([
        "tine",
        "add",
        "--under",
        "P/1~abcdef",
        "--page",
        "P",
        "text"
    ])
    .is_err());
    assert!(Cli::try_parse_from(["tine", "add", "text"]).is_err());
    let Some(Command::Add { content, place, .. }) =
        Cli::try_parse_from(["tine", "add", "--journal", "text"])
            .unwrap()
            .command
    else {
        panic!("add");
    };
    assert_eq!(content, "text");
    assert_eq!(place.journal.as_deref(), Some("today"));
    assert!(Cli::try_parse_from(["tine", "add", "--journal"]).is_err());
    // A date without `=` is content, never a silently re-read date.
    assert!(Cli::try_parse_from(["tine", "add", "--journal", "2026-10-18", "text"]).is_err());
}

#[test]
fn external_url_argv_never_becomes_a_relative_graph_path() {
    let url = "tine://block/11111111-1111-4111-8111-111111111111";
    assert_eq!(
        launch_request(&argv(&["tine", url]), Path::new("/wrong")),
        LaunchRequest::Link(url.into())
    );
}

#[test]
fn export_requires_an_output_parent_and_accepts_both_formats() {
    for format in ["static", "live"] {
        assert!(Cli::try_parse_from(["tine", "export", format, "g"]).is_err());
        let parsed = Cli::try_parse_from([
            "tine",
            "export",
            format,
            "g",
            "--output",
            "/o",
            "--name",
            "N",
            "--all-pages",
        ])
        .unwrap();
        assert!(matches!(parsed.command, Some(Command::Export { .. })));
    }
    let live = Cli::try_parse_from([
        "tine",
        "export",
        "live",
        "g",
        "--output",
        "/o",
        "--home",
        "Directory",
    ])
    .unwrap();
    let Some(Command::Export {
        format: ExportFormat::Live(args),
    }) = live.command
    else {
        panic!("live export did not parse");
    };
    assert_eq!(args.home.as_deref(), Some("Directory"));
    assert!(Cli::try_parse_from([
        "tine",
        "export",
        "static",
        "g",
        "--output",
        "/o",
        "--home",
        "Directory",
    ])
    .is_err());
    let relative = export(ExportFormat::Static(ExportArgs {
        graph: PathBuf::from("g"),
        output: PathBuf::from("relative"),
        name: None,
        all_pages: false,
        no_comments: false,
    }));
    assert_eq!(
        relative.unwrap_err(),
        "--output must be an absolute folder path"
    );
}

#[test]
fn static_export_explains_empty_public_selection_and_all_pages_override() {
    let graph = doctor_fixture("private-export");
    let output = tempfile::tempdir().unwrap();
    let publish = |name: &str, all_pages| {
        export(ExportFormat::Static(ExportArgs {
            graph: graph.path().to_path_buf(),
            output: output.path().to_path_buf(),
            name: Some(name.into()),
            all_pages,
            no_comments: false,
        }))
        .unwrap()
    };
    let empty = publish("public-only", false);
    assert_eq!(empty.pages, 0);
    assert!(empty
        .warnings
        .iter()
        .any(|warning| warning.contains("public:: true") && warning.contains("--all-pages")));
    let all = publish("all-pages", true);
    assert_eq!(all.pages, 1);
    assert!(all.warnings.is_empty());
    assert_eq!(
        std::fs::read_to_string(graph.path().join("pages/Note.md")).unwrap(),
        "- fine\n"
    );
}

fn run_query_export(graph: &Path, uuid: &str, extra: &[&str]) -> Result<ExportReceipt, String> {
    let mut argv = vec!["tine", "export", "query", graph.to_str().unwrap(), uuid];
    argv.extend_from_slice(extra);
    let parsed = Cli::try_parse_from(argv).map_err(|e| e.to_string())?;
    let Some(Command::Export {
        format: ExportFormat::Query(args),
    }) = parsed.command
    else {
        panic!("query export");
    };
    // Default test builds do not embed the frontend. Supply only that build
    // artifact while exercising the real parser, selector and publisher.
    let request = query_export_args(&args)?;
    let store = open_store(&args.graph)?;
    crate::owner::query_export::publish(
        &store,
        request,
        &[(
            "index.html".into(),
            b"<!doctype html><html><head></head><body><div id=\"root\"></div></body></html>"
                .to_vec(),
        )],
    )
}

#[test]
fn query_export_by_uuid() {
    let graph = doctor_fixture("query-export");
    let uuid = "82400000-0000-4000-8000-000000000001";
    let raw = format!(
        "- {{{{query (task TODO)}}}}\n  id:: {uuid}\n  query-table:: true\n- TODO selected\n"
    );
    std::fs::write(graph.path().join("pages/Tasks.md"), &raw).unwrap();
    let receipt = run_query_export(
        graph.path(),
        &format!("(({uuid}))"),
        &["--name", "Task review"],
    )
    .unwrap();
    assert_eq!(receipt.pages, 1);
    assert!(receipt.path.ends_with("published-queries/task-review"));
    assert!(Path::new(&receipt.path).join("app/snapshot.json").is_file());
    let before = std::fs::read(Path::new(&receipt.path).join("index.html")).unwrap();
    let refused = run_query_export(graph.path(), uuid, &["--name", "Task review"]).unwrap_err();
    assert!(refused.contains("--force"), "{refused}");
    assert_eq!(
        std::fs::read(Path::new(&receipt.path).join("index.html")).unwrap(),
        before
    );
    let replaced =
        run_query_export(graph.path(), uuid, &["--name", "Task review", "--force"]).unwrap();
    assert!(Path::new(replaced.retired.as_ref().unwrap()).is_dir());
    let default = run_query_export(graph.path(), uuid, &[]).unwrap();
    assert!(!default.path.is_empty());
    assert_eq!(
        std::fs::read_to_string(graph.path().join("pages/Tasks.md")).unwrap(),
        raw
    );
}

#[test]
fn query_export_by_uuid_errors() {
    let graph = doctor_fixture("query-export-errors");
    let uuid = "82400000-0000-4000-8000-000000000002";
    let unknown = run_query_export(graph.path(), uuid, &[]).unwrap_err();
    assert!(unknown.contains("unknown block UUID"), "{unknown}");
    for (raw, message) in [
        ("ordinary text", "not a query"),
        ("`{{query (task TODO)}}`", "not a query"),
        (
            "{{query (task TODO)}} {{tine-query @block}}",
            "more than one query",
        ),
    ] {
        std::fs::write(
            graph.path().join("pages/Query.md"),
            format!("- {raw}\n  id:: {uuid}\n"),
        )
        .unwrap();
        let error = run_query_export(graph.path(), uuid, &[]).unwrap_err();
        assert!(error.contains(message), "{raw}: {error}");
    }
    assert!(!graph.path().join("published-queries").exists());
    let bad = run_query_export(graph.path(), "not-a-uuid", &[]).unwrap_err();
    assert!(bad.contains("UUID"), "{bad}");
}

/// og I1f (#35, port of master e7af4db9): doctor names an unreadable page
/// and a duplicate page identity, and exits 1, instead of printing OK.
fn doctor_fixture(tag: &str) -> tempfile::TempDir {
    let root = tempfile::Builder::new()
        .prefix(&format!("tine-doctor-{tag}-"))
        .tempdir()
        .unwrap();
    std::fs::create_dir_all(root.path().join("pages")).unwrap();
    std::fs::create_dir_all(root.path().join("journals")).unwrap();
    std::fs::write(root.path().join("pages/Note.md"), "- fine\n").unwrap();
    root
}

#[test]
fn doctor_passes_a_clean_graph() {
    let root = doctor_fixture("clean");
    assert_eq!(doctor(root.path()), Ok(()));
}

#[test]
fn doctor_reports_an_unreadable_page_and_fails() {
    let root = doctor_fixture("unreadable");
    std::fs::write(root.path().join("pages/Bad.md"), [0xff, 0xfe]).unwrap();
    let report = doctor_report(root.path()).unwrap();
    assert!(
        report
            .problems
            .iter()
            .any(|line| line.starts_with("parse failure: ") && line.contains("Bad.md")),
        "{:?}",
        report.problems
    );
    assert_eq!(
        doctor(root.path()).unwrap_err(),
        "graph checks found problems"
    );
}

#[test]
fn doctor_reports_a_duplicate_page_identity_and_fails() {
    let root = doctor_fixture("duplicate");
    std::fs::write(root.path().join("pages/A.md"), "title:: Same\n\n- a\n").unwrap();
    std::fs::write(root.path().join("pages/B.md"), "title:: same\n\n- b\n").unwrap();
    let report = doctor_report(root.path()).unwrap();
    assert!(
        report.problems.iter().any(
            |line| line.starts_with("duplicate page identity \"same\": ")
                && line.contains("A.md")
                && line.contains("B.md")
        ),
        "{:?}",
        report.problems
    );
    assert_eq!(
        doctor(root.path()).unwrap_err(),
        "graph checks found problems"
    );
}

#[test]
fn checked_in_man_pages_are_generated_from_the_cli_schema() {
    const MAN_PAGES: &[(&str, &str)] = &[
        ("tine.1", include_str!("../../docs/tine.1")),
        ("tine-open.1", include_str!("../../docs/tine-open.1")),
        ("tine-capture.1", include_str!("../../docs/tine-capture.1")),
        ("tine-export.1", include_str!("../../docs/tine-export.1")),
        (
            "tine-export-static.1",
            include_str!("../../docs/tine-export-static.1"),
        ),
        (
            "tine-export-live.1",
            include_str!("../../docs/tine-export-live.1"),
        ),
        (
            "tine-export-query.1",
            include_str!("../../docs/tine-export-query.1"),
        ),
        ("tine-doctor.1", include_str!("../../docs/tine-doctor.1")),
        ("tine-version.1", include_str!("../../docs/tine-version.1")),
        ("tine-serve.1", include_str!("../../docs/tine-serve.1")),
        ("tine-status.1", include_str!("../../docs/tine-status.1")),
        ("tine-page.1", include_str!("../../docs/tine-page.1")),
        (
            "tine-page-current.1",
            include_str!("../../docs/tine-page-current.1"),
        ),
        (
            "tine-page-create.1",
            include_str!("../../docs/tine-page-create.1"),
        ),
        (
            "tine-page-rename.1",
            include_str!("../../docs/tine-page-rename.1"),
        ),
        (
            "tine-page-delete.1",
            include_str!("../../docs/tine-page-delete.1"),
        ),
        ("tine-ref.1", include_str!("../../docs/tine-ref.1")),
        ("tine-embed.1", include_str!("../../docs/tine-embed.1")),
        (
            "tine-page-show.1",
            include_str!("../../docs/tine-page-show.1"),
        ),
        ("tine-pages.1", include_str!("../../docs/tine-pages.1")),
        ("tine-journal.1", include_str!("../../docs/tine-journal.1")),
        ("tine-block.1", include_str!("../../docs/tine-block.1")),
        (
            "tine-block-show.1",
            include_str!("../../docs/tine-block-show.1"),
        ),
        ("tine-refs.1", include_str!("../../docs/tine-refs.1")),
        ("tine-search.1", include_str!("../../docs/tine-search.1")),
        ("tine-query.1", include_str!("../../docs/tine-query.1")),
        ("tine-edit.1", include_str!("../../docs/tine-edit.1")),
        ("tine-delete.1", include_str!("../../docs/tine-delete.1")),
        ("tine-add.1", include_str!("../../docs/tine-add.1")),
        ("tine-move.1", include_str!("../../docs/tine-move.1")),
        ("tine-prop.1", include_str!("../../docs/tine-prop.1")),
        (
            "tine-prop-set.1",
            include_str!("../../docs/tine-prop-set.1"),
        ),
        (
            "tine-prop-remove.1",
            include_str!("../../docs/tine-prop-remove.1"),
        ),
        ("tine-run.1", include_str!("../../docs/tine-run.1")),
        (
            "tine-run-begin.1",
            include_str!("../../docs/tine-run-begin.1"),
        ),
        (
            "tine-run-show.1",
            include_str!("../../docs/tine-run-show.1"),
        ),
        ("tine-run-end.1", include_str!("../../docs/tine-run-end.1")),
        (
            "tine-run-revert.1",
            include_str!("../../docs/tine-run-revert.1"),
        ),
    ];
    if std::env::var_os("TINE_UPDATE_MAN_PAGE").is_some() {
        clap_mangen::generate_to(
            Cli::command(),
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs"),
        )
        .unwrap();
        return;
    }
    let generated = tempfile::tempdir().unwrap();
    clap_mangen::generate_to(Cli::command(), generated.path()).unwrap();
    for (name, expected) in MAN_PAGES {
        assert_eq!(
            std::fs::read_to_string(generated.path().join(name)).unwrap(),
            *expected,
            "{name} drifted from the CLI schema (TINE_UPDATE_MAN_PAGE=1 regenerates)"
        );
    }
    let packaged = include_str!("../tauri.conf.json");
    for (name, _) in MAN_PAGES {
        assert!(
            packaged
                .matches(&format!("usr/share/man/man1/{name}"))
                .count()
                == 2,
            "{name} must ship in both the deb and the rpm package"
        );
    }
}
