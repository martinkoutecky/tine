//! Desktop command line over og's Store. One typed clap schema owns parsing,
//! `--help`, `--version`, the generated man pages, the cold GUI launch and the
//! request forwarded to an already-running instance (master ADR 0068; before this
//! the hand-rolled parser, `graph::resolve_root` and the single-instance handler
//! each re-interpreted argv, and `tine open GRAPH` was forwarded as a page called
//! "open"). Whole-graph export creates a child in an external `--output` parent.
//! Query export uses the shared Store-owned query leaf publisher, with explicit
//! `--force` replacement preserving the previous output in recovery.

use clap::{ArgAction, Args, Parser, Subcommand};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use tine_graph_features::publish_query::{publish_live_home, publish_static, ExportReceipt};
use tine_store::Store;

mod prop;
use prop::PropAction;

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
    /// Run a graph's owner for the command line. Read commands start it
    /// when the app does not have the graph open; it exits after 30 idle
    /// minutes or when the app opens the graph.
    Serve {
        #[arg(value_name = "GRAPH", value_hint = clap::ValueHint::DirPath)]
        graph: PathBuf,
        /// Ask the graph's command-line owner to finish its work and exit.
        #[arg(long)]
        stop: bool,
    },
    /// Show which process owns the graph and what it is doing.
    Status(ReadArgs),
    /// Read, create, rename or delete pages.
    Page {
        #[command(subcommand)]
        action: PageAction,
    },
    /// Read blocks.
    Block {
        #[command(subcommand)]
        action: BlockAction,
    },
    /// Show a journal page: today, or the given date (YYYY-MM-DD).
    Journal {
        #[arg(value_name = "DATE")]
        date: Option<String>,
        #[command(flatten)]
        read: ReadArgs,
    },
    /// Show the linked references of a page, or of a block with --block.
    Refs {
        #[arg(value_name = "PAGE|UUID")]
        target: String,
        /// TARGET is a block's id:: UUID.
        #[arg(long)]
        block: bool,
        #[command(flatten)]
        read: ReadArgs,
    },
    /// Print ((uuid)), durably assigning the block an id when needed.
    Ref {
        #[arg(value_name = "BLOCK")]
        handle: String,
        #[command(flatten)]
        write: WriteArgs,
    },
    /// Print {{embed ((uuid))}}, ensuring a durable block id first.
    Embed {
        #[arg(value_name = "BLOCK")]
        handle: String,
        #[command(flatten)]
        write: WriteArgs,
    },
    /// Search page names and block text, as the app's search does.
    Search {
        #[arg(value_name = "TEXT")]
        text: String,
        /// At most this many page hits.
        #[arg(long, default_value_t = 20)]
        pages: usize,
        /// At most this many block hits.
        #[arg(long, default_value_t = 50)]
        blocks: usize,
        #[command(flatten)]
        read: ReadArgs,
    },
    /// Run a query: an OG simple or advanced query, or TQL with --tql.
    Query {
        #[arg(value_name = "QUERY")]
        text: String,
        /// QUERY is TQL, as in {{tine-query …}}.
        #[arg(long)]
        tql: bool,
        #[command(flatten)]
        read: ReadArgs,
    },
    /// List pages, or the pages at or under a namespace.
    Pages {
        #[arg(long, value_name = "NAMESPACE")]
        namespace: Option<String>,
        #[command(flatten)]
        read: ReadArgs,
    },
    /// Replace a block's text. Its properties (id::, status::, tine.author::) and
    /// children are kept; a property named in the new text replaces the kept one.
    /// Supplying a LOGBOOK drawer requires --rev from the read; otherwise
    /// reread and pass --rev (refused:revision-required).
    Edit {
        /// The block: its id:: UUID, or PAGE/POS~FP as a read prints it.
        #[arg(value_name = "BLOCK")]
        handle: String,
        /// The new text; `-` reads it from standard input.
        #[arg(value_name = "TEXT", allow_hyphen_values = true)]
        text: String,
        #[command(flatten)]
        write: WriteArgs,
    },
    /// Insert a page fragment, attributing new blocks unless --no-author is set.
    Add {
        /// Markdown or Org page fragment; `-` reads standard input.
        #[arg(value_name = "CONTENT", allow_hyphen_values = true)]
        content: String,
        #[command(flatten)]
        place: AddPlace,
        #[command(flatten)]
        write: WriteArgs,
    },
    #[command(about = crate::i18n_shell::shell_text("en", "shell-cli-delete-help"))]
    Delete {
        #[arg(value_name = "BLOCK")]
        handle: String,
        #[arg(long, visible_alias = "force", help = crate::i18n_shell::shell_text("en", "shell-cli-delete-confirm-help"))]
        yes: bool,
        #[command(flatten)]
        write: WriteArgs,
    },
    /// Move a block, with its children, under, after or before another block
    /// on the same page.
    Move {
        #[arg(value_name = "BLOCK")]
        handle: String,
        #[command(flatten)]
        place: MovePlace,
        #[command(flatten)]
        write: WriteArgs,
    },
    /// Set or remove one property of a block.
    Prop {
        #[command(subcommand)]
        action: PropAction,
    },
    /// Group writes into a git run: each write made with `--run ID` is
    /// committed (after a snapshot of your own pending edits), and the run can
    /// be squashed or reverted as one.
    Run {
        #[command(subcommand)]
        action: RunAction,
    },
}

