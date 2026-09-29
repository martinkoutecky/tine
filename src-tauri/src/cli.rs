//! Desktop export commands over og's Store. The output argument names an
//! existing external parent; both formats install one create-only child there.
//! No direct projection, graph-relative publication, or replacement path exists.

use std::path::{Path, PathBuf};
use tine_graph_features::publish_query::{publish_live, publish_static, ExportReceipt};
use tine_store::Store;

const HELP: &str = "Usage: tine [GRAPH | --capture | open GRAPH | capture]\n\
  tine export (static|live) GRAPH --output PARENT [--name NAME] [--all-pages]\n\
  tine doctor GRAPH\n\
  tine --help | --version\n\n\
PARENT must be an existing folder outside the graph. Export creates a new\n\
named child and refuses to replace an existing output.";

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

fn export(args: &[String]) -> Result<ExportReceipt, String> {
    let format = args.first().ok_or("choose static or live")?.as_str();
    if !matches!(format, "static" | "live") {
        return Err("choose static or live".into());
    }
    let graph = args.get(1).ok_or("supply GRAPH")?;
    let mut output = None;
    let mut name = None;
    let mut all_pages = false;
    let mut index = 2;
    while index < args.len() {
        match args[index].as_str() {
            "--output" => {
                index += 1;
                output = Some(args.get(index).ok_or("--output needs a folder")?.clone());
            }
            "--name" => {
                index += 1;
                name = Some(args.get(index).ok_or("--name needs a value")?.clone());
            }
            "--all-pages" => all_pages = true,
            option => return Err(format!("unknown export option {option}")),
        }
        index += 1;
    }
    let parent = PathBuf::from(output.ok_or("--output PARENT is required")?);
    if !parent.is_absolute() {
        return Err("--output must be an absolute folder path".into());
    }
    let store = open_store(Path::new(graph))?;
    let display = name.unwrap_or_else(|| {
        Path::new(graph)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Tine export".into())
    });
    let result = if format == "live" {
        publish_live(&store, &parent, &display, all_pages, &bundle())
    } else {
        publish_static(&store, &parent, &display, all_pages)
    };
    result.map_err(|e| e.to_string())
}

fn doctor(args: &[String]) -> Result<(), String> {
    if args.len() != 1 {
        return Err("doctor needs one GRAPH path".into());
    }
    let store = open_store(Path::new(&args[0]))?;
    let graph = store
        .whole_graph()
        .map_err(|e| format!("cannot parse graph: {e:?}"))?;
    println!("Graph: {}", Path::new(&args[0]).display());
    println!("Pages and journals: {}", graph.corpus().pages.len());
    println!("OK: graph files are readable and parseable");
    Ok(())
}

/// Handle non-GUI desktop commands before Tauri starts. `None` continues into
/// the ordinary GUI; `Some(code)` exits after printing the result or error.
pub(crate) fn dispatch() -> Option<i32> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        return None;
    };
    let result = match command {
        "--help" | "-h" | "help" => {
            println!("{HELP}");
            return Some(0);
        }
        "--version" | "-v" | "-V" | "version" => {
            println!("tine {}", env!("CARGO_PKG_VERSION"));
            return Some(0);
        }
        "export" => export(&args[1..]).map(|receipt| {
            println!("Published {} pages to {}", receipt.pages, receipt.path);
        }),
        "doctor" => doctor(&args[1..]),
        _ => return None,
    };
    match result {
        Ok(()) => Some(0),
        Err(error) => {
            eprintln!("tine: {error}");
            Some(1)
        }
    }
}
