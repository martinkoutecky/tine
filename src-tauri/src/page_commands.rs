//! The page host's commands and its mail bridge (STEP3 §3, plan v3 P1). A
//! window issues them through `src/document/host/protocol.ts` `HostPort`;
//! mutating commands carry the window's session (R8, S2), so a command from
//! an earlier window or owner binding is not admitted. With no host running
//! for the window's graph each command fails with "no page host"; no
//! production host starts before step 3b P2b.

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
    page: String,
    ctx: GraphContext<'_>,
) -> Result<PageOperation, String> {
    hosted(ctx, move |host| {
        host.delete(session, &PageId::from(page.as_str()))
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

/// True once every need is published; false on a terminal notice for a
/// needed key or at `bound_ms` (S1). The wait holds the host read lock, so a
/// restore or retirement of this binding waits at most `bound_ms` for it.
#[tauri::command]
pub(crate) async fn page_wait(
    needs: Vec<PublishedNeed>,
    bound_ms: u64,
    ctx: GraphContext<'_>,
) -> Result<bool, String> {
    let needs: Vec<_> = needs
        .into_iter()
        .map(|need| (need.key, need.version, need.witness))
        .collect();
    hosted(ctx, move |host| {
        host.wait_published(&needs, Duration::from_millis(bound_ms))
    })
    .await
}

#[tauri::command]
pub(crate) async fn page_save_now(keys: Vec<String>, ctx: GraphContext<'_>) -> Result<(), String> {
    hosted(ctx, move |host| host.save_now(&keys)).await
}

/// One `page_owed` entry (`protocol.ts` `OwedPage`).
#[derive(serde::Serialize)]
pub(crate) struct OwedPage {
    key: String,
    version: u64,
}

#[tauri::command]
pub(crate) async fn page_owed(
    paths: Option<Vec<String>>,
    ctx: GraphContext<'_>,
) -> Result<Vec<OwedPage>, String> {
    hosted(ctx, move |host| {
        let paths: Option<Vec<PageId>> = paths.map(|paths| {
            paths
                .iter()
                .map(|path| PageId::from(path.as_str()))
                .collect()
        });
        host.owed(paths.as_deref())
            .into_iter()
            .map(|(key, version)| OwedPage { key, version })
            .collect()
    })
    .await
}

#[tauri::command]
pub(crate) async fn page_drafts_retry(ctx: GraphContext<'_>) -> Result<(), String> {
    hosted(ctx, PageHost::drafts_retry).await?
}
