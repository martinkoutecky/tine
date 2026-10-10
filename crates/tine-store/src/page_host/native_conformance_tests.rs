//! Same committed programs and observed-state driver on real files. ModelFs
//! is a physical witness ledger: readable entries and results are independently
//! checked against the native adapter at every phase. Stable entries represent
//! API witnesses, not power-cut evidence. Power and speculative `.fail()` forks
//! stay ModelFs-only, explicitly excluded by the corpus selector below.
use super::*;
use crate::page_host::io::{IoResult, MoveResult, Witness};
use crate::page_host::production::ProductionIo;
use std::fs;
use std::io;
use std::path::PathBuf;

struct NativeFs {
    root: tempfile::TempDir,
    graph: PathBuf,
    app: PathBuf,
    trash: PathBuf,
    native: ProductionIo,
    ledger: ModelFs,
    count: usize,
}

impl NativeFs {
    fn check_readable(&self) {
        for p in 0..self.count {
            let bytes = match fs::read(self.graph.join(key(p))) {
                Ok(bytes) => Some(Arc::<[u8]>::from(bytes)),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => panic!("native graph read: {error}"),
            };
            assert_eq!(
                bytes,
                self.ledger.files.get(&format!("graph/{}", key(p))).cloned()
            );
        }
        let real: BTreeMap<_, _> = fs::read_dir(self.app.join("drafts-v2/native"))
            .unwrap()
            .map(|entry| entry.unwrap())
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".draft"))
            .map(|entry| {
                (
                    entry.file_name().into_string().unwrap(),
                    fs::read(entry.path()).unwrap(),
                )
            })
            .collect();
        let ledger: BTreeMap<_, _> = self.ledger.draft_files(false).into_iter().collect();
        assert_eq!(real, ledger, "readable native draft files");
        assert_eq!(
            self.native
                .draft_files(false)
                .into_iter()
                .collect::<BTreeMap<_, _>>(),
            real
        );
        let mut real_trash: Vec<_> = fs::read_dir(&self.trash)
            .unwrap()
            .map(|entry| fs::read(entry.unwrap().path()).unwrap())
            .collect();
        let mut ledger_trash: Vec<_> = self
            .ledger
            .files
            .iter()
            .filter(|(key, _)| key.starts_with("trash/"))
            .map(|(_, bytes)| bytes.to_vec())
            .collect();
        real_trash.sort();
        ledger_trash.sort();
        assert_eq!(real_trash, ledger_trash, "native trash bytes");
        let real_markers: BTreeMap<_, _> =
            fs::read_dir(self.app.join("drafts-v2/native/trash-custody"))
                .into_iter()
                .flatten()
                .map(|entry| entry.unwrap())
                .map(|entry| {
                    (
                        entry.file_name().into_string().unwrap(),
                        fs::read(entry.path()).unwrap(),
                    )
                })
                .collect();
        let ledger_markers: BTreeMap<_, _> = self
            .ledger
            .files
            .iter()
            .filter_map(|(key, bytes)| {
                Some((
                    key.strip_prefix("draft/trash-custody/")?.to_string(),
                    bytes.to_vec(),
                ))
            })
            .collect();
        assert_eq!(real_markers, ledger_markers, "native custody markers");
        assert!(self.root.path().exists());
    }

    fn phase<T: std::fmt::Debug + PartialEq>(
        &mut self,
        phase: Phase,
        model: impl FnOnce(&mut ModelFs) -> IoResult<T>,
        native: impl FnOnce(&mut ProductionIo) -> IoResult<T>,
    ) -> IoResult<T> {
        if let Some(fault) = self
            .ledger
            .faults
            .get(&phase)
            .and_then(|faults| faults.front())
        {
            assert!(
                matches!(fault, Fault::Before),
                "native corpus only injects pre-phase errors"
            );
            self.native
                .faults
                .insert(phase, [io::ErrorKind::Other].into());
        } else if self.ledger.weak_graph && matches!(phase, Phase::PageSync | Phase::TrashSync) {
            crate::directory_durability::SYNC_ERROR
                .with(|error| error.set(Some(io::ErrorKind::InvalidInput)));
        }
        let expected = model(&mut self.ledger);
        let actual = native(&mut self.native);
        // The one-shot seam must not leak into a later phase that did not use it.
        crate::directory_durability::SYNC_ERROR.with(|error| error.set(None));
        assert_eq!(
            actual.as_ref().map_err(|e| e.kind),
            expected.as_ref().map_err(|e| e.kind),
            "{phase:?}"
        );
        self.check_readable();
        actual
    }
}

