//! Re-share keeps the reviewed revision and membership in memory.
use super::*;
use std::collections::BTreeMap;
use tine_core::refs::page_key;

pub(super) struct UpdateReview {
    row: ShareRow,
    revision: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Membership {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub current_snapshot_at: String,
}

pub(super) struct CurrentShare {
    pub title: String,
    pub created_at: String,
    pub pages: BTreeMap<String, String>,
}

fn legacy_summary(bytes: &[u8]) -> Result<(String, String, Vec<String>), QsError> {
    let doc = share::validate_legacy_document(bytes).map_err(|_| QsError::Unavailable)?;
    let title = doc["title"].as_str().ok_or(QsError::Unavailable)?;
    let date = doc["created_at"].as_str().ok_or(QsError::Unavailable)?;
    let snapshot = &doc["snapshot"];
    let pages = snapshot["pages"].as_array().ok_or(QsError::Unavailable)?;
    if doc["quick_share"] != 1
        || snapshot["schema"] != 1
        || !pages
            .iter()
            .all(|page| page["name"].is_string() && page["path"].is_string())
    {
        return Err(QsError::Unavailable);
    }
    let names = pages
        .iter()
        .filter(|p| p["path"].as_str().is_some_and(|path| !path.is_empty()))
        .filter_map(|p| p["name"].as_str())
        .map(str::to_owned)
        .collect();
    Ok((title.to_owned(), date.to_owned(), names))
}

fn names_by_key(names: impl IntoIterator<Item = String>) -> BTreeMap<String, String> {
    names
        .into_iter()
        .map(|name| (page_key(&name), name))
        .collect()
}

/// Pages compared by page identity (`page_key`), so a case-only rename is
/// neither added nor removed. Returns (added, removed) display names.
pub(super) fn membership_diff<'a>(
    old: &BTreeMap<String, String>,
    new: impl IntoIterator<Item = &'a str>,
) -> (Vec<String>, Vec<String>) {
    let new = names_by_key(new.into_iter().map(str::to_owned));
    let added = new
        .iter()
        .filter(|(key, _)| !old.contains_key(*key))
        .map(|(_, name)| name.clone())
        .collect();
    let removed = old
        .iter()
        .filter(|(key, _)| !new.contains_key(*key))
        .map(|(_, name)| name.clone())
        .collect();
    (added, removed)
}

impl Service {
    pub(super) fn update_lock(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.update_locks.lock().unwrap_or_else(|p| p.into_inner());
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(id).and_then(std::sync::Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        locks.insert(id.to_owned(), Arc::downgrade(&lock));
        lock
    }

    /// GET identifies a revision and raw_url pins that exact revision, avoiding
    /// a mixed preview if another writer updates between the two reads.
    pub(super) async fn read_update_snapshot(
        &self,
        account: &Account,
        row: &ShareRow,
    ) -> Result<(String, CurrentShare), QsError> {
        let gist_id = row.gist_id.as_deref().ok_or(QsError::NotFound)?;
        let gist = self.github.gist(&account.token, gist_id).await?;
        if gist.id != gist_id || !is_tine_share(&gist) || gist.owner_id != Some(account.account_id)
        {
            return Err(QsError::Refused {
                message: "this gist is not a Tine share of the connected account".into(),
            });
        }
        let revision = gist.revision.ok_or(QsError::Unavailable)?;
        if revision.len() != 40 || !revision.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(QsError::Unavailable);
        }
        let raw = gist.share_raw_url.ok_or(QsError::Unavailable)?;
        let raw_revision = raw
            .split_once(&format!("/{gist_id}/raw/"))
            .and_then(|(_, rest)| rest.split('/').next());
        if !raw_revision
            .is_some_and(|value| value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(QsError::Unavailable);
        }
        let envelope = self.github.raw_envelope(&raw).await?;
        let envelope = envelope.strip_suffix('\n').unwrap_or(&envelope);
        let key =
            decode_key(row).ok_or_else(|| records_error("a share's key is damaged".into()))?;
        let payload = share::open(envelope, &key).map_err(|_| QsError::Refused {
            message: "the key does not match, or the share was changed".into(),
        })?;
        if envelope_share_id(envelope).as_deref() != share::b64u_decode(&row.share_id).as_deref() {
            return Err(QsError::Refused {
                message: "the share id does not match this device's record".into(),
            });
        }
        let bytes = share::gunzip_bounded(&payload, share::MAX_DOCUMENT_BYTES)
            .map_err(|_| QsError::Unavailable)?;
        let (title, created_at, names) = match row.viewer_generation {
            1 => legacy_summary(&bytes)?,
            share::GENERATION => {
                share::read_share_summary(&bytes).map_err(|_| QsError::Unavailable)?
            }
            _ => return Err(QsError::Unavailable),
        };
        if parse_time(&created_at).is_none() {
            return Err(QsError::Unavailable);
        }
        Ok((
            revision,
            CurrentShare {
                title,
                created_at,
                pages: names_by_key(names),
            },
        ))
    }

