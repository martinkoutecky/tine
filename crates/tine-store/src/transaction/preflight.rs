//! Preflight validation and byte preparation for graph transactions.

use super::*;

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
                if doc.guide {
                    return Err(Why::Refused(Refusal::InvalidTarget(
                        "Guide pages are ephemeral".into(),
                    )));
                }
                if !self.page(&file) {
                    return Err(Why::Refused(Refusal::InvalidTarget(file.as_str().into())));
                }
                if !crate::model::dto_depth_within_limit(doc) {
                    return Err(Why::Refused(Refusal::InvalidTarget(
                        "page content nesting exceeds 512 levels".into(),
                    )));
                }
                if matches!(base, SaveBase::CreateNew) {
                    if let Some(existing) = self.disk_twin(&file)? {
                        return Err(Why::Refused(Refusal::Twin {
                            existing: PageId::from(existing.as_str()),
                        }));
                    }
                    self.absent(&file)?;
                    self.twin(&file, None)?;
                }
                let old = match base {
                    // `save_page` maps ResolvingMarkers to Existing + Markers::Resolve.
                    SaveBase::Existing(rev) | SaveBase::ResolvingMarkers(rev) => {
                        Some(self.stage(&file, rev)?)
                    }
                    SaveBase::CreateNew => None,
                };
                if let Some(old) = old.as_ref() {
                    crate::model::validate_parse_bytes_for_path(old, &self.path(&file)?)
                        .map_err(content_refusal)?;
                }
                let path = self.path(&file)?;
                let text = match old.as_deref() {
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
                // all reach it). Only `SaveBase::ResolvingMarkers` passes.
                let found = text
                    .map(tine_core::concord_queue::vcs_conflict_markers)
                    .unwrap_or_default();
                if !found.is_empty() && *markers == Markers::Refuse {
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
                crate::model::validate_parse_bytes_for_path(&new, &path)
                    .map_err(content_refusal)?;
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
                        format!("{stem}_{index}{ext}")
                    };
                    let candidate = self
                        .store
                        .file_id(*area, &rel)
                        .map_err(|_| Why::Refused(Refusal::InvalidTarget(rel)))?;
                    match self.absent(&candidate) {
                        Ok(()) => {
                            self.twin(&candidate, None)?;
                            if self.fixed_step_names().contains(&candidate) {
                                return Err(Why::Refused(Refusal::RepeatedFile(candidate)));
                            }
                            break;
                        }
                        Err(Why::Conflict { .. }) => continue,
                        Err(error) => return Err(error),
                    }
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
            } => {
                let file = id.file();
                if !self.page(&file) {
                    return Err(Why::Refused(Refusal::InvalidTarget(file.as_str().into())));
                }
                let old = self.stage(&file, expected)?;
                let path = self.path(&file)?;
                let new = if *rebind_title {
                    rewrite_move(&old, &path, renames, self.store.config().file_name_format)?
                } else {
                    rewrite(&old, &path, renames)?
                };
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
                self.absent(to)?;
                self.twin(to, Some(file))?;
                let new = match (&old, renames) {
                    (Some(old), Some(map)) => Some(rewrite_move(
                        old,
                        &self.path(to)?,
                        map,
                        self.store.config().file_name_format,
                    )?),
                    (Some(old), None) => Some(old.clone()),
                    (None, None) => None,
                    (None, Some(_)) => unreachable!(),
                };
                Ok(Prepared {
                    src: file.clone(),
                    dst: Some(to.clone()),
                    old,
                    new,
                    saved_page: None,
                    opaque_rev,
                })
            }
            Step::Trash { file, expected } => {
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