impl ConformanceIo for NativeFs {
    fn native_create_refinement() -> bool {
        true
    }
    fn fresh(profile: &str, count: usize) -> Self {
        let root = match std::env::var_os("TINE_HOST_FS_ROOT") {
            Some(base) => tempfile::tempdir_in(base).unwrap(),
            None => tempfile::tempdir().unwrap(),
        };
        let graph = root.path().join("graph");
        let app = root.path().join("app");
        let trash = graph.join("logseq/.tine-trash/pages");
        fs::create_dir_all(&graph).unwrap();
        fs::create_dir_all(&app).unwrap();
        fs::create_dir_all(&trash).unwrap();
        let ledger = ModelFs::fresh(profile, count);
        for (key, bytes) in &ledger.files {
            if let Some(page) = key.strip_prefix("graph/") {
                fs::write(graph.join(page), bytes).unwrap();
            }
        }
        let native = ProductionIo::new(&graph, &app, "native", &trash).unwrap();
        Self {
            root,
            graph,
            app,
            trash,
            native,
            ledger,
            count,
        }
    }
    fn physical(&self) -> &ModelFs {
        &self.ledger
    }
    fn physical_mut(&mut self) -> &mut ModelFs {
        &mut self.ledger
    }
    fn external(&mut self, page: &str, bytes: Text, durable: bool) {
        match &bytes {
            Some(bytes) => {
                if durable {
                    crate::atomic_file::atomic_write_with_check(
                        &self.graph.join(page),
                        bytes,
                        || Ok(()),
                        || {},
                        || {},
                        || {},
                    )
                    .unwrap();
                } else {
                    fs::write(self.graph.join(page), bytes).unwrap();
                }
            }
            None => {
                if self.graph.join(page).exists() {
                    fs::remove_file(self.graph.join(page)).unwrap();
                }
                if durable {
                    crate::directory_durability::sync_directory_entry(&self.graph).unwrap();
                }
            }
        }
        self.ledger.external(page, bytes, durable);
        self.check_readable();
    }
    fn crash(&mut self) {
        self.ledger.crash();
        self.native = ProductionIo::new(&self.graph, &self.app, "native", &self.trash).unwrap();
        self.check_readable();
    }
    fn power(&mut self, _keep: &BTreeSet<String>, _keep_drafts: bool) {
        panic!("power cuts are excluded from the native corpus");
    }
    fn fork(&self) -> Option<Self> {
        None
    }
}

