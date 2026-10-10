//! Bundled Guide creation and copying. A demo graph writes O(seed bytes);
//! copying performs one transaction per page and asset, O(copied bytes plus
//! each logical page lookup). Existing files are left untouched. An unknown
//! title and graph I/O errors are returned as I/O errors. Callers need no
//! graph path, name encoding, or transaction details.

use std::io;

use tine_core::guide::{
    guide_copy_page_name, guide_link_renames, rewrite_bundled_guide_links, CONFIG_EDN,
    GUIDE_ASSETS, GUIDE_TEMPLATES, QUICK_CAPTURE_PNG,
};
use tine_store::{Area, Content, Input, OpenError, PageHost, Resolved, Store, TxOutcome, Why};

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
pub fn create_demo_graph(
    parent: &std::path::Path,
    name: &str,
) -> Result<std::path::PathBuf, OpenError> {
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
    Store::create_graph(parent, name, &seed)
}

/// A retained writer (STEP3 §7, A-K2): a page the host holds as a fileless
/// draft is saved first, so the Guide meets it as an existing page and skips it.
fn create_if_absent(
    store: &Store,
    host: Option<&PageHost>,
    area: Area,
    rel: &str,
    bytes: &[u8],
) -> io::Result<bool> {
    let id = store.file_id(area, rel).map_err(store_error)?;
    let discover = || Ok(crate::retained::pages(store, [&id]));
    crate::retained::reserved(
        host,
        Input::Flush,
        discover,
        crate::retained::unsaved,
        |_| {
            let mut tx = if area == Area::Pages {
                store.transaction(Some(tine_store::EditKind::ReplacePage))
            } else {
                store.transaction(None)
            };
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
        },
    )
}

/// Copy every Guide page in template order and the bundled assets in manifest
/// (sorted) order. Each file commits independently; an existing page or asset is skipped.
pub fn copy_guide_into_graph(
    store: &Store,
    host: Option<&PageHost>,
    title: &str,
) -> io::Result<GuideCopyResult> {
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
            .refresh(tine_store::Depth::Stamps)
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
        let markdown = markdown
            .strip_prefix(&format!("title:: {}\n", template.title))
            .map(|rest| format!("title:: {name}\n{rest}"))
            .unwrap_or(markdown);
        if create_if_absent(store, host, Area::Pages, &rel, markdown.as_bytes())? {
            created_pages.push(name);
        } else {
            skipped_pages.push(name);
        }
    }
    let mut copied_assets = Vec::new();
    for asset in GUIDE_ASSETS {
        if create_if_absent(store, host, Area::Assets, asset.name, asset.bytes)? {
            copied_assets.push(asset.name.to_string());
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
