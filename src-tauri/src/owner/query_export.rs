//! Internal CLI publication verb: the admitted owner runs the shared publisher.
//! The request contains selectors only; frontend assets come from this binary.
use serde::{Deserialize, Serialize};
use tine_graph_features::publish_query::{
    plan_query, publish_query, query_request_by_uuid, ExportReceipt,
};
use tine_store::Store;

/// Same-binary owner request; CLI syntax and output remain in `cli`.
#[derive(Deserialize, Serialize)]
pub(crate) struct Args {
    pub(crate) uuid: String,
    pub(crate) name: Option<String>,
    pub(crate) force: bool,
}

/// Run publication in the owning Store, preserving review and confirmation.
/// UUID resolution reads indexed candidate documents; query evaluation and
/// selected-page output retain the publisher's costs and refusal semantics.
pub(crate) fn publish(
    store: &Store,
    args: Args,
    bundle: &[(String, Vec<u8>)],
) -> Result<ExportReceipt, String> {
    let request = query_request_by_uuid(store, &args.uuid, args.name.as_deref(), args.force)
        .map_err(|e| e.to_string())?;
    let plan = plan_query(store, &request).map_err(|e| e.to_string())?;
    if plan.exists && !args.force {
        return Err(format!("output already exists at {}; use --force to replace it (the previous export is kept in recovery)", plan.path));
    }
    publish_query(store, &request, &plan.fingerprint, bundle).map_err(|e| e.to_string())
}
