//! Private, opt-in unit-cost measurement. Never prints corpus content or names.
use super::io::Phase;
use super::production::ProductionIo;
use super::progress::{Clock, Progress};
use super::*;
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};

pub(super) struct ManualClock(pub(super) Cell<u64>);
impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.0.get()
    }
}

fn send(host: &mut Host<ProductionIo>, page: &str, kind: RequestKind) {
    let request = Request {
        id: host.last_admitted + 1,
        generation: host.generation,
        page: page.into(),
        kind,
    };
    assert_eq!(host.admit(request), Disposition::Applied);
    assert_eq!(host.dequeue(), Disposition::Pending);
    assert_eq!(host.apply_request(), Disposition::Applied);
}

fn pump(progress: &mut Progress<ProductionIo, ManualClock>, drafts: &Path, peak: &mut usize) {
    for _ in 0..100 {
        let result = progress.poll(0);
        let files = fs::read_dir(drafts)
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|ext| ext == "draft")
            })
            .count();
        *peak = (*peak).max(files);
        if result == Disposition::Disabled {
            return;
        }
    }
    panic!("healthy draft disk did not quiesce");
}

fn reset_counts() {
    crate::atomic_file::FILE_SYNCS.with(|count| count.set(0));
    crate::atomic_file::TEMP_WRITES.with(|count| count.set((0, 0)));
    crate::directory_durability::take_synced_directories();
}

fn counts() -> serde_json::Value {
    let file_syncs = crate::atomic_file::FILE_SYNCS.with(Cell::get);
    let (files, bytes) = crate::atomic_file::TEMP_WRITES.with(Cell::get);
    let directories = crate::directory_durability::take_synced_directories().len();
    serde_json::json!({"files_written": files, "bytes_written": bytes,
        "file_fsyncs": file_syncs, "directory_fsyncs": directories, "fsyncs": file_syncs + directories as u64})
}

fn blocks(blocks: &[tine_core::DocBlock]) -> usize {
    blocks
        .iter()
        .map(|block| 1 + self::blocks(&block.children))
        .sum()
}

fn candidates(root: &Path, output: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            candidates(&entry.path(), output);
        } else if entry
            .path()
            .extension()
            .is_some_and(|extension| extension == "md")
        {
            output.push(entry.path());
        }
    }
}