#[derive(Debug, Subcommand)]
enum RunAction {
    /// Check the graph's git repository and print a new run id.
    Begin {
        #[command(flatten)]
        read: ReadArgs,
    },
    /// List the run's commits.
    Show {
        #[arg(value_name = "RUN", env = "TINE_RUN")]
        run: String,
        #[command(flatten)]
        read: ReadArgs,
    },
    /// Squash the run into one commit when none of it is pushed, branched
    /// or interleaved with other commits; otherwise leave it as it is.
    End {
        #[arg(value_name = "RUN", env = "TINE_RUN")]
        run: String,
        /// Subject of the squashed commit.
        #[arg(long, short = 'm', value_name = "MESSAGE")]
        message: Option<String>,
        #[command(flatten)]
        read: ReadArgs,
    },
    /// Undo the run's edits, newest first. A block you changed since keeps
    /// your text and is listed.
    Revert {
        #[arg(value_name = "RUN", env = "TINE_RUN")]
        run: String,
        #[command(flatten)]
        authors: AuthorArgs,
        #[command(flatten)]
        read: ReadArgs,
    },
}

/// Where `tine move` puts the block: exactly one of the three.
#[derive(Debug, Args)]
#[group(required = true, multiple = false)]
struct MovePlace {
    /// As the last child of TARGET.
    #[arg(long, value_name = "TARGET")]
    under: Option<String>,
    /// Right after TARGET, as its next sibling.
    #[arg(long, value_name = "TARGET")]
    after: Option<String>,
    /// Right before TARGET, as its previous sibling.
    #[arg(long, value_name = "TARGET")]
    before: Option<String>,
}

#[derive(Debug, Args)]
#[group(required = true, multiple = false)]
struct AddPlace {
    /// Append as the last child of BLOCK.
    #[arg(long, value_name = "BLOCK")]
    under: Option<String>,
    /// Insert as the next sibling of BLOCK.
    #[arg(long, value_name = "BLOCK")]
    after: Option<String>,
    /// Insert as the previous sibling of BLOCK.
    #[arg(long, value_name = "BLOCK")]
    before: Option<String>,
    /// Append to PAGE, creating it when absent.
    #[arg(long, value_name = "PAGE")]
    page: Option<String>,
    /// Append to a journal, creating it when absent: `--journal` is today,
    /// `--journal=DATE` another day.
    #[arg(long, value_name = "DATE", num_args = 0..=1, require_equals = true, default_missing_value = "today")]
    journal: Option<String>,
}

/// Which graph a write asks, its optional page guard, and how it prints.
#[derive(Debug, Args)]
struct WriteArgs {
    #[command(flatten)]
    read: ReadArgs,
    /// Write only if the page is still at this rev (as `page show` prints it);
    /// required for a bare PAGE/POS without a fingerprint.
    #[arg(long, value_name = "REV")]
    rev: Option<String>,
    /// Commit this write to the graph's git repository as part of RUN (from
    /// `tine run begin`).
    #[arg(long, value_name = "RUN", env = "TINE_RUN")]
    run: Option<String>,
    #[command(flatten)]
    authors: AuthorArgs,
}

