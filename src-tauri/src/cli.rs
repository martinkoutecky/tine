//! Desktop command line over og's Store. One typed clap schema owns parsing,
//! `--help`, `--version`, the generated man pages, the cold GUI launch and the
//! request forwarded to an already-running instance (master ADR 0068; before this
//! the hand-rolled parser, `graph::resolve_root` and the single-instance handler
//! each re-interpreted argv, and `tine open GRAPH` was forwarded as a page called
//! "open"). Export keeps og's create-only shape: `--output` names an existing
//! external parent and both formats install one new child there; no direct
//! projection, graph-relative publication or replacement path exists.

use clap::{ArgAction, Args, Parser, Subcommand};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use tine_graph_features::publish_query::{publish_live_home, publish_static, ExportReceipt};
use tine_store::Store;

#[derive(Debug, Parser)]
#[command(
    name = "tine",
    about = "A fast, local-first outliner for Logseq-compatible graphs",
    version = env!("CARGO_PKG_VERSION"),
    disable_version_flag = true,
    subcommand_precedence_over_arg = true
)]
struct Cli {
    /// Print the Tine version and exit.
    #[arg(short = 'v', visible_short_alias = 'V', long = "version", action = ArgAction::Version)]
    show_version: Option<bool>,

    /// Enable diagnostic logging for the desktop app.
    #[arg(long, global = true)]
    debug: bool,

    /// Open Quick Capture (legacy spelling; prefer `tine capture`).
    #[arg(long, conflicts_with = "graph")]
    capture: bool,

    /// Graph to open in the desktop app (legacy shorthand for `tine open GRAPH`).
    #[arg(value_name = "GRAPH", value_hint = clap::ValueHint::DirPath)]
    graph: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Open a graph in the desktop app.
    Open {
        #[arg(value_name = "GRAPH", value_hint = clap::ValueHint::DirPath)]
        graph: PathBuf,
    },
    /// Open Quick Capture in the desktop app.
    Capture,
    /// Publish a graph as a static site or a read-only Tine app.
    Export {
        #[command(subcommand)]
        format: ExportFormat,
    },
    /// Check that a graph can be read and parsed without changing it.
    Doctor {
        #[arg(value_name = "GRAPH", value_hint = clap::ValueHint::DirPath)]
        graph: PathBuf,
    },
    /// Print the Tine version and exit.
    Version,
}

#[derive(Debug, Subcommand)]
enum ExportFormat {
    /// Write the static HTML site.
    Static(ExportArgs),
    /// Write the read-only Tine app.
    Live(LiveExportArgs),
}

#[derive(Debug, Args)]
struct LiveExportArgs {
    #[command(flatten)]
    export: ExportArgs,

    /// Page to open first. Defaults to the configured home page when it is exported, Welcome to Tine, or the first page.
    #[arg(long, value_name = "PAGE")]
    home: Option<String>,
}

#[derive(Debug, Args)]
struct ExportArgs {
    #[arg(value_name = "GRAPH", value_hint = clap::ValueHint::DirPath)]
    graph: PathBuf,

    /// Existing absolute folder outside the graph; a new child is created in it.
    #[arg(long, required = true, value_name = "PARENT", value_hint = clap::ValueHint::DirPath)]
    output: PathBuf,

    /// Name of the created child and of the published site. Defaults to the graph folder name.
    #[arg(long, value_name = "NAME")]
    name: Option<String>,

    /// Publish every page, ignoring `public:: true` selection for this run.
    #[arg(long)]
    all_pages: bool,
}

/// What a launch asks the GUI to do; the same value comes from the cold-start
/// argv and from a second instance's forwarded argv.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LaunchRequest {
    Focus,
    Open(PathBuf),
    Capture,
}

fn terminal_stdout(arguments: std::fmt::Arguments<'_>) {
    let _ = writeln!(std::io::stdout().lock(), "{arguments}");
}

fn terminal_stderr(arguments: std::fmt::Arguments<'_>) {
    let _ = writeln!(std::io::stderr().lock(), "{arguments}");
}

