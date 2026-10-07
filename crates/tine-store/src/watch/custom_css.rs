//! The `logseq/custom.css` lane: one external edit of the user's stylesheet is
//! one publication, so the window can re-apply it without reopening the graph.
//!
//! Interface. `Core::observe_custom_css` answers "did the bytes of
//! `logseq/custom.css` change since the last observation?". It costs one stat
//! and one hash of a small user file per call, and it runs only when an event
//! named the file, an event batch needed a full stat diff, or a poll cycle
//! (which already walks the graph) ran; there is no timer of its own. A change
//! is published as an `Origin::External` file tuple with no page and no config
//! change (the asset lane's shape), so it costs no reparse and no scan. The
//! store's own creation or replacement of the file updates the stamp instead
//! (`note_own`), so a window never hears an echo of its own write.

use super::*;

const CUSTOM_CSS: &str = "logseq/custom.css";

impl Core {
    /// Publish `logseq/custom.css` when its bytes differ from the last
    /// observation; metadata-only changes (a `touch`) publish nothing. Takes
    /// the writer lock so it orders with transactions like every other lane.
    pub(super) fn observe_custom_css(&self) {
        let _writer = self.writer.lock().unwrap();
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        let path = self.graph.root.join(CUSTOM_CSS);
        let mut current = stamp(&path);
        let mut previous = self.custom_css_stamp.lock().unwrap();
        // A file that vanished or failed to read between its metadata and its
        // bytes (an editor's save-by-rename): look again before calling it gone.
        if (current.is_none() && previous.is_some())
            || current.as_ref().is_some_and(|value| value.rev.is_none())
        {
            current = stamp(&path);
        }
        let rev = current.as_ref().and_then(|value| value.rev.clone());
        if previous.as_ref().and_then(|value| value.rev.as_ref()) == rev.as_ref() {
            *previous = current;
            return;
        }
        let kind = match (previous.as_ref(), current.as_ref()) {
            (None, Some(_)) => ChangeKind::Created,
            (Some(_), None) => ChangeKind::Removed,
            _ => ChangeKind::Modified,
        };
        *previous = current;
        drop(previous);
        self.changes.publish_watched(
            Origin::External,
            vec![(FileId::from(CUSTOM_CSS.to_owned()), kind, rev)],
            false,
            Vec::new(),
            || {},
            None,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{OpenOptions, Store};
    use crate::{Area, Content, TxOutcome};

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "tine-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("pages")).unwrap();
        fs::create_dir_all(root.join("journals")).unwrap();
        fs::create_dir_all(root.join("logseq")).unwrap();
        root
    }

    fn css_change(
        subscription: &crate::store::Subscription,
    ) -> Option<(ChangeKind, Option<FileRev>)> {
        let deadline = Instant::now() + Duration::from_secs(12);
        while Instant::now() < deadline {
            if let Some(change) = subscription.try_recv().unwrap() {
                let named = change
                    .files
                    .iter()
                    .find(|(id, _, _)| id.as_str() == CUSTOM_CSS);
                if let (Origin::External, Some((_, kind, rev))) = (change.origin, named) {
                    assert_eq!(change.files.len(), 1, "custom.css is its own publication");
                    return Some((*kind, rev.clone()));
                }
            } else {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        None
    }

    /// An event naming `logseq/custom.css` takes the stylesheet lane only: no
    /// page stat diff, no config re-read, no asset scan. Any other `logseq/`
    /// file (or a differently cased spelling on a case-folding volume) is
    /// classified by name, not by being under `logseq/`.
    #[test]
    fn a_custom_css_event_flags_only_its_own_lane() {
        let root = temp_root("css-lane");
        let graph = crate::model::Graph::open(&root);
        let scope = AssetScope::new(&graph);
        let config = tine_core::Config::default();
        let dirs = [root.clone()];
        let event = |paths: Vec<PathBuf>| notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Any,
            )),
            paths,
            attrs: Default::default(),
        };

        let mut pending = Pending::default();
        let css = event(vec![root.join("logseq/custom.css")]);
        assert!(pending.add_event(Ok(css), &dirs, &config, &scope));
        assert!(pending.custom_css, "the stylesheet lane was not flagged");
        assert!(!pending.config && !pending.full && pending.paths.is_empty());

        let mut pending = Pending::default();
        let other = event(vec![root.join("logseq/Custom.CSS")]);
        assert!(pending.add_event(Ok(other), &dirs, &config, &scope));
        assert!(
            pending.custom_css,
            "a case-folded spelling must reach the lane"
        );

        let mut pending = Pending::default();
        let notes = event(vec![root.join("logseq/bak.css")]);
        pending.add_event(Ok(notes), &dirs, &config, &scope);
        assert!(!pending.custom_css, "only custom.css belongs to the lane");

        let mut pending = Pending::default();
        let config_edit = event(vec![root.join("logseq/config.edn")]);
        assert!(pending.add_event(Ok(config_edit), &dirs, &config, &scope));
        assert!(pending.config && !pending.custom_css);
        fs::remove_dir_all(root).unwrap();
    }

    /// The lane, end to end through the store: an outside create, edit and
    /// delete of `logseq/custom.css` each publish one external file tuple; a
    /// metadata-only touch publishes none; the store's own creation is not
    /// echoed back as an external change.
    #[test]
    fn external_custom_css_edits_publish_and_own_writes_do_not_echo() {
        let root = fs::canonicalize(temp_root("css-publish")).unwrap();
        let (store, _, _) = Store::open(
            &root,
            OpenOptions {
                watch: WatchMode::Notify,
                ..OpenOptions::default()
            },
        )
        .unwrap();
        store.whole_graph().unwrap();
        let subscription = store.subscribe();
        let path = root.join("logseq/custom.css");

        fs::write(&path, "a { color: red }\n").unwrap();
        let (kind, rev) = css_change(&subscription).expect("create was never published");
        assert_eq!(kind, ChangeKind::Created);
        assert_eq!(rev, FileRev::from_file(&path).ok());

        fs::write(&path, "a { color: blue }\n").unwrap();
        let (kind, rev) = css_change(&subscription).expect("edit was never published");
        assert_eq!(kind, ChangeKind::Modified);
        assert_eq!(rev, FileRev::from_file(&path).ok());

        // Same bytes, new mtime: nothing to re-apply.
        fs::write(&path, "a { color: blue }\n").unwrap();
        assert!(
            css_change(&subscription).is_none(),
            "a rewrite with identical bytes published a change"
        );

        fs::remove_file(&path).unwrap();
        let (kind, rev) = css_change(&subscription).expect("delete was never published");
        assert_eq!((kind, rev), (ChangeKind::Removed, None));

        // The store's own create through a guarded transaction stamps the file
        // as its own: the watcher must not publish it again as external.
        let id = store.file_id(Area::Meta, "custom.css").unwrap();
        let mut tx = store.transaction(None);
        tx.create(&id, Content::Bytes(b"/* own */\n".to_vec()));
        assert!(matches!(tx.commit(), TxOutcome::Committed { .. }));
        assert!(
            css_change(&subscription).is_none(),
            "an own write echoed back as an external custom.css change"
        );
        store.close();
        fs::remove_dir_all(root).unwrap();
    }
}