/// Shared block attribution and git identity options for CLI writes.
#[derive(Debug, Args)]
struct AuthorArgs {
    /// Author on created blocks and on the run's commits (default: TINE_AUTHOR,
    /// else agent; blank environment values count as unset). With
    /// `edit`, the flag (not the variable) also sets the block's tine.author::.
    #[arg(long)]
    author: Option<String>,
    /// Skip automatic block attribution for personal scripts. Existing properties
    /// and git run commit names stay unchanged.
    #[arg(long, conflicts_with = "author")]
    no_author: bool,
}

impl AuthorArgs {
    fn args(&self, mut fields: serde_json::Value) -> serde_json::Value {
        fields["author"] = serde_json::json!(self.author);
        fields["no_author"] = serde_json::json!(self.no_author);
        fields
    }
}

impl WriteArgs {
    /// The verb's arguments plus `rev`, `run` and `author`.
    fn args(&self, mut fields: serde_json::Value) -> serde_json::Value {
        if let Some(object) = fields.as_object_mut() {
            object.insert("rev".into(), serde_json::json!(self.rev));
            object.insert("run".into(), serde_json::json!(self.run));
        }
        self.authors.args(fields)
    }
}

/// Rename is graph-wide; the single-page revision/run options cannot guard it.
#[derive(Debug, Args)]
struct RenameOptions {
    #[command(flatten)]
    read: ReadArgs,
    /// Unsupported for graph-wide rename; supplying this option refuses without changes.
    #[arg(long, value_name = "REV")]
    rev: Option<String>,
    /// Unsupported for graph-wide rename; supplying this option (or a nonempty
    /// TINE_RUN) refuses without changes. End/unset the run before renaming.
    #[arg(long, value_name = "RUN", env = "TINE_RUN")]
    run: Option<String>,
}

#[derive(Debug, Subcommand)]
enum PageAction {
    /// Show the focused page name and Tine links without changing notes.
    Current {
        #[command(flatten)]
        read: ReadArgs,
    },
    /// Create an empty page; refuse an existing page or alias.
    Create {
        name: String,
        #[command(flatten)]
        write: WriteArgs,
    },
    /// Rename a page and rewrite references, as the app does. An existing
    /// destination refuses; merges require confirmation in the app.
    Rename {
        old: String,
        new: String,
        #[command(flatten)]
        write: RenameOptions,
    },
    /// Delete a page to graph trash and report broken references.
    Delete {
        name: String,
        /// Confirm deletion without an interactive prompt.
        #[arg(long, visible_alias = "force")]
        yes: bool,
        #[command(flatten)]
        write: WriteArgs,
    },
    /// Show a page's outline (the text a window shows, unsaved edits included).
    Show {
        #[arg(value_name = "PAGE")]
        name: String,
        #[command(flatten)]
        read: ReadArgs,
    },
}

#[derive(Debug, Subcommand)]
enum BlockAction {
    /// Show a block and its children by its id:: UUID.
    Show {
        #[arg(value_name = "UUID")]
        uuid: String,
        #[command(flatten)]
        read: ReadArgs,
    },
}

/// Which graph a read command asks, and how it prints.
#[derive(Debug, Args)]
struct ReadArgs {
    /// The graph. Defaults to $TINE_GRAPH, else the graph folder around the
    /// current directory.
    #[arg(long, short = 'g', value_name = "GRAPH", value_hint = clap::ValueHint::DirPath)]
    graph: Option<PathBuf>,
    /// Print the typed result (schema tine.cli/1) as JSON.
    #[arg(long)]
    json: bool,
}

impl ReadArgs {
    fn target(self) -> crate::owner::terminal::Target {
        crate::owner::terminal::Target {
            graph: self.graph,
            json: self.json,
        }
    }
}

#[derive(Debug, Subcommand)]
enum ExportFormat {
    /// Write the static HTML site.
    /// A zero-page public selection explains public:: true and --all-pages.
    Static(ExportArgs),
    /// Write the read-only Tine app.
    Live(LiveExportArgs),
    /// Publish result pages of a query into published-queries/.
    /// Sheets keep plain row outlines, as in other command-line exports.
    Query(QueryExportArgs),
}