fn bundle() -> Vec<(String, Vec<u8>)> {
    let context: tauri::Context<tauri::Wry> = tauri::generate_context!();
    let assets = context.assets();
    let mut files: Vec<_> = assets
        .iter()
        .map(|(path, _)| path.into_owned())
        .filter(|path| {
            let name = path.trim_start_matches('/');
            name == "index.html" || (name.starts_with("assets/") && !name.contains(".."))
        })
        .filter_map(|path| {
            let key = tauri::utils::assets::AssetKey::from(path.as_str());
            assets
                .get(&key)
                .map(|bytes| (path.trim_start_matches('/').to_owned(), bytes.into_owned()))
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

fn open_store(path: &Path) -> Result<Store, String> {
    let root = path
        .canonicalize()
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    Store::open(&root, Default::default())
        .map(|(store, _, _)| store)
        .map_err(|e| format!("cannot read graph {}: {e:?}", root.display()))
}

fn export(format: ExportFormat) -> Result<ExportReceipt, String> {
    let (live, args) = match format {
        ExportFormat::Static(args) => (None, args),
        ExportFormat::Live(args) => (Some(args.home), args.export),
    };
    if !args.output.is_absolute() {
        return Err("--output must be an absolute folder path".into());
    }
    let store = open_store(&args.graph)?;
    let display = args.name.unwrap_or_else(|| {
        args.graph
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Tine export".into())
    });
    let result = match live {
        Some(home) => publish_live_home(
            &store,
            &args.output,
            &display,
            args.all_pages,
            home.as_deref(),
            &bundle(),
        ),
        None => publish_static(&store, &args.output, &display, args.all_pages),
    };
    result.map_err(|e| e.to_string())
}

/// What `tine doctor` found: summary lines for stdout and one line per problem.
#[derive(Debug)]
struct DoctorReport {
    summary: Vec<String>,
    problems: Vec<String>,
}

fn doctor_report(path: &Path) -> Result<DoctorReport, String> {
    let store = open_store(path)?;
    let graph = store
        .whole_graph()
        .map_err(|e| format!("cannot parse graph: {e:?}"))?;
    let corpus = graph.corpus();
    // One identity per page name under the store's own fold (`page_key`): two
    // files claiming it make links and edits land on only one of them.
    let mut identities: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for page in &corpus.pages {
        identities
            .entry(tine_core::refs::page_key(&page.name))
            .or_default()
            .push(String::from(page.id.clone()));
    }
    let mut problems: Vec<String> = graph
        .unreadable_files()
        .iter()
        .map(|(file, reason)| format!("parse failure: {}: {reason}", file.as_str()))
        .collect();
    problems.extend(
        identities
            .into_iter()
            .filter(|(_, paths)| paths.len() > 1)
            .map(|(name, paths)| format!("duplicate page identity {name:?}: {}", paths.join(", "))),
    );
    let config = store.config();
    let summary = vec![
        format!("Graph: {}", path.display()),
        format!("Pages and journals: {}", corpus.pages.len()),
        format!(
            "Default home: {}",
            config.config.default_home.as_deref().unwrap_or("(none)")
        ),
    ];
    Ok(DoctorReport { summary, problems })
}

fn doctor(path: &Path) -> Result<(), String> {
    let report = doctor_report(path)?;
    for line in &report.summary {
        terminal_stdout(format_args!("{line}"));
    }
    if report.problems.is_empty() {
        terminal_stdout(format_args!(
            "OK: graph files are readable and parseable; page identities are unique"
        ));
        return Ok(());
    }
    for line in &report.problems {
        terminal_stderr(format_args!("{line}"));
    }
    Err("graph checks found problems".into())
}

/// Handle non-GUI desktop commands before Tauri starts. `None` continues into
/// the ordinary GUI; `Some(code)` exits after printing the result or error.
pub(crate) fn dispatch() -> Option<i32> {
    let argv = std::env::args_os().collect::<Vec<_>>();
    prepare_console_if_needed(&argv);
    let cli = match Cli::try_parse_from(&argv) {
        Ok(cli) => cli,
        Err(error) => {
            let code = if error.use_stderr() { 2 } else { 0 };
            let _ = error.print();
            return Some(code);
        }
    };
    let result = match cli.command {
        Some(Command::Version) => {
            terminal_stdout(format_args!("tine {}", env!("CARGO_PKG_VERSION")));
            return Some(0);
        }
        Some(Command::Doctor { graph }) => doctor(&graph),
        Some(Command::Export { format }) => export(format).map(|receipt| {
            terminal_stdout(format_args!(
                "Published {} pages to {}",
                receipt.pages, receipt.path
            ));
        }),
        _ => return None,
    };
    match result {
        Ok(()) => Some(0),
        Err(error) => {
            terminal_stderr(format_args!("tine: {error}"));
            Some(1)
        }
    }
}

/// The GUI request in `argv`, with a relative graph resolved against `cwd`
/// (the sender's, for a forwarded launch). Unparseable argv only focuses.
pub(crate) fn launch_request(argv: &[String], cwd: &Path) -> LaunchRequest {
    let Ok(cli) = Cli::try_parse_from(argv) else {
        return LaunchRequest::Focus;
    };
    let request = match cli.command {
        Some(Command::Open { graph }) => LaunchRequest::Open(graph),
        Some(Command::Capture) => LaunchRequest::Capture,
        Some(_) => LaunchRequest::Focus,
        None if cli.capture => LaunchRequest::Capture,
        None => cli
            .graph
            .map(LaunchRequest::Open)
            .unwrap_or(LaunchRequest::Focus),
    };
    match request {
        LaunchRequest::Open(path) if path.is_relative() => LaunchRequest::Open(cwd.join(path)),
        request => request,
    }
}

pub(crate) fn launch_request_env() -> LaunchRequest {
    let argv = std::env::args().collect::<Vec<_>>();
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    launch_request(&argv, &cwd)
}

/// A GUI-subsystem Windows binary has no console; attach the parent's so
/// `--help`, `doctor` and `export` can print. GUI launches never attach.
#[cfg(target_os = "windows")]
fn prepare_console_if_needed(argv: &[OsString]) {
    let headless = argv.iter().skip(1).any(|argument| {
        let argument = argument.to_string_lossy();
        let gui_flag = matches!(argument.as_ref(), "--debug" | "--capture");
        matches!(
            argument.as_ref(),
            "help" | "version" | "export" | "doctor" | "-h" | "--help" | "-v" | "-V" | "--version"
        ) || (argument.starts_with('-') && !gui_flag)
    });
    if headless {
        // SAFETY: AttachConsole takes no pointer; failure (no parent console) is ignored.
        unsafe {
            windows_sys::Win32::System::Console::AttachConsole(
                windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS,
            );
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn prepare_console_if_needed(_: &[OsString]) {}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| (*part).to_owned()).collect()
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
            "tine", "export", "live", "g", "--output", "/o", "--home", "Directory",
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
            "tine", "export", "static", "g", "--output", "/o", "--home", "Directory",
        ])
        .is_err());
        let relative = export(ExportFormat::Static(ExportArgs {
            graph: PathBuf::from("g"),
            output: PathBuf::from("relative"),
            name: None,
            all_pages: false,
        }));
        assert_eq!(
            relative.unwrap_err(),
            "--output must be an absolute folder path"
        );
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
            report.problems.iter().any(|line| line.starts_with("parse failure: ")
                && line.contains("Bad.md")),
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
            report.problems.iter().any(|line| line
                .starts_with("duplicate page identity \"same\": ")
                && line.contains("A.md")
                && line.contains("B.md")),
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
            ("tine-doctor.1", include_str!("../../docs/tine-doctor.1")),
            ("tine-version.1", include_str!("../../docs/tine-version.1")),
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
}
