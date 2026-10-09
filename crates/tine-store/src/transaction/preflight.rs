//! Preflight validation and byte preparation for graph transactions.

use super::*;
use tine_core::model::Format;

impl<'a> Transaction<'a> {
    pub(super) fn preflight(&self, step: &Step) -> Result<Prepared, Why> {
        let unsupported_config_step = match step {
            Step::Trash { file, .. } => file.as_str() == "logseq/config.edn",
            Step::Unique {
                area, stem, ext, ..
            } => *area == Area::Meta && stem == "config" && ext == ".edn",
            Step::Move { file, to, .. } => {
                file.as_str() == "logseq/config.edn" || to.as_str() == "logseq/config.edn"
            }
            _ => false,
        };
        if unsupported_config_step {
            return Err(Why::Refused(Refusal::InvalidTarget(
                "config.edn requires live config publication".into(),
            )));
        }
        match step {
            Step::Expect { file, expected } => Ok(Prepared {
                src: file.clone(),
                dst: None,
                old: Some(self.stage(file, expected)?),
                new: None,
                saved_page: None,
                opaque_rev: None,
            }),
            Step::Save {
                id,
                base,
                doc,
                markers,
            } => {
                let file = id.file();
                self.page_save_target(&file, doc)?;
                if matches!(base, SaveBase::CreateNew) {
                    self.page_create_checks(&file, doc.name.as_str())?;
                }
                let old = match base {
                    // `save_page` maps ResolvingMarkers to Existing + Markers::Resolve.
                    SaveBase::Existing(rev) | SaveBase::ResolvingMarkers(rev) => {
                        Some(self.stage(&file, rev)?)
                    }
                    SaveBase::CreateNew => None,
                };
                let (new, saved_page) =
                    self.serialize_page(&file, doc, old.as_deref(), *markers == Markers::Resolve)?;
                Ok(Prepared {
                    src: file,
                    dst: None,
                    old,
                    new: Some(new),
                    saved_page: Some(saved_page),
                    opaque_rev: None,
                })
            }
            Step::Create { file, content } => {
                if file.as_str().starts_with("logseq/.tine-trash/")
                    || file.as_str().starts_with("logseq/.tine-")
                {
                    return Err(Why::Refused(Refusal::InvalidTarget(file.as_str().into())));
                }
                self.path(file)?;
                self.absent(file)?;
                self.twin(file, None)?;
                self.unreadable_owner(file, &[])?;
                if file.as_str() == "logseq/config.edn" {
                    validate_config_content(self.store, content)?;
                }
                if self.page(file) {
                    validate_page_content(&file, content)?;
                } else if let Content::Stream { source, max_bytes } = content {
                    validate_stream(source, *max_bytes)?;
                }
                Ok(Prepared {
                    src: file.clone(),
                    dst: None,
                    old: None,
                    new: None,
                    saved_page: None,
                    opaque_rev: None,
                })
            }
            Step::Unique {
                area,
                stem,
                ext,
                content,
            } => {
                let first = format!("{stem}{ext}");
                if first.is_empty()
                    || first == "."
                    || first == ".."
                    || first.contains('/')
                    || first.contains('\\')
                    || (!ext.is_empty() && !ext.starts_with('.'))
                {
                    return Err(Why::Refused(Refusal::InvalidTarget(format!("{stem}{ext}"))));
                }
                let file = self
                    .store
                    .file_id(*area, &first)
                    .map_err(|_| Why::Refused(Refusal::InvalidTarget(first.clone())))?;
                if *area == Area::Trash
                    || (*area == Area::Meta && file.as_str().contains("/.tine-"))
                {
                    return Err(Why::Refused(Refusal::InvalidTarget(file.as_str().into())));
                }
                self.path(&file)?;
                if self.page(&file) {
                    validate_page_content(&file, content)?;
                } else if let Content::Stream { source, max_bytes } = content {
                    validate_stream(source, *max_bytes)?;
                }
                for index in 0usize.. {
                    let rel = if index == 0 {
                        first.clone()
                    } else {
                        crate::atomic_file::marked_name(stem, &format!("_{index}"), ext)
                    };
                    let candidate = self
                        .store
                        .file_id(*area, &rel)
                        .map_err(|_| Why::Refused(Refusal::InvalidTarget(rel)))?;
                    // Only occupancy matters here: never read an occupant.
                    if self.occupied(&candidate)? {
                        continue;
                    }
                    self.twin(&candidate, None)?;
                    self.unreadable_owner(&candidate, &[])?;
                    if self.fixed_step_names().contains(&candidate) {
                        return Err(Why::Refused(Refusal::RepeatedFile(candidate)));
                    }
                    break;
                }
                Ok(Prepared {
                    src: file,
                    dst: None,
                    old: None,
                    new: None,
                    saved_page: None,
                    opaque_rev: None,
                })
            }
            Step::Replace {
                file,
                expected,
                bytes,
            } => {
                if self.page(file) || file.as_str().starts_with("logseq/.tine-") {
                    return Err(Why::Refused(Refusal::InvalidTarget(file.as_str().into())));
                }
                let old = self.stage(file, expected)?;
                if file.as_str() == "logseq/config.edn" {
                    validate_config_bytes(self.store, bytes)?;
                }
                Ok(Prepared {
                    src: file.clone(),
                    dst: None,
                    old: Some(old),
                    new: Some(bytes.clone()),
                    saved_page: None,
                    opaque_rev: None,
                })
            }
            Step::Rewrite {
                id,
                expected,
                renames,
                rebind_title,
                prepared,
            } => {
                let file = id.file();
                if !self.page(&file) {
                    return Err(Why::Refused(Refusal::InvalidTarget(file.as_str().into())));
                }
                let old = self.stage(&file, expected)?;
                let path = self.path(&file)?;
                let name_format = self.store.config().file_name_format;
                let reused = prepared
                    .as_ref()
                    .and_then(|prepared| prepared.reuse(&file, &old, name_format));
                let new = if *rebind_title {
                    rewrite_move(&old, &path, renames, name_format)?
                } else if let Some(new) = reused {
                    let is_org = path.extension().and_then(|ext| ext.to_str()) == Some("org");
                    refuse_read_only_org(&old, &new, is_org)?;
                    new
                } else {
                    rewrite(&old, &path, renames, name_format)?
                };
                refuse_marker_rewrite(&old, &new, Format::from_path(&path))?;
                Ok(Prepared {
                    src: file,
                    dst: None,
                    old: Some(old),
                    new: Some(new),
                    saved_page: None,
                    opaque_rev: None,
                })
            }
            Step::Move {
                file,
                expected,
                to,
                renames,
            } => {
                if to.as_str().starts_with("logseq/.tine-") {
                    return Err(Why::Refused(Refusal::InvalidTarget(to.as_str().into())));
                }
                let opaque_rev = if renames.is_none() {
                    self.oversized_page_rev(file, expected)?
                } else {
                    None
                };
                let old = if opaque_rev.is_some() {
                    None
                } else {
                    Some(self.stage(file, expected)?)
                };
                self.available_move_destination(file, to)?;
                self.twin(to, Some(file))?;
                let new = match (&old, renames) {
                    (Some(old), Some(map)) => Some(rewrite_move(
                        old,
                        &self.spelled_path(to)?,
                        map,
                        self.store.config().file_name_format,
                    )?),
                    (Some(old), None) => Some(old.clone()),
                    (None, None) => None,
                    (None, Some(_)) => unreachable!(),
                };
                if let (Some(old), Some(new)) = (&old, &new) {
                    refuse_marker_rewrite(old, new, Format::from_path(&self.path(file)?))?;
                }
                Ok(Prepared {
                    src: file.clone(),
                    dst: Some(to.clone()),
                    old,
                    new,
                    saved_page: None,
                    opaque_rev,
                })
            }
            Step::Trash {
                file,
                expected,
                orphan_only,
            } => {
                if *orphan_only {
                    self.check_orphan_asset(file)?;
                }
                if file.as_str().starts_with("logseq/.tine-trash/") {
                    return Err(Why::Refused(Refusal::InvalidTarget(file.as_str().into())));
                }
                let opaque_rev = self.oversized_page_rev(file, expected)?;
                let old = if opaque_rev.is_some() {
                    None
                } else {
                    Some(self.stage(file, expected)?)
                };
                Ok(Prepared {
                    src: file.clone(),
                    dst: None,
                    old,
                    new: None,
                    saved_page: None,
                    opaque_rev,
                })
            }
        }
    }
}

