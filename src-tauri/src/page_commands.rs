//! The page host's commands and its mail bridge (STEP3 §3, plan v3 P1). A
//! window issues them through `src/document/host/protocol.ts` `HostPort`;
//! mutating commands carry the window's session (R8, S2), so a command from
//! an earlier window or owner binding is not admitted. Every graph-window
//! binding runs a host (`graph.rs` `load_graph_for_label`, step 3b P2b); a
//! binding mid-restore or revoked answers with the stale-binding error.

use crate::state::{off_ui, slot_for_context, GraphContext, GraphSlot, PageHostSlot};
use std::time::Duration;
use tine_core::model::PageDto;
use tine_store::{
    DiskToken, DraftStatus, EditKind, Opened, PageHost, PageId, PageMail, PageOperation,
    PageRefusal, Reloaded,
};

/// Run `work` on the binding's running host, under the slot's host read
/// lock: a restore (S8) or a retirement waits for it, and it for them.
fn with_host<T>(slot: &GraphSlot, work: impl FnOnce(&PageHost) -> T) -> Result<T, String> {
    match &*slot.host.read().unwrap_or_else(|e| e.into_inner()) {
        PageHostSlot::Running(host) => Ok(work(host)),
        PageHostSlot::Off => Err("no page host runs for this graph".into()),
        PageHostSlot::Revoked => Err(crate::state::STALE_BINDING.into()),
    }
}

async fn hosted<T: Send + 'static>(
    ctx: GraphContext<'_>,
    work: impl FnOnce(&PageHost) -> T + Send + 'static,
) -> Result<T, String> {
    let slot = slot_for_context(&ctx)?;
    off_ui(move || with_host(&slot, work)).await
}

/// Where page mail goes: the window that owns the binding (`label`). An
/// adopting window retargets it (`PageHost::retarget`, S2).
pub(crate) fn page_mail_sink(
    app: tauri::AppHandle,
    label: String,
) -> impl FnMut(PageMail) + Send + 'static {
    use tauri::Emitter;
    move |mail| {
        let _ = app.emit_to(label.as_str(), "page-mail", mail);
    }
}

/// Crash-recovery availability for the `load_graph` reply (§4, B-Q1);
/// None with no host.
pub(crate) fn draft_status(slot: &GraphSlot) -> Option<DraftStatus> {
    with_host(slot, PageHost::draft_status).ok()
}

#[tauri::command]
pub(crate) async fn page_window_reloaded(ctx: GraphContext<'_>) -> Result<Reloaded, String> {
    hosted(ctx, PageHost::window_reloaded).await
}

/// `page_open`'s reply: the host's key, or why it was not admitted.
#[derive(serde::Serialize)]
#[serde(untagged)]
pub(crate) enum OpenReply {
    Opened(Opened),
    Refused(PageRefusal),
}