impl HostIo for NativeFs {
    fn spelling(&self, key: &str) -> String {
        self.native.spelling(key)
    }
    fn spell(&mut self, key: &str, spelling: &str) {
        self.ledger.spell(key, spelling);
        self.native.spell(key, spelling);
    }
    fn page_twin(&mut self, page: &str) -> IoResult<Option<String>> {
        self.native.page_twin(page)
    }
    fn graph_launch(&mut self, pages: &BTreeSet<String>) {
        self.native.graph_launch(pages);
    }
    fn page_finish(&mut self, page: &str) {
        self.ledger.page_finish(page);
        self.native.page_finish(page);
        self.check_readable();
    }
    fn read_page(&mut self, page: &str) -> IoResult<Text> {
        self.phase(
            Phase::Read,
            |fs| fs.read_page(page),
            |fs| fs.read_page(page),
        )
    }
    fn page_temp(&mut self, page: &str, bytes: &Text) -> IoResult<()> {
        self.phase(
            Phase::PageTemp,
            |fs| fs.page_temp(page, bytes),
            |fs| fs.page_temp(page, bytes),
        )
    }
    fn page_rename(&mut self, page: &str) -> IoResult<()> {
        self.phase(
            Phase::PageRename,
            |fs| fs.page_rename(page),
            |fs| fs.page_rename(page),
        )
    }
    fn page_sync(&mut self, page: &str) -> IoResult<Witness> {
        self.phase(
            Phase::PageSync,
            |fs| fs.page_sync(page),
            |fs| fs.page_sync(page),
        )
    }
    fn trash_move(&mut self, page: &str, name: &str) -> MoveResult {
        let mut removed = None;
        let result = self.phase(
            Phase::TrashMove,
            |fs| {
                let moved = fs.trash_move(page, name);
                Ok((moved.removed, moved.result))
            },
            |fs| {
                let moved = fs.trash_move(page, name);
                removed = moved.removed.clone();
                Ok((moved.removed, moved.result))
            },
        );
        MoveResult {
            removed,
            result: result.unwrap().1,
        }
    }
    fn trash_sync(&mut self, page: &str, payload: &str) -> IoResult<Witness> {
        self.phase(
            Phase::TrashSync,
            |fs| fs.trash_sync(page, payload),
            |fs| fs.trash_sync(page, payload),
        )
    }
    fn custody_write(&mut self, name: &str, bytes: &[u8]) -> IoResult<()> {
        self.phase(
            Phase::CustodyWrite,
            |fs| fs.custody_write(name, bytes),
            |fs| fs.custody_write(name, bytes),
        )
    }
    fn custody_retire(&mut self, name: &str) -> IoResult<()> {
        self.phase(
            Phase::CustodyRetire,
            |fs| fs.custody_retire(name),
            |fs| fs.custody_retire(name),
        )
    }
    fn custody_markers(&mut self) -> Result<Vec<(String, Vec<u8>)>, String> {
        let expected = self.ledger.custody_markers();
        let actual = self.native.custody_markers();
        assert_eq!(
            actual.as_ref().ok(),
            expected.as_ref().ok(),
            "custody markers"
        );
        actual
    }
    fn draft_files(&self, durable: bool) -> Vec<(String, Vec<u8>)> {
        self.native.draft_files(durable)
    }
    fn draft_changes(&mut self) -> Vec<(String, Option<Vec<u8>>)> {
        // The host's index is checked against the durable census itself
        // (`Host::logical_drafts` under test); the ledger only keeps pace.
        self.ledger.draft_changes();
        self.native.draft_changes()
    }
    fn draft_temp(&mut self, name: &str, bytes: &[u8]) -> IoResult<()> {
        self.phase(
            Phase::DraftTemp,
            |fs| fs.draft_temp(name, bytes),
            |fs| fs.draft_temp(name, bytes),
        )
    }
    fn draft_rename(&mut self, name: &str) -> IoResult<()> {
        self.phase(
            Phase::DraftRename,
            |fs| fs.draft_rename(name),
            |fs| fs.draft_rename(name),
        )
    }
    fn draft_unlink(&mut self, name: &str) -> IoResult<()> {
        self.phase(
            Phase::DraftUnlink,
            |fs| fs.draft_unlink(name),
            |fs| fs.draft_unlink(name),
        )
    }
    fn draft_sync(&mut self) -> IoResult<Witness> {
        let result = self.phase(Phase::DraftSync, |fs| fs.draft_sync(), |fs| fs.draft_sync());
        assert_eq!(
            self.native
                .draft_files(true)
                .into_iter()
                .collect::<BTreeMap<_, _>>(),
            self.ledger.draft_files(true).into_iter().collect()
        );
        result
    }
    fn quarantine(&mut self, name: &str) -> IoResult<()> {
        self.phase(
            Phase::Quarantine,
            |fs| fs.quarantine(name),
            |fs| fs.quarantine(name),
        )
    }
}

fn excluded(program: &Value) -> bool {
    if let Some(values) = program.as_array() {
        if values.first().and_then(Value::as_str) == Some("fail") {
            return true;
        }
        if values.first().and_then(Value::as_str) == Some("action")
            && values
                .get(1)
                .and_then(Value::as_str)
                .is_some_and(|name| name.starts_with("power"))
        {
            return true;
        }
        values.iter().any(excluded)
    } else {
        false
    }
}

#[test]
#[cfg(test)]
fn native_committed_scenarios_in_four_profiles() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/s2/scenarios.json")).unwrap();
    assert_eq!(fixture["model_sha256"], Oracle::model_sha());
    let mut ran = vec![];
    let mut skipped = vec![];
    let mut refinements = vec![];
    let mut comparisons = 0;
    for (name, program) in fixture["scenarios"].as_object().unwrap() {
        if excluded(program) {
            skipped.push(name.as_str());
            continue;
        }
        for oracle in fixture["oracles"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|o| o["mutant"] == "none")
        {
            let profile = oracle["profile"].as_str().unwrap();
            let mut driver = Driver::<NativeFs>::new_with_io(profile, 3);
            let outcome = driver.program(program.as_array().unwrap());
            if outcome == Err("native-create-refinement") {
                refinements.push(format!("{profile}/{name}"));
                continue;
            }
            assert_eq!(
                outcome.map_or_else(|e| e, |_| "pass"),
                oracle["outcomes"][name],
                "native {profile}/{name}"
            );
            comparisons += 1;
        }
        ran.push(name.as_str());
    }
    assert!(!ran.is_empty());
    eprintln!(
        "native scenarios: {} selected, {comparisons} profile outcomes; ran={ran:?}; ModelFs-only={skipped:?}; separate no-replace refinement={refinements:?}", ran.len()
    );
}
