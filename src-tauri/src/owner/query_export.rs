//! Internal CLI publication verb: the admitted owner runs the shared publisher.
//! The request contains selectors only; frontend assets come from this binary.
use serde::{Deserialize, Serialize};
use tine_graph_features::publish_query::{
    plan_query, publish_query_at, query_request_by_uuid, query_request_inline,
    query_request_on_page, ExportReceipt,
};
use tine_store::Store;

/// Same-binary owner request; CLI syntax and output remain in `cli`.
#[derive(Deserialize, Serialize)]
pub(crate) struct Args {
    #[serde(flatten)]
    pub(crate) source: Source,
    pub(crate) name: Option<String>,
    pub(crate) force: bool,
    #[serde(default)]
    pub(crate) no_comments: bool,
    /// The invoking CLI's export clock (its `SOURCE_DATE_EPOCH` or now), so
    /// the owner's own environment never stamps the output.
    pub(crate) epoch: i64,
}

/// One source, selected by the CLI's mutually exclusive argument group.
#[derive(Deserialize, Serialize)]
#[serde(untagged)]
pub(crate) enum Source {
    Uuid { uuid: String },
    Inline { query: String },
    Page { page: String },
}

/// Run publication in the owning Store, preserving review and confirmation.
/// UUID resolution reads indexed candidate documents; query evaluation and
/// selected-page output retain the publisher's costs and refusal semantics.
pub(crate) fn publish(
    store: &Store,
    args: Args,
    bundle: &[(String, Vec<u8>)],
) -> Result<ExportReceipt, String> {
    let name = args.name.as_deref();
    let mut request = match args.source {
        Source::Uuid { uuid } => query_request_by_uuid(store, &uuid, name, args.force),
        Source::Inline { query } => Ok(query_request_inline(
            query,
            name.ok_or("inline --query requires --name")?,
            args.force,
        )),
        Source::Page { page } => {
            let graph = store
                .whole_graph()
                .map_err(|e| format!("graph load failed: {e:?}"))?;
            let id = super::verbs::resolve_read_page(&graph, &page).map_err(|e| e.message)?;
            query_request_on_page(&graph, &id, name, args.force)
        }
    }
    .map_err(|e| e.to_string())?;
    if args.no_comments {
        request.include_comments = Some(false);
    }
    let plan = plan_query(store, &request).map_err(|e| e.to_string())?;
    if plan.exists && !args.force {
        return Err(format!("output already exists at {}; use --force to replace it (the previous export is kept in recovery)", plan.path));
    }
    publish_query_at(store, &request, &plan.fingerprint, bundle, args.epoch)
        .map_err(|e| e.to_string())
}