#[derive(Debug, Args)]
#[command(group(clap::ArgGroup::new("source").required(true).args(["uuid", "query", "page"])))]
#[command(
    override_usage = "tine export query [OPTIONS] <GRAPH> <((UUID))|--query <TEXT>|--page <PAGE>>"
)]
struct QueryExportArgs {
    #[arg(value_name = "GRAPH", value_hint = clap::ValueHint::DirPath)]
    graph: PathBuf,

    /// Persisted block UUID, bare or wrapped as ((UUID)); quote parentheses in the shell.
    #[arg(value_name = "((UUID))")]
    uuid: Option<String>,

    /// Inline argument of a query macro: Logseq simple or advanced, as decided by the shared parser.
    #[arg(long, value_name = "TEXT", requires = "name")]
    query: Option<String>,

    /// First query on this page, in document order (parents before children; embeds are not followed).
    #[arg(long, value_name = "PAGE")]
    page: Option<String>,

    /// Output name; required for --query, otherwise the title or Query UUID/PAGE.
    #[arg(long, value_name = "NAME")]
    name: Option<String>,

    /// Replace an existing output, keeping the previous export in recovery.
    #[arg(long)]
    force: bool,
    /// Leave comment threads out of this export.
    #[arg(long)]
    no_comments: bool,
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
    /// Leave comment threads out of this export.
    #[arg(long)]
    no_comments: bool,
}

/// What a launch asks the GUI to do; the same value comes from the cold-start
/// argv and from a second instance's forwarded argv.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LaunchRequest {
    Focus,
    Open(PathBuf),
    Capture,
    Link(String),
}

fn terminal_stdout(arguments: std::fmt::Arguments<'_>) {
    let _ = writeln!(std::io::stdout().lock(), "{arguments}");
}

fn terminal_stderr(arguments: std::fmt::Arguments<'_>) {
    let _ = writeln!(std::io::stderr().lock(), "{arguments}");
}

#[cfg(test)]
pub(crate) fn bundle() -> Vec<(String, Vec<u8>)> {
    vec![(
        "index.html".into(),
        b"<!doctype html><html><head></head><body><div id=\"root\"></div></body></html>".to_vec(),
    )]
}

#[cfg(not(test))]
pub(crate) fn bundle() -> Vec<(String, Vec<u8>)> {
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
        ExportFormat::Query(args) => return export_query(args),
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
    let public_only_static = live.is_none() && !args.all_pages;
    let result = match live {
        Some(home) => publish_live_home(
            &store,
            &args.output,
            &display,
            args.all_pages,
            !args.no_comments,
            home.as_deref(),
            &bundle(),
        ),
        None => publish_static(
            &store,
            &args.output,
            &display,
            args.all_pages,
            !args.no_comments,
        ),
    };
    result
        .map(|mut receipt| {
            if public_only_static && receipt.pages == 0 {
                receipt.warnings.push(
                    "No pages were published: no publishable page has public:: true. Add public:: true to the pages you want to publish, or use --all-pages to include unmarked pages (public:: false pages stay private).".into(),
                );
            }
            receipt
        })
        .map_err(|e| e.to_string())
}

fn export_query(args: QueryExportArgs) -> Result<ExportReceipt, String> {
    let request = query_export_args(&args)?;
    let root = args
        .graph
        .canonicalize()
        .map_err(|e| format!("cannot open {}: {e}", args.graph.display()))?;
    let mut conn = crate::owner::client::connect(&root).map_err(|e| e.to_string())?;
    let reply = conn
        .call(
            "query.export",
            serde_json::to_value(request).map_err(|e| e.to_string())?,
        )
        .map_err(|error| match error {
            crate::owner::client::Failure::Owner(body) => body.message,
            crate::owner::client::Failure::Reach(error) => error.to_string(),
            crate::owner::client::Failure::Lost(message) => message,
        })?;
    serde_json::from_value(reply).map_err(|e| e.to_string())
}