    pub(super) async fn review_update(
        &self,
        mut result: PrepareResult,
        share_id: &str,
    ) -> Result<PrepareResult, QsError> {
        let reviewed = async {
            let lock = self.update_lock(share_id);
            let _guard = lock.lock().await;
            let (account, row) = self.owned_row(share_id)?;
            let (revision, document) = self.read_update_snapshot(&account, &row).await?;
            let new = result.preview.pages.iter().map(|p| p.name.as_str());
            let (added, removed) = membership_diff(&document.pages, new);
            result.membership = Some(Membership {
                current_snapshot_at: document.created_at,
                added,
                removed,
            });
            let mut all = self.prepared.lock().unwrap_or_else(|p| p.into_inner());
            let prepared = all
                .iter_mut()
                .find(|p| p.id == result.prepare_id && p.expires > Instant::now())
                .ok_or(QsError::PreviewExpired)?;
            if row.viewer_generation == 1 && prepared.generation == share::GENERATION {
                // The frozen viewer needs its original gist. This preview keeps
                // its fresh id and uses the ordinary pending/create protocol.
                prepared.replacement_account = Some(account.account_id);
                return Ok(());
            }
            if row.viewer_generation != prepared.generation {
                return Err(QsError::Disabled);
            }
            prepared.share_id = share::b64u_decode(&row.share_id)
                .and_then(|id| id.try_into().ok())
                .ok_or(QsError::NotFound)?;
            prepared.update = Some(UpdateReview { row, revision });
            result.share_id = share_id.to_owned();
            Ok(())
        }
        .await;
        if let Err(error) = reviewed {
            self.discard_prepared(&result.prepare_id);
            return Err(error);
        }
        Ok(result)
    }

    pub(super) fn refresh_update_record(
        &self,
        row: &ShareRow,
        document: &CurrentShare,
    ) -> Result<(), QsError> {
        self.records
            .update_shares(|rows| {
                if let Some(current) = rows.iter_mut().find(|r| {
                    r.share_id == row.share_id && r.key == row.key && r.gist_id == row.gist_id
                }) {
                    current.title.clone_from(&document.title);
                    current.pages = document.pages.len();
                }
            })
            .map_err(records_error)
    }

    pub(super) async fn update_share(
        &self,
        prepared: Prepared,
        copy: impl FnOnce(&str) -> bool,
    ) -> Result<Created, QsError> {
        let review = prepared.update.as_ref().ok_or(QsError::PreviewExpired)?;
        let lock = self.update_lock(&review.row.share_id);
        let _guard = lock.lock().await;
        let (account, row) = self.owned_row(&review.row.share_id)?;
        if row != review.row {
            return Err(QsError::ShareChanged);
        }
        let key = decode_key(&row).ok_or(QsError::NotFound)?;
        let envelope = share::seal_update(&prepared.payload, &key, &prepared.share_id)
            .map_err(|_| QsError::Unavailable)?;
        let gist_id = row.gist_id.as_deref().ok_or(QsError::NotFound)?;
        // Last network read immediately before PATCH. No undocumented If-Match.
        let current = self.github.gist(&account.token, gist_id).await?;
        if current.id != gist_id
            || current.owner_id != Some(account.account_id)
            || !is_tine_share(&current)
            || current.revision.as_deref() != Some(&review.revision)
        {
            return Err(QsError::ShareChanged);
        }
        match self
            .github
            .update_gist(&account.token, gist_id, &envelope)
            .await
        {
            Ok(()) => {}
            Err(GhError::Unknown | GhError::Server(_) | GhError::BadAnswer) => {
                return Err(QsError::MaybeUpdated)
            }
            Err(error) => return Err(error.into()),
        }
        self.records
            .update_shares(|rows| {
                if let Some(current) = rows.iter_mut().find(|r| **r == row) {
                    current.title = prepared.title;
                    current.pages = prepared.pages;
                }
            })
            .map_err(|_| QsError::MaybeUpdated)?;
        Ok(Created {
            share_id: row.share_id.clone(),
            gist_id: gist_id.to_owned(),
            login: row.login.clone(),
            copied: copy(&record_link(&row, gist_id, &key)?),
        })
    }
}
