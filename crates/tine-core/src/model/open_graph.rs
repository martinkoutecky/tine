//! Opening a graph: open / open_checked, recovery of interrupted publishes and
//! editor publications, and the write-target guards (graph root, config, trash,
//! assets).

use super::*;

impl Graph {
    /// Open a graph for use by the application, rejecting any configured page or
    /// journal directory that can escape the selected graph. `Graph::open` stays
    /// available for the many in-crate disposable fixtures, but runtime graph
    /// binding must use this checked entry point.
    pub fn open_checked(root: impl AsRef<Path>) -> io::Result<Graph> {
        Self::open_checked_with_assets(root, None)
    }

    /// Resolve an `assets` link/junction that lands outside the graph. The
    /// returned path is canonical and therefore suitable for showing to the user
    /// and binding a device-local approval. An in-graph directory (or a missing
    /// directory that Tine may create normally) returns `None`.
    pub fn external_assets_target(root: impl AsRef<Path>) -> io::Result<Option<PathBuf>> {
        let root = fs::canonicalize(root.as_ref())?;
        let assets = root.join("assets");
        match fs::symlink_metadata(&assets) {
            Ok(_) => {
                let resolved = fs::canonicalize(&assets)?;
                if !resolved.is_dir() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("assets path is not a directory: {}", assets.display()),
                    ));
                }
                Ok((!resolved.starts_with(&root)).then_some(resolved))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Checked runtime open with one narrowly-scoped exception to the graph-root
    /// boundary: an external `assets` link/junction is accepted only when its
    /// current canonical target exactly matches the caller's approved target.
    /// This makes a retargeted link fail closed instead of inheriting old trust.
    pub fn open_checked_with_assets(
        root: impl AsRef<Path>,
        approved_assets: Option<&Path>,
    ) -> io::Result<Graph> {
        let mut graph = Self::open(root);
        validate_graph_dir(&graph.root, &graph.config.journals_dir, "journals")?;
        validate_graph_dir(&graph.root, &graph.config.pages_dir, "pages")?;
        validate_graph_dir(&graph.root, "logseq", "logseq")?;
        validate_graph_dir(&graph.root, "publish", "publish")?;
        // `.tine-sync` (left behind by the removed Managed Storage mode) is not
        // part of Direct Files authority: a graph open must neither inspect nor
        // require its shape, and never modifies it.
        if let Some(resolved) = Self::external_assets_target(&graph.root)? {
            let approved = approved_assets.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "external assets directory requires approval: {}",
                        resolved.display()
                    ),
                )
            })?;
            let approved = fs::canonicalize(approved).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("approved assets directory is unavailable: {error}"),
                )
            })?;
            if approved != resolved {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "external assets directory changed; approved {} but graph now resolves to {}",
                        approved.display(),
                        resolved.display()
                    ),
                ));
            }
            graph.assets_root = resolved;
        } else {
            validate_graph_dir(&graph.root, "assets", "assets")?;
            graph.assets_root = graph.root.join("assets");
        }
        let summary = graph.recover_interrupted_publishes()?;
        *graph.interrupted_publication_claimants.write().unwrap() = summary.claimants;
        Ok(graph)
    }

    /// Restore any small file or Direct editor publication left mid-publish by
    /// a crash.
    ///
    /// [`atomic_replace_expected`] vacates the target name for the length of one
    /// rename, so a crash in that window leaves the content under a `.retired`
    /// sibling and the file itself missing. Small-file recovery scans only its
    /// registered directories. Editor recovery performs one bounded no-follow
    /// graph-scope name walk and never reads unrelated document contents.
    pub fn recover_interrupted_publishes(&self) -> io::Result<RecoverySummary> {
        let recovered = restore_retired_files(&self.root, &[self.root.join("logseq")])?;
        let mut summary = self.recover_interrupted_editor_publications()?;
        summary.reconciled = summary.reconciled.saturating_add(recovered);
        Ok(summary)
    }

    /// Reconcile exact files stranded by a crash inside the Direct Files
    /// retire/publish window. A sole claim for a missing live name is restored
    /// with no-replace. When a live name exists, every recognized artifact is
    /// moved intact to typed recovery trash. Multiple claims for one missing
    /// target stay untouched because choosing one would discard information.
    fn recover_interrupted_editor_publications(&self) -> io::Result<RecoverySummary> {
        let write = self.admit_graph_text_writer()?;
        let (claims, cleaned_retired) = self.editor_publication_recovery_claims(&write)?;
        let mut by_target = std::collections::BTreeMap::<PathBuf, Vec<PathBuf>>::new();
        for (artifact, target) in claims {
            by_target.entry(target).or_default().push(artifact);
        }

        let mut summary = RecoverySummary {
            reconciled: cleaned_retired,
            ..RecoverySummary::default()
        };
        for (target, artifacts) in by_target {
            let target_present = self.graph_text_exists(&write, &target)?;
            if !target_present {
                if artifacts.len() != 1 {
                    summary.record_claimant(self, &target)?;
                    continue;
                }
                let artifact = &artifacts[0];
                let identity =
                    self.graph_text_move_editor_recovery_noreplace(&write, artifact, &target)?;
                if self.graph_text_optional_file_identity(&write, &target)? == Some(identity) {
                    summary.reconciled = summary.reconciled.saturating_add(1);
                } else {
                    summary.record_claimant(self, &target)?;
                }
                continue;
            }

            let trash = typed_trash_dir(&self.root, TrashEntryKind::Conflict);
            self.graph_text_create_dir_all(&write, &trash)?;
            for artifact in artifacts {
                let Some(filename) = artifact.file_name().and_then(|name| name.to_str()) else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "editor recovery artifact has no UTF-8 filename",
                    ));
                };
                let Some(target_name) = editor_recovery_target_name(filename) else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "discovered editor recovery artifact no longer parses",
                    ));
                };
                let Some(extension) = text_extension_from_path(Path::new(target_name)) else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "editor recovery artifact no longer names graph text",
                    ));
                };
                let destination = trash.join(format!(
                    "{}__editor-publication__{}.{}",
                    trash_stamp(),
                    filename.trim_start_matches('.'),
                    extension
                ));
                let identity = self.graph_text_move_editor_recovery_noreplace(
                    &write,
                    &artifact,
                    &destination,
                )?;
                if self.graph_text_optional_file_identity(&write, &destination)? == Some(identity) {
                    summary.reconciled = summary.reconciled.saturating_add(1);
                } else {
                    summary.record_claimant(self, &target)?;
                }
            }
        }
        Ok(summary)
    }

    /// Discover only names emitted by `graph_text_atomic_replace_bound`, through
    /// the retained no-follow graph capability. This is not a suffix glob: the
    /// parser requires the complete producer shape, and the claimed target must
    /// be an eligible graph-text file in the same retained directory.
    fn editor_publication_recovery_claims(
        &self,
        permit: &GraphTextWritePermit,
    ) -> io::Result<(Vec<(PathBuf, PathBuf)>, usize)> {
        struct PendingDirectory {
            directory: Dir,
            relative: String,
            depth: usize,
        }

        let limits = graph_text_inventory_limits();
        let mut pending = vec![PendingDirectory {
            directory: self.graph_text_permit_root(permit)?.try_clone()?,
            relative: String::new(),
            depth: 0,
        }];
        let mut claims = Vec::new();
        let mut cleaned_retired = 0_usize;
        let mut all_entries = 0_usize;
        let mut directories = 1_usize;
        let mut path_bytes = 0_u64;

        while let Some(PendingDirectory {
            directory,
            relative,
            depth,
        }) = pending.pop()
        {
            for entry in directory.entries()? {
                all_entries = all_entries
                    .checked_add(1)
                    .ok_or_else(|| graph_text_inventory_limit_error("all directory entries"))?;
                if all_entries > limits.all_entries {
                    return Err(graph_text_inventory_limit_error("all directory entries"));
                }
                let entry = entry?;
                let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                    continue;
                };
                let child_relative = if relative.is_empty() {
                    name.clone()
                } else {
                    format!("{relative}/{name}")
                };
                path_bytes = path_bytes
                    .checked_add(usize_to_u64(child_relative.len())?)
                    .ok_or_else(|| graph_text_inventory_limit_error("aggregate path bytes"))?;
                if path_bytes > limits.path_bytes {
                    return Err(graph_text_inventory_limit_error("aggregate path bytes"));
                }
                let file_type = entry.file_type()?;
                if file_type.is_symlink() {
                    continue;
                }
                if file_type.is_dir() {
                    if !self.graph_text_scope.should_descend(&child_relative) {
                        continue;
                    }
                    let child_depth = depth
                        .checked_add(1)
                        .ok_or_else(|| graph_text_inventory_limit_error("graph directory depth"))?;
                    if child_depth > limits.directory_depth {
                        return Err(graph_text_inventory_limit_error("graph directory depth"));
                    }
                    directories = directories
                        .checked_add(1)
                        .ok_or_else(|| graph_text_inventory_limit_error("directory count"))?;
                    if directories > limits.directories {
                        return Err(graph_text_inventory_limit_error("directory count"));
                    }
                    projection_real_directory(&directory, &name)?;
                    pending.push(PendingDirectory {
                        directory: open_projection_dir_nofollow(&directory, &name)?,
                        relative: child_relative,
                        depth: child_depth,
                    });
                    continue;
                }
                if !file_type.is_file() {
                    continue;
                }
                if let Some(target_name) = editor_retired_target_name(&name) {
                    let target_relative = if relative.is_empty() {
                        target_name.to_owned()
                    } else {
                        format!("{relative}/{target_name}")
                    };
                    if !self.graph_text_scope.is_eligible(&target_relative) {
                        continue;
                    }
                    // `.editor-retired` is a durable cleanup-only state emitted
                    // after the replacement is already live and validated. It
                    // is never a publication claimant and can never restore a
                    // document. A failed unlink aborts checked open, leaves the
                    // exact entry intact, and is retried on the next open.
                    editor_retired_cleanup_hook()?;
                    match directory.remove_file(&name) {
                        Ok(()) => {
                            cleaned_retired = cleaned_retired.saturating_add(1);
                        }
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error),
                    }
                    continue;
                }
                let Some(target_name) = editor_recovery_target_name(&name) else {
                    continue;
                };
                let target_relative = if relative.is_empty() {
                    target_name.to_owned()
                } else {
                    format!("{relative}/{target_name}")
                };
                if !self.graph_text_scope.is_eligible(&target_relative) {
                    continue;
                }
                claims.push((
                    self.root.join(&child_relative),
                    self.root.join(target_relative),
                ));
            }
        }
        Ok((claims, cleaned_retired))
    }

    pub(crate) fn ensure_write_target(&self, target: &Path) -> io::Result<()> {
        self.ensure_within_graph_root(target)
    }

    /// Pure containment: the target must resolve inside this graph root. Split
    /// out of `ensure_write_target` so the asset capability can reuse the check
    /// without inheriting the graph-text read-only refusal -- `assets/` is
    /// not graph text and stays writable in a read-only view.
    pub(super) fn ensure_within_graph_root(&self, target: &Path) -> io::Result<()> {
        if path_stays_within_root(&self.root, target)
            && !path_uses_graph_text_alias(&self.root, target)
        {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("write target escapes graph root: {}", target.display()),
            ))
        }
    }

    /// Graph configuration has its own capability boundary.
    ///
    /// `logseq/config.edn` is **not** graph text: no page read, save or projection
    /// ever covers it. Configuration is therefore writable in a read-only view — a
    /// Settings toggle is not a page edit.
    ///
    /// Narrowed to that one exact path so the capability can never widen into a
    /// general `logseq/` write.
    pub(crate) fn ensure_config_write_target(&self, target: &Path) -> io::Result<()> {
        if target != self.root.join("logseq").join("config.edn") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "write target is not this graph's configuration: {}",
                    target.display()
                ),
            ));
        }
        self.ensure_within_graph_root(target)
    }

    /// The recoverable trash tree has its own capability boundary.
    ///
    /// `logseq/.tine-trash` sits beside `assets`, `publish` and `.tine-sync` in
    /// `graph_text_scope::fixed_excluded`, so nothing under it is ever scanned,
    /// imported or projected: it is not graph text, exactly the way `assets/` is.
    /// Only the *destination* is covered here. Page and
    /// journal trashing still passes through [`Graph::admit_graph_text_writer`]
    /// because their sources are graph text. A recognized sync-conflict copy is
    /// excluded from the document domain too, so its explicit discard path uses
    /// this point capability just like an asset does.
    pub(super) fn ensure_trash_write_target(&self, target: &Path) -> io::Result<()> {
        let trash = trash_root(&self.root);
        if target != trash && !target.starts_with(&trash) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "write target is not inside the recoverable trash: {}",
                    target.display()
                ),
            ));
        }
        self.ensure_within_graph_root(target)
    }

    /// Asset writes have their own capability boundary. Keeping this separate
    /// from `ensure_write_target` means approving external assets cannot widen a
    /// page/config/publish write into the same directory.
    pub(super) fn ensure_asset_write_target(&self, target: &Path) -> io::Result<()> {
        if self.assets_root == self.root.join("assets") {
            return self.ensure_within_graph_root(target);
        }
        if path_stays_within_root(&self.assets_root, target)
            && !path_uses_graph_text_alias(&self.assets_root, target)
        {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "write target escapes approved assets root: {}",
                    target.display()
                ),
            ))
        }
    }

    /// Open a graph directory, reading `logseq/config.edn` if present.
    pub fn open(root: impl AsRef<Path>) -> Graph {
        Self::open_inner(root)
    }

    fn open_inner(root: impl AsRef<Path>) -> Graph {
        let root = root.as_ref().to_path_buf();
        let projection_root = open_projection_root_nofollow(&root).ok();
        let graph_text_write_binding =
            graph_text_write_binding_for_resource(&root, projection_root.as_ref());
        let guarded_resource_epoch = graph_text_write_binding
            .as_ref()
            .ok()
            .map(|binding| binding.gate.identity_mutation_epoch());
        let config_path = reconciliation_scan_config_path_at_open(&root);
        let config_bytes = fs::read(config_path).ok();
        let config_text = config_bytes
            .as_deref()
            .and_then(|bytes| std::str::from_utf8(bytes).ok());
        let config = config_text.map(Config::parse).unwrap_or_default();
        let journal_format = JournalFormat::new(
            config.journal_file_name_format.as_deref(),
            config.journal_page_title_format.as_deref(),
        );
        let graph_text_scope =
            GraphTextScope::new(&config.hidden, config.hidden_parse_failed_closed);
        let graph_text_admission_instance = Arc::new(GraphTextAdmissionInstance);
        Graph {
            assets_root: root.join("assets"),
            projection_root,
            interrupted_publication_claimants: RwLock::new(std::collections::BTreeSet::new()),
            root,
            config,
            graph_text_scope,
            reconciliation_scan_open_config_description: config_bytes
                .as_deref()
                .map(BlobDescription::of),
            recent_config_write: RwLock::new(None),
            graph_text_admission_instance,
            guarded_graph_text_identity: RwLock::new(GuardedGraphTextIdentityState {
                observed_resource_epoch: guarded_resource_epoch,
                ..GuardedGraphTextIdentityState::default()
            }),
            journal_format,
            cache: RwLock::new(None),
            session_page_ids: RwLock::new(std::collections::HashMap::new()),
            projection_recovery: std::sync::Mutex::new(()),
            page_index_failures: RwLock::new(Vec::new()),
            cache_index: RwLock::new(None),
            effective_identity_index: RwLock::new(None),
            cache_gen: std::sync::atomic::AtomicU64::new(0),
            cache_structural_gen: std::sync::atomic::AtomicU64::new(0),
            indexing_progress: Default::default(),
            external_observation_epoch: std::sync::atomic::AtomicU64::new(0),
            external_reconciled_epoch: std::sync::atomic::AtomicU64::new(0),
            external_observation_instance: NEXT_EXTERNAL_OBSERVATION_INSTANCE
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            page_build_flight: std::sync::Mutex::new(None),
            retired: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            page_build_test: PageBuildTestState::default(),
            derived_cache: RwLock::new(None),
            direct_projection: std::sync::Mutex::new(None),
            page_list_cache: RwLock::new(None),
            referenced_names_cache: RwLock::new(None),
            find_entry_cache: RwLock::new(None),
            recent_writes: std::sync::Mutex::new(std::collections::HashMap::new()),
            recent_graph_text_states: std::sync::Mutex::new(std::collections::HashMap::new()),
            concord_ledger: std::sync::OnceLock::new(),
            marker_resolutions: std::sync::Mutex::new(std::collections::HashSet::new()),
            disk_revs: RwLock::new(std::collections::HashMap::new()),
            loaded_file_identities: RwLock::new(std::collections::HashMap::new()),
            conflict_authority: std::sync::Mutex::new(ConflictAuthorityState::default()),
            editor_activations: std::sync::Mutex::new(EditorActivationState::default()),
            page_locks: std::sync::Mutex::new(std::collections::HashMap::new()),
            graph_text_write_binding,
            search_lanes: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}