#[tauri::command]
pub(crate) async fn page_open(
    session: u64,
    id: u64,
    path: String,
    name: String,
    ctx: GraphContext<'_>,
) -> Result<OpenReply, String> {
    hosted(ctx, move |host| {
        host.open(session, id, &PageId::from(path.as_str()), &name)
            .map_or_else(OpenReply::Refused, OpenReply::Opened)
    })
    .await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub(crate) async fn page_submit(
    session: u64,
    id: u64,
    key: String,
    dto: PageDto,
    version: u64,
    resolve: Option<DiskToken>,
    kinds: Vec<EditKind>,
    ctx: GraphContext<'_>,
) -> Result<Option<PageRefusal>, String> {
    hosted(ctx, move |host| {
        host.submit(session, id, &key, &dto, version, resolve.as_ref(), &kinds)
            .err()
    })
    .await
}

#[tauri::command]
pub(crate) async fn page_move(
    session: u64,
    id: u64,
    source: (String, PageDto, u64),
    receiver: (String, PageDto, u64),
    kinds: Vec<EditKind>,
    ctx: GraphContext<'_>,
) -> Result<Option<PageRefusal>, String> {
    hosted(ctx, move |host| {
        host.move_blocks(
            session,
            id,
            (&source.0, &source.1, source.2),
            (&receiver.0, &receiver.1, receiver.2),
            &kinds,
        )
        .err()
    })
    .await
}

#[tauri::command]
pub(crate) async fn page_discard(
    session: u64,
    id: u64,
    key: String,
    version: u64,
    ctx: GraphContext<'_>,
) -> Result<Option<PageRefusal>, String> {
    hosted(ctx, move |host| {
        host.discard(session, id, &key, version).err()
    })
    .await
}

#[tauri::command]
pub(crate) async fn page_close(
    session: u64,
    id: u64,
    key: String,
    ctx: GraphContext<'_>,
) -> Result<Option<PageRefusal>, String> {
    hosted(ctx, move |host| host.close(session, id, &key).err()).await
}

#[tauri::command]
pub(crate) async fn page_delete(
    session: u64,
    name: String,
    kind: tine_core::model::PageKind,
    expected_path: Option<String>,
    ctx: GraphContext<'_>,
) -> Result<PageOperation, String> {
    let slot = slot_for_context(&ctx)?;
    off_ui(move || {
        with_host(&slot, |host| {
            tine_graph_features::pages::delete_page_expected(
                &slot.store,
                host,
                session,
                &name,
                kind,
                expected_path.as_deref(),
            )
            .map_err(|error| error.to_string())
        })?
    })
    .await
}

/// One `page_wait` entry (`protocol.ts` `PublishedNeed`).
#[derive(serde::Deserialize)]
pub(crate) struct PublishedNeed {
    key: String,
    version: u64,
    witness: Option<String>,
}

/// The longest one `page_wait` holds the slot's host read lock (REVIEW-3b-P1
/// F1): a restore or a retirement of this binding waits at most this long
/// for it. The window asks again while its barrier still waits.
const PAGE_WAIT_BOUND: Duration = Duration::from_secs(5);

/// For the window `session` (F1): true once every need is published; false
/// on a terminal notice for a needed key, or when the session is no longer
/// current; null at `bound_ms` (at most 5 s), never success there (S1).
#[tauri::command]
pub(crate) async fn page_wait(
    session: u64,
    needs: Vec<PublishedNeed>,
    bound_ms: u64,
    ctx: GraphContext<'_>,
) -> Result<Option<bool>, String> {
    let needs: Vec<_> = needs
        .into_iter()
        .map(|need| (need.key, need.version, need.witness))
        .collect();
    let bound = Duration::from_millis(bound_ms).min(PAGE_WAIT_BOUND);
    hosted(ctx, move |host| {
        match host.wait_published(session, &needs, bound) {
            PageOperation::Applied => Some(true),
            // `wait_published` answers only Applied, Pending or Refused.
            PageOperation::Pending
            | PageOperation::Waiting
            | PageOperation::Uncertain
            | PageOperation::Superseded => None,
            PageOperation::Refused => Some(false),
        }
    })
    .await
}

#[tauri::command]
pub(crate) async fn page_save_now(
    session: u64,
    keys: Vec<String>,
    ctx: GraphContext<'_>,
) -> Result<(), String> {
    hosted(ctx, move |host| host.save_now(session, &keys)).await
}

/// One `page_owed` entry (`protocol.ts` `OwedPage`).
#[derive(serde::Serialize)]
pub(crate) struct OwedPage {
    key: String,
    version: u64,
}

/// The window `session`'s publication debt; null when that session is not
/// current (F1).
#[tauri::command]
pub(crate) async fn page_owed(
    session: u64,
    paths: Option<Vec<String>>,
    ctx: GraphContext<'_>,
) -> Result<Option<Vec<OwedPage>>, String> {
    hosted(ctx, move |host| {
        let paths: Option<Vec<PageId>> = paths.map(|paths| {
            paths
                .iter()
                .map(|path| PageId::from(path.as_str()))
                .collect()
        });
        host.owed(session, paths.as_deref()).map(|owed| {
            owed.into_iter()
                .map(|(key, version)| OwedPage { key, version })
                .collect()
        })
    })
    .await
}

#[tauri::command]
pub(crate) async fn page_drafts_retry(ctx: GraphContext<'_>) -> Result<(), String> {
    hosted(ctx, PageHost::drafts_retry).await?
}
