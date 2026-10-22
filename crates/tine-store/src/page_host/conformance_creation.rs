//! L8-02's extra production refusal: independently witnessed physical owners,
//! a saveFail successor, and unchanged buffers, drafts and graph files.
use super::*;

impl<F: ConformanceIo> Driver<F> {
    pub(super) fn creating_owner_predicate(&self) -> Option<String> {
        let job = self.host.job.as_ref()?;
        if job.base != Base::Known(None) || job.bytes.is_none() {
            return None;
        }
        let physical = self.host.fs.physical();
        // Extra owner vehicles are explicit environmental inputs, independent
        // of the ModelFs adapter's scripted refusal. Existing corpora have none.
        if !physical
            .files
            .keys()
            .any(|path| path.starts_with("graph/owner-"))
        {
            return None;
        }
        let root = tempfile::tempdir().unwrap();
        native_conformance::write_pages(physical, &root.path().join("pages"));
        let store = crate::Store::open(root.path(), Default::default())
            .unwrap()
            .0;
        store.whole_graph_reconciled().unwrap();
        let file = crate::FileId::from(format!("pages/{}", job.page));
        let source = self
            .host
            .order
            .partner(&job.page)
            .map(|key| crate::FileId::from(format!("pages/{key}")));
        let proposed = store.graph.entry_for_path_in(
            &root.path().join(file.as_str()),
            Some(std::str::from_utf8(job.bytes.as_deref().unwrap()).unwrap()),
        );
        let result = store.transaction(None).creating_save_owner(
            &file,
            proposed.as_ref().map(|entry| entry.name.as_str()),
            source.as_ref(),
        );
        store.close();
        match result {
            Err(crate::Why::Refused(crate::Refusal::Twin { existing })) => {
                Some(existing.as_str().into())
            }
            Err(crate::Why::Refused(crate::Refusal::UnreadableOwner { file })) => {
                Some(file.as_str().into())
            }
            Ok(()) => None,
            other => panic!("unexpected owner predicate: {other:?}"),
        }
    }

    pub(super) fn assert_creating_refusal(&self, before: &Untouched) {
        // saveFail removes only the current page's unpublished temp; compare
        // every other file, both stable files and the user's logical state.
        let mut expected = before.files.clone();
        let page = before.state["job"]["p"].as_u64().unwrap() as usize;
        expected.remove(&format!("temp/page/{}", key(page)));
        assert_eq!(
            self.host.fs.physical().files,
            expected,
            "owner refusal changed files"
        );
        assert_eq!(
            self.host.fs.physical().stable,
            before.stable,
            "owner refusal changed stable files"
        );
        assert_eq!(
            self.host.version, before.version,
            "owner refusal changed version"
        );
        assert_eq!(
            self.versions, before.versions,
            "owner refusal changed recorded versions"
        );
        for component in ["pages", "drafts", "up", "gates", "mb", "w"] {
            assert_eq!(
                self.abstract_state()[component],
                before.state[component],
                "owner refusal changed {component}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creating() -> Driver {
        let mut d = Driver::new("base", 3);
        run(
            &mut d,
            &[
                ("load", json!([2])),
                ("opApply", json!([[[2, 3]], [[2, ABSENT]], NONE, NONE])),
            ],
        );
        d
    }

    #[test]
    fn l8_02_owner_refusal_refines_to_save_fail_without_losing_input() {
        for body in [
            b"title:: 0002\n- claim\n".as_slice(),
            b"title:: 0002\ntags:: caf\xe9\n",
        ] {
            let mut d = creating();
            d.host
                .fs
                .external("owner-arrived.md", Some(Arc::from(body)), true);
            d.host
                .fs
                .owners
                .insert(key(2), "pages/owner-arrived.md".into());
            assert!(d.step("flush", &[json!(2)]));
            assert!(d.step("check", &[]));
            assert_eq!(d.owner_failures, 1);
            assert_eq!(label(&d.host.pages[&key(2)].buf), 3);
            assert!(!d.host.pages[&key(2)].clean());
            assert!(!d.host.fs.files.contains_key("graph/0002.md"));
        }
    }

    #[test]
    #[should_panic(expected = "check/job")]
    fn l8_02_planted_spurious_owner_refusal_is_rejected() {
        let mut d = creating();
        d.host.fs.external(
            "owner-unrelated.md",
            Some(Arc::from(b"title:: Other\n".as_slice())),
            true,
        );
        d.host
            .fs
            .owners
            .insert(key(2), "pages/owner-unrelated.md".into());
        assert!(d.step("flush", &[json!(2)]));
        d.step("check", &[]);
    }
}