fn query_export_args(args: &QueryExportArgs) -> Result<crate::owner::query_export::Args, String> {
    use crate::owner::query_export::Source;
    let source = if let Some(raw) = &args.uuid {
        let raw = raw.trim();
        let uuid = raw
            .strip_prefix("((")
            .and_then(|value| value.strip_suffix("))"))
            .unwrap_or(raw);
        let uuid = uuid::Uuid::parse_str(uuid)
            .map_err(|_| "expected a block UUID, bare or wrapped as ((UUID))".to_owned())?
            .to_string();
        Source::Uuid { uuid }
    } else if let Some(query) = &args.query {
        Source::Inline {
            query: query.clone(),
        }
    } else {
        Source::Page {
            page: args.page.clone().expect("clap requires a source"),
        }
    };
    Ok(crate::owner::query_export::Args {
        source,
        name: args.name.clone(),
        force: args.force,
        no_comments: args.no_comments,
        epoch: tine_graph_features::publish_query::export_epoch().map_err(|e| e.to_string())?,
    })
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
    let corpus = graph.corpus(None, false);
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

/// Whether `tine edit` was given `--author` itself. An agent's session-wide
/// `TINE_AUTHOR` names its commits; it must not stamp every block it edits, or
/// overwrite the `tine.author::` a person's block already has (#840).
fn edit_author_is_a_flag<I, T>(argv: I) -> bool
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    use clap::CommandFactory;
    Cli::command()
        .try_get_matches_from(argv)
        .is_ok_and(|parsed| {
            parsed.subcommand_matches("edit").is_some_and(|edit| {
                edit.value_source("author") == Some(clap::parser::ValueSource::CommandLine)
            })
        })
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
        Some(Command::Serve { graph, stop }) => {
            return Some(crate::owner::terminal::serve(&graph, stop))
        }
        Some(
            command @ (Command::Status(_)
            | Command::Page { .. }
            | Command::Block { .. }
            | Command::Journal { .. }
            | Command::Refs { .. }
            | Command::Ref { .. }
            | Command::Embed { .. }
            | Command::Search { .. }
            | Command::Query { .. }
            | Command::Pages { .. }
            | Command::Edit { .. }
            | Command::Prop { .. }
            | Command::Move { .. }
            | Command::Delete { .. }
            | Command::Add { .. }
            | Command::Run { .. }),
        ) => return Some(read_command(command)),
        Some(Command::Export { format }) => export(format).map(|receipt| {
            terminal_stdout(format_args!(
                "Published {} pages to {}",
                receipt.pages, receipt.path
            ));
            for warning in &receipt.warnings {
                terminal_stderr(format_args!("tine: warning: {warning}"));
            }
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

/// A read command: one request to the graph's owner (CLI PROTOCOL §5).
fn read_command(command: Command) -> i32 {
    use crate::owner::terminal::{read_verb, write_verb};
    use serde_json::json;
    match command {
        Command::Status(args) => read_verb(args.target(), "owner.status", json!(null)),
        Command::Page {
            action: PageAction::Current { read },
        } => read_verb(read.target(), "page.current", json!(null)),
        Command::Page {
            action: PageAction::Show { name, read: args },
        } => read_verb(args.target(), "page.show", json!({ "name": name })),
        Command::Block {
            action: BlockAction::Show { uuid, read: args },
        } => read_verb(args.target(), "block.show", json!({ "uuid": uuid })),
        Command::Page {
            action: PageAction::Create { name, write },
        } => {
            let args = write.args(json!({"name":name}));
            write_verb(write.read.target(), "page.create", args)
        }
        Command::Page {
            action: PageAction::Rename { old, new, write },
        } => {
            let args = json!({"old":old,"new":new,"rev":write.rev,"run":write.run});
            write_verb(write.read.target(), "page.rename", args)
        }
        Command::Page {
            action: PageAction::Delete { name, yes, write },
        } => {
            let args = write.args(json!({"name":name,"yes":yes}));
            write_verb(write.read.target(), "page.delete", args)
        }
        Command::Ref { handle, write } => {
            let args = write.args(json!({"handle":handle}));
            write_verb(write.read.target(), "ref", args)
        }
        Command::Embed { handle, write } => {
            let args = write.args(json!({"handle":handle}));
            write_verb(write.read.target(), "embed", args)
        }
        Command::Journal { date, read: args } => {
            read_verb(args.target(), "journal", json!({ "date": date }))
        }
        Command::Refs {
            target,
            block,
            read: args,
        } => read_verb(
            args.target(),
            "refs",
            json!({ "target": target, "block": block }),
        ),
        Command::Search {
            text,
            pages,
            blocks,
            read: args,
        } => read_verb(
            args.target(),
            "search",
            json!({ "text": text, "pages": pages, "blocks": blocks }),
        ),
        Command::Query {
            text,
            tql,
            read: args,
        } => read_verb(args.target(), "query", json!({ "text": text, "tql": tql })),
        Command::Pages {
            namespace,
            read: args,
        } => read_verb(args.target(), "pages", json!({ "namespace": namespace })),
        Command::Edit {
            handle,
            text,
            write,
        } => {
            let text = if text == "-" {
                let mut buffer = String::new();
                if let Err(error) =
                    std::io::Read::read_to_string(&mut std::io::stdin(), &mut buffer)
                {
                    eprintln!("tine: could not read the text from standard input: {error}");
                    return 2;
                }
                buffer.trim_end_matches(['\n', '\r']).to_string()
            } else {
                text
            };
            let mut args = write.args(json!({ "handle": handle, "text": text }));
            if edit_author_is_a_flag(std::env::args_os()) {
                args["author_property"] = json!(write.authors.author);
            }
            write_verb(write.read.target(), "block.edit", args)
        }
        Command::Add {
            content,
            place,
            write,
        } => {
            let content = if content == "-" {
                let mut text = String::new();
                if let Err(error) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)
                {
                    eprintln!("tine: could not read content from standard input: {error}");
                    return 2;
                }
                text
            } else {
                content
            };
            let args = write.args(json!({
                "content": content, "under": place.under, "after": place.after,
                "before": place.before, "page": place.page, "journal": place.journal,
            }));
            write_verb(write.read.target(), "block.add", args)
        }
        Command::Delete { handle, yes, write } => {
            let args = write.args(json!({ "handle": handle, "yes": yes }));
            write_verb(write.read.target(), "block.delete", args)
        }
        Command::Move {
            handle,
            place,
            write,
        } => {
            let (at, to) = match (place.under, place.after, place.before) {
                (Some(to), _, _) => ("under", to),
                (_, Some(to), _) => ("after", to),
                (_, _, Some(to)) => ("before", to),
                _ => return 2,
            };
            let args = write.args(json!({ "handle": handle, "to": to, "at": at }));
            write_verb(write.read.target(), "block.move", args)
        }
        Command::Prop { action } => action.dispatch_property(),
        Command::Run { action } => match action {
            RunAction::Begin { read } => write_verb(read.target(), "run.begin", json!({})),
            RunAction::Show { run, read } => {
                read_verb(read.target(), "run.show", json!({ "run": run }))
            }
            RunAction::End { run, message, read } => write_verb(
                read.target(),
                "run.end",
                json!({ "run": run, "message": message }),
            ),
            RunAction::Revert { run, authors, read } => write_verb(
                read.target(),
                "run.revert",
                authors.args(json!({ "run": run })),
            ),
        },
        _ => 2,
    }
}

/// The GUI request in `argv`, with a relative graph resolved against `cwd`
/// (the sender's, for a forwarded launch). Unparseable argv only focuses.
pub(crate) fn launch_request(argv: &[String], cwd: &Path) -> LaunchRequest {
    if let Some(url) = argv.iter().skip(1).find(|arg| arg.starts_with("tine:")) {
        return LaunchRequest::Link(url.clone());
    }
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
            "help"
                | "version"
                | "export"
                | "doctor"
                | "serve"
                | "status"
                | "page"
                | "block"
                | "journal"
                | "refs"
                | "ref"
                | "embed"
                | "search"
                | "query"
                | "pages"
                | "add"
                | "edit"
                | "prop"
                | "move"
                | "delete"
                | "run"
                | "-h"
                | "--help"
                | "-v"
                | "-V"
                | "--version"
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
#[path = "cli_tests.rs"]
mod tests;