/// Refusal R-VCS-MARKERS for reference rewrites (og 21a, master a8fd4230d).
/// Threat scenario: an external VCS merge or a sync service left a referrer
/// mid-conflict; rewriting `[[Old]]` inside it would edit one or both sides
/// of a merge the user has not adjudicated. A caller (the rename) skips such
/// files and reports them; this is the store's backstop for any caller that
/// does not. A rewrite that changes nothing, or a byte-exact move, passes.
fn refuse_marker_rewrite(old: &[u8], new: &[u8], format: Format) -> Result<(), Why> {
    if old == new {
        return Ok(());
    }
    let found = std::str::from_utf8(old)
        .map(|text| tine_core::concord_queue::vcs_conflict_markers(text, format))
        .unwrap_or_default();
    if found.is_empty() {
        return Ok(());
    }
    Err(Why::Refused(Refusal::ReadOnly(format!(
        "unresolved VCS merge conflict markers ({}); resolve them first",
        found.join(" ")
    ))))
}

/// A page save's checks by precondition (STEP3 §3.2). The transaction's
/// preflight composes them; the page host runs each where its precondition
/// holds, so none is added and none runs where it did not before.
impl Transaction<'_> {
    /// Every page save, whatever its base: Guide-ephemeral, page target and
    /// nesting depth.
    pub(crate) fn page_save_target(&self, file: &FileId, doc: &PageDto) -> Result<(), Why> {
        if doc.guide {
            return Err(Why::Refused(Refusal::InvalidTarget(
                "Guide pages are ephemeral".into(),
            )));
        }
        if !self.page(file) {
            return Err(Why::Refused(Refusal::InvalidTarget(file.as_str().into())));
        }
        if !crate::model::dto_depth_within_limit(doc) {
            return Err(Why::Refused(Refusal::InvalidTarget(
                "page content nesting exceeds 512 levels".into(),
            )));
        }
        Ok(())
    }

    /// Create-only: a `CreateNew` save, and the page host's open of a path
    /// with no file. `name` is the name the new page claims.
    pub(crate) fn page_create_checks(&self, file: &FileId, name: &str) -> Result<(), Why> {
        if let Some(existing) = self.disk_twin(file)? {
            return Err(Why::Refused(Refusal::Twin {
                existing: PageId::from(existing.as_str()),
            }));
        }
        self.absent(file)?;
        self.twin(file, None)?;
        self.unreadable_owner(file, &[name])
    }

    /// Serialize `doc` over `old`, the bytes it replaces (None creates), with
    /// every content firewall: parse validation of both sides, VCS markers
    /// (unless `resolving` them), the page-header and Org round-trip
    /// firewalls in `prepare_page_bytes`.
    pub(crate) fn serialize_page(
        &self,
        file: &FileId,
        doc: &PageDto,
        old: Option<&[u8]>,
        resolving: bool,
    ) -> Result<(Vec<u8>, Document), Why> {
        if let Some(old) = old {
            crate::model::validate_parse_bytes_for_path(old, &self.path(file)?)
                .map_err(content_refusal)?;
        }
        let path = self.path(file)?;
        let text = match old {
            Some(bytes) => Some(std::str::from_utf8(bytes).map_err(|error| {
                content_refusal(io::Error::new(io::ErrorKind::InvalidData, error))
            })?),
            None => None,
        };
        // Refusal R-VCS-MARKERS (docs/storage-contract.md). Threat
        // scenario: a VCS merge by an external writer left unresolved
        // markers; a rewrite would re-indent them and silently lose a
        // side. This is the one place og serializes page bytes for a
        // save (ordinary, forced, merged and PDF-highlight page saves
        // all reach it, and the page host's submits). Only a resolving save
        // passes.
        let found = text
            .map(|text| {
                tine_core::concord_queue::vcs_conflict_markers(text, Format::from_path(&path))
            })
            .unwrap_or_default();
        if !found.is_empty() && !resolving {
            return Err(Why::Refused(Refusal::ReadOnly(format!(
                "unresolved VCS merge conflict markers ({}); resolve them first",
                found.join(" ")
            ))));
        }
        let (new, saved_page) = self
            .store
            .graph
            .prepare_page_bytes(doc, &path, text)
            .map_err(|error| {
                if error.kind() == io::ErrorKind::PermissionDenied {
                    Why::Refused(Refusal::ReadOnly(error.to_string()))
                } else {
                    Why::Failed(error.into())
                }
            })?;
        crate::model::validate_parse_bytes_for_path(&new, &path).map_err(content_refusal)?;
        Ok((new, saved_page))
    }
}
