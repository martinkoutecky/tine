//! Bundled Guide creation and copying. A demo graph writes O(seed bytes);
//! copying performs one transaction per page and asset, O(copied bytes plus
//! each logical page lookup). Existing files are left untouched. An unknown
//! title and graph I/O errors are returned as I/O errors. Callers need no
//! graph path, name encoding, or transaction details.

use std::collections::HashSet;
use std::io;

use tine_core::guide::{
    collect_guide_asset_refs, guide_copy_page_name, guide_link_renames,
    rewrite_bundled_guide_links, CONFIG_EDN, GUIDE_ASSETS, GUIDE_TEMPLATES, QUICK_CAPTURE_PNG,
};
use tine_store::{Area, Content, OpenError, Resolved, Store, TxOutcome, Why};

use crate::{store_error, tx_error};

#[derive(Debug, Clone, serde::Serialize)]
pub struct GuideCopyResult {
    pub name: String,
    pub created: bool,
    pub created_pages: Vec<String>,
    pub skipped_pages: Vec<String>,
    pub copied_assets: Vec<String>,
}

/// Seed and scaffold the onboarding graph; returns its chosen root.
pub fn create_demo_graph(parent: &std::path::Path) -> Result<std::path::PathBuf, OpenError> {
    let config = tine_core::config::Config::parse(CONFIG_EDN);
    let mut seed = vec![
        (
            Area::Meta,
            "config.edn".to_string(),
            CONFIG_EDN.as_bytes().to_vec(),
        ),
        (
            Area::Assets,
            "quick-capture.png".to_string(),
            QUICK_CAPTURE_PNG.to_vec(),
        ),
    ];
    for template in GUIDE_TEMPLATES {
        seed.push((
            Area::Pages,
            format!(
                "{}.md",
                tine_core::model::encode_page_name(template.title, config.file_name_format)
            ),
            template.markdown.as_bytes().to_vec(),
        ));
    }
    Store::create_graph(parent, &seed)
}

fn create_if_absent(store: &Store, area: Area, rel: &str, bytes: &[u8]) -> io::Result<bool> {
    let id = store.file_id(area, rel).map_err(store_error)?;
    let mut tx = store.transaction();
    tx.create(&id, Content::Bytes(bytes.to_vec()));
    let outcome = tx.commit();
    match &outcome {
        TxOutcome::Committed { .. } => Ok(true),
        TxOutcome::NotCommitted {
            why: Why::Conflict { .. } | Why::Refused(tine_store::Refusal::Twin { .. }),
            ..
        } => Ok(false),
        TxOutcome::NotCommitted {
            why: Why::Failed(error),
            ..
        } if error.kind == io::ErrorKind::IsADirectory => Ok(false),
        _ => tx_error(outcome).map(|_| true),
    }
}

/// Copy every Guide page in template order and referenced assets in sorted
/// order. Each file commits independently; an existing page or asset is skipped.
pub fn copy_guide_into_graph(store: &Store, title: &str) -> io::Result<GuideCopyResult> {
    let Some(viewed) = GUIDE_TEMPLATES
        .iter()
        .find(|template| tine_core::refs::same_page(template.title, title))
    else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "unknown bundled guide page",
        ));
    };
    let config = store.config();
    let renames = guide_link_renames();
    let mut created_pages = Vec::new();
    let mut skipped_pages = Vec::new();
    for template in GUIDE_TEMPLATES {
        store
            .scan_refresh()
            .map_err(|error| io::Error::other(format!("{error:?}")))?;
        let graph = store
            .whole_graph()
            .map_err(|error| io::Error::other(format!("{error:?}")))?;
        let name = guide_copy_page_name(template.title);
        if matches!(graph.resolve(&name, false), Resolved::Existing { .. }) {
            skipped_pages.push(name);
            continue;
        }
        let rel = format!(
            "{}.md",
            tine_core::model::encode_page_name(&name, config.config.file_name_format)
        );
        let markdown = rewrite_bundled_guide_links(template.markdown, &renames);
        if create_if_absent(store, Area::Pages, &rel, markdown.as_bytes())? {
            created_pages.push(name);
        } else {
            skipped_pages.push(name);
        }
    }
    let mut referenced = HashSet::new();
    for template in GUIDE_TEMPLATES {
        collect_guide_asset_refs(template.markdown, &mut referenced);
    }
    let mut referenced: Vec<String> = referenced.into_iter().collect();
    referenced.sort();
    let mut copied_assets = Vec::new();
    for name in referenced {
        if name.contains('/') || name.contains('\\') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "guide assets must be top-level files",
            ));
        }
        let Some(asset) = GUIDE_ASSETS.iter().find(|asset| asset.name == name) else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("missing bundled guide asset {name}"),
            ));
        };
        if create_if_absent(store, Area::Assets, &name, asset.bytes)? {
            copied_assets.push(name);
        }
    }
    Ok(GuideCopyResult {
        name: guide_copy_page_name(viewed.title),
        created: !created_pages.is_empty() || !copied_assets.is_empty(),
        created_pages,
        skipped_pages,
        copied_assets,
    })
}