#[test]
#[cfg(test)]
#[ignore = "requires TINE_DRAFT_COST_CORPUS pointing to the private anonymized graph COPY"]
fn measure_anonymized_draft_unit_cost() {
    let corpus =
        PathBuf::from(std::env::var_os("TINE_DRAFT_COST_CORPUS").expect("private corpus copy"));
    // Require the input to be inside this worktree's explicitly named scratch
    // copy, so an accidental original path cannot become a graph-write target.
    let expected = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scratch/step2b/anon-copy")
        .canonicalize()
        .unwrap();
    assert_eq!(corpus.canonicalize().unwrap(), expected);
    let mut files = vec![];
    candidates(&corpus, &mut files);
    files.sort();
    let mut samples = BTreeMap::new();
    for path in files {
        let bytes = fs::read(&path).unwrap();
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        let count = blocks(&tine_core::doc::parse(text).roots);
        if matches!(count, 1 | 60) {
            samples.entry(count).or_insert((path, bytes));
        }
        if samples.len() == 2 {
            break;
        }
    }
    assert_eq!(
        samples.len(),
        2,
        "copy must contain parsed 1-block and 60-block pages"
    );
    for (block_count, (path, base)) in samples {
        let page = path
            .strip_prefix(&corpus)
            .unwrap()
            .components()
            .map(|c| c.as_os_str().to_str().unwrap())
            .collect::<Vec<_>>()
            .join("/");
        let page = page.as_str();
        let app = tempfile::tempdir().unwrap();
        let native = ProductionIo::new(
            &corpus,
            app.path(),
            "cost",
            &corpus.join("logseq/.tine-trash/pages"),
        )
        .unwrap();
        let mut host = Host::new(
            native,
            BTreeMap::from([(page.into(), Arc::new(Mutex::new(())))]),
        );
        send(&mut host, page, RequestKind::Open);
        let mut bytes = base.clone();
        bytes.push(b' ');
        let version = host.pages[page].version;
        send(
            &mut host,
            page,
            RequestKind::Submit {
                bytes: Some(bytes.clone().into()),
                version,
                resolve: None,
            },
        );
        host.fs
            .faults
            .insert(Phase::PageTemp, [std::io::ErrorKind::Other].into());
        assert_eq!(host.start_save(page), Disposition::Pending);
        host.advance_save(0);
        assert!(host.pages[page].risk);
        reset_counts();
        assert_eq!(host.begin_draft(page), Disposition::Pending);
        for _ in 0..20 {
            if host.worker.is_none() {
                break;
            }
            host.advance_draft();
        }
        assert!(host.worker.is_none());
        let first = counts();
        let drafts = app.path().join("drafts-v2/cost");
        let disk_bytes: u64 = fs::read_dir(&drafts)
            .unwrap()
            .map(|entry| entry.unwrap().metadata().unwrap())
            .filter(fs::Metadata::is_file) // not the trash-custody directory
            .map(|metadata| metadata.len())
            .sum();
        assert_eq!(first["files_written"], 1);
        assert_eq!(first["bytes_written"], disk_bytes);
        // A4 cost line: one checksummed marker per deletion, written strictly
        // (temp + fsync + no-replace rename + directory sync) before the move
        // and retired (unlink + directory sync) after publication. Measured on
        // a separate private app-data directory; the corpus copy is untouched.
        let id = uuid::Uuid::new_v4().simple().to_string();
        let file = std::path::Path::new(page)
            .file_name()
            .unwrap()
            .to_str()
            .unwrap();
        let marker = drafts::encode_marker(&drafts::Marker {
            page: page.to_string(),
            payload: crate::atomic_file::prefixed_name(&format!("{id}__"), file),
        });
        let custody_store = tempfile::tempdir().unwrap();
        let mut custody_io = ProductionIo::new(
            &corpus,
            custody_store.path(),
            "cost",
            &corpus.join("logseq/.tine-trash/pages"),
        )
        .unwrap();
        reset_counts();
        custody_io
            .custody_write(&format!("{id}.tcm"), &marker)
            .unwrap();
        let write = counts();
        reset_counts();
        custody_io.custody_retire(&format!("{id}.tcm")).unwrap();
        let retire = counts();
        let custody_cost = serde_json::json!({"marker_bytes": marker.len(),
            "write": write, "retire": retire});
        reset_counts();
        bytes.push(b'x');
        let version = host.pages[page].version;
        send(
            &mut host,
            page,
            RequestKind::Submit {
                bytes: Some(bytes.clone().into()),
                version,
                resolve: None,
            },
        );
        assert_eq!(host.begin_draft(page), Disposition::Pending);
        for _ in 0..30 {
            if host.worker.is_none() {
                break;
            }
            host.advance_draft();
        }
        assert!(host.worker.is_none());
        let refresh = counts();
        assert_eq!(refresh["files_written"], 1);
        assert_eq!(refresh["fsyncs"], 3);
        let mut progress = Progress::new(host, ManualClock(Cell::new(0)));
        progress.host.fs.faults.insert(
            Phase::PageTemp,
            std::iter::repeat_n(std::io::ErrorKind::Other, 1000).collect(),
        );
        reset_counts();
        let mut peak = 1;
        for now in (0..60_000).step_by(100) {
            progress.clock.0.set(now);
            bytes.push(b'x');
            progress.with_host(|host| {
                let version = host.pages[page].version;
                send(
                    host,
                    page,
                    RequestKind::Submit {
                        bytes: Some(bytes.clone().into()),
                        version,
                        resolve: None,
                    },
                );
            });
            pump(&mut progress, &drafts, &mut peak);
        }
        let minute = counts();
        assert!(minute["files_written"].as_u64().unwrap() <= 120);
        assert!(
            fs::read(&path).unwrap() == base,
            "measurement never publishes graph bytes"
        );
        assert_eq!(progress.host.fs.draft_files(true).len(), 1);
        let final_record = &progress.host.logical_drafts()[page];
        assert!(
            bytes.len() - final_record.bytes.as_ref().unwrap().len() < 5,
            "coalescing lag stays below 500 ms of typing"
        );
        eprintln!(
            "draft_cost {}",
            serde_json::json!({"blocks":block_count,"base_bytes":base.len(),
            "buffer_bytes":base.len()+1,"page_key_bytes":page.len(),"draft_bytes":disk_bytes,
            "envelope_bytes":disk_bytes as usize - 2*base.len()-1,"initial_install":first,
            "one_at_risk_refresh_and_retirement": refresh,
            "custody": custody_cost,
            "typing_600_edits_10hz_60000ms":minute,
            "retained_files":progress.host.fs.draft_files(true).len(),"peak_vehicles":peak})
        );
    }
}
