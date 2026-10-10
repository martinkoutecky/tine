//! Retirement of a page host no window owns any more (plan v3 §3, REVIEW-3b
//! S2). A released binding whose host still runs leaves the registry and
//! enters this map in one registry-locked step, so no opener sees its root
//! unowned until the host has stopped and its Store has closed. A later open
//! of the same root adopts the host instead (`adopt`); otherwise the
//! retirement thread stops it with `PageHost::orphan_stop` (the model's
//! `windowCrash`, once per abandoned owner) and closes the Store.
//!
//! Lock order: registry, then this map; a slot's host lock, then this map.
//! The map is always taken last and held only briefly.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::state::{GraphSlot, PageHostSlot};

#[derive(Clone, Default)]
pub(crate) struct HostRetirement(Arc<Inner>);

#[derive(Default)]
struct Inner {
    retiring: Mutex<HashMap<PathBuf, Arc<GraphSlot>>>,
    idle: Condvar,
}

impl HostRetirement {
    /// Take a released slot. One with no host comes straight back, for the
    /// caller to drop outside the registry lock (its Store close takes
    /// ~200 ms); one whose host runs is kept until its host stops. Called
    /// under the registry write lock. A slot whose host lock is held (a
    /// restore in flight) is kept as well: the retirement thread waits for
    /// the restore's outcome.
    pub(crate) fn retire(&self, slot: Arc<GraphSlot>) -> Option<Arc<GraphSlot>> {
        if matches!(slot.host.try_read().as_deref(), Ok(PageHostSlot::Off)) {
            return Some(slot);
        }
        let previous = self
            .0
            .retiring
            .lock()
            .unwrap()
            .insert(slot.root_key.clone(), slot.clone());
        debug_assert!(previous.is_none(), "a root is bound or retiring, not both");
        let this = self.clone();
        let spawned = std::thread::Builder::new()
            .name("tine-host-retirement".into())
            .spawn(move || this.stop_and_close(slot));
        if spawned.is_err() {
            crate::debug::diag("page-host-retirement-spawn-failed");
        }
        None
    }

    /// Stop `slot`'s host and close its Store, unless an open adopts it
    /// first. Each pass takes the host lock briefly, so an adoption or a
    /// restore can come in between passes.
    fn stop_and_close(&self, slot: Arc<GraphSlot>) {
        let mut backoff = Duration::from_secs(1);
        loop {
            let mut host = slot.host.write().unwrap_or_else(|e| e.into_inner());
            if !self.claims(&slot) {
                return;
            }
            let pause = match &*host {
                PageHostSlot::Off => None,
                // Revoked only by an adoption, which `claims` saw.
                PageHostSlot::Revoked => return,
                PageHostSlot::Running(running) => match running.orphan_stop() {
                    tine_store::StopState::Ready => {
                        let PageHostSlot::Running(running) = std::mem::take(&mut *host) else {
                            unreachable!()
                        };
                        match running.stop_finish() {
                            Ok(_stopped) => None,
                            Err(back) => {
                                *host = PageHostSlot::Running(back);
                                Some(Duration::from_millis(100))
                            }
                        }
                    }
                    tine_store::StopState::Waiting => Some(Duration::from_millis(100)),
                    tine_store::StopState::Aborted(_) => {
                        // A draft failure (or a save failure in a restore
                        // the owner began): the host keeps the pages and
                        // stays alive, and the stop is retried. The
                        // stuck-graph window is P2b (STEP3-DESIGN B-QA).
                        running.stop_abort();
                        crate::debug::diag("page-host-retirement-aborted");
                        let pause = backoff;
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                        Some(pause)
                    }
                },
            };
            let Some(pause) = pause else {
                // Still under the host lock: an adoption either took the
                // host before this pass or finds the slot gone.
                slot.store.close();
                self.0.retiring.lock().unwrap().remove(&slot.root_key);
                self.0.idle.notify_all();
                return;
            };
            drop(host);
            std::thread::sleep(pause);
        }
    }

    fn claims(&self, slot: &Arc<GraphSlot>) -> bool {
        let retiring = self.0.retiring.lock().unwrap();
        retiring
            .get(&slot.root_key)
            .is_some_and(|kept| Arc::ptr_eq(kept, slot))
    }

    /// Take over the retiring binding of `root`, if any: its Store and its
    /// running host. A stop the retirement began ends when the adopting
    /// window reloads (`PageHost::window_reloaded`, the model's
    /// `windowCrash`), as every window's first command. The old slot is
    /// revoked and no longer closes the Store. Called outside the registry lock; the
    /// caller binds a fresh slot around them.
    pub(crate) fn adopt(
        &self,
        root: &std::path::Path,
    ) -> Option<(Arc<tine_store::Store>, PageHostSlot)> {
        let slot = self.0.retiring.lock().unwrap().get(root).cloned()?;
        let mut host = slot.host.write().unwrap_or_else(|e| e.into_inner());
        {
            let mut retiring = self.0.retiring.lock().unwrap();
            if !retiring
                .get(root)
                .is_some_and(|kept| Arc::ptr_eq(kept, &slot))
            {
                return None;
            }
            retiring.remove(root);
        }
        self.0.idle.notify_all();
        // Revoked under the gate (REVIEW-3b-P1 B1): a writer, transaction or
        // restore that still holds the old slot gets the stale-binding
        // outcome; one already under the gate finished before this write.
        let taken = std::mem::replace(&mut *host, PageHostSlot::Revoked);
        Some((slot.store.clone(), taken))
    }

    /// A retiring root that overlaps `root` (one contains the other), or
    /// with `same` also `root` itself.
    pub(crate) fn overlapping(&self, root: &std::path::Path, same: bool) -> Option<PathBuf> {
        overlap(&self.0.retiring.lock().unwrap(), root, same)
    }

    /// Wait up to `bound` until no other retiring root overlaps `root`
    /// (`root` itself is adopted, not waited for). `waiting` runs once,
    /// with the root still saving, when the wait has to begin. The root
    /// still retiring past the bound comes back.
    pub(crate) fn wait_overlapping(
        &self,
        root: &std::path::Path,
        bound: Duration,
        waiting: impl FnOnce(&std::path::Path),
    ) -> Result<(), PathBuf> {
        let deadline = Instant::now() + bound;
        let mut waiting = Some(waiting);
        let mut retiring = self.0.retiring.lock().unwrap();
        while let Some(saving) = overlap(&retiring, root, false) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(saving);
            }
            if let Some(waiting) = waiting.take() {
                drop(retiring);
                waiting(&saving);
                retiring = self.0.retiring.lock().unwrap();
            }
            retiring = self.0.idle.wait_timeout(retiring, left).unwrap().0;
        }
        Ok(())
    }

    /// Whether no host is retiring.
    pub(crate) fn is_idle(&self) -> bool {
        self.0.retiring.lock().unwrap().is_empty()
    }

    /// Wait up to `bound` for every retiring host to stop; the roots still
    /// retiring otherwise.
    pub(crate) fn wait_idle(&self, bound: Duration) -> Result<(), Vec<PathBuf>> {
        let deadline = Instant::now() + bound;
        let mut retiring = self.0.retiring.lock().unwrap();
        loop {
            if retiring.is_empty() {
                return Ok(());
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(retiring.keys().cloned().collect());
            }
            retiring = self.0.idle.wait_timeout(retiring, left).unwrap().0;
        }
    }
}

fn overlap(
    retiring: &HashMap<PathBuf, Arc<GraphSlot>>,
    root: &std::path::Path,
    same: bool,
) -> Option<PathBuf> {
    retiring
        .keys()
        .find(|kept| (same || kept.as_path() != root) && crate::state::roots_overlap(kept, root))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{
        exit_when_unowned, wait_for_retiring_overlaps, AppState, GraphRegistry, STALE_BINDING,
    };
    use std::sync::atomic::AtomicBool;
    use std::sync::{mpsc, Barrier, Mutex, RwLock};
    use tine_store::{EditKind, Input, PageHost, PageId, PageMail, Store};

    struct Fixture {
        dir: PathBuf,
        root: PathBuf,
        app_data: PathBuf,
        store: Arc<Store>,
    }

    impl Fixture {
        fn new(tag: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("tine-retire-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let graph = dir.join("graph");
            std::fs::create_dir_all(graph.join("pages")).unwrap();
            std::fs::write(graph.join("pages/a.md"), "- one\n").unwrap();
            let app_data = dir.join("app");
            std::fs::create_dir_all(&app_data).unwrap();
            let store = Arc::new(Store::open(&graph, Default::default()).unwrap().0);
            store.whole_graph().unwrap();
            let root = Store::canonical_root(&graph).unwrap();
            Self {
                dir,
                root,
                app_data,
                store,
            }
        }

        /// A slot bound to `window` in `registry`, running a host.
        fn bind(&self, registry: &mut GraphRegistry, window: &str) -> Arc<GraphSlot> {
            let mut slot = GraphSlot::new(self.store.clone(), self.root.clone());
            let host = PageHost::start_for_tests(&self.store, &self.app_data).unwrap();
            *slot.host.get_mut().unwrap() = PageHostSlot::Running(host);
            let slot = Arc::new(slot);
            assert!(registry
                .bind(window.into(), slot.clone())
                .unwrap()
                .is_none());
            slot
        }

        fn disk(&self) -> String {
            std::fs::read_to_string(self.root.join("pages/a.md")).unwrap()
        }

        fn closed(&self) -> bool {
            matches!(
                self.store.is_graph_ready(),
                Err(tine_store::LoadError::Closed)
            )
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// A window's view of `slot`'s host: its session, page a's key and
    /// version, and its next request id.
    struct Window {
        session: u64,
        key: String,
        version: u64,
        next: u64,
    }

    fn json<T: serde::Serialize>(value: T) -> serde_json::Value {
        serde_json::to_value(value).unwrap()
    }

    fn running<T>(slot: &GraphSlot, work: impl FnOnce(&PageHost) -> T) -> T {
        match &*slot.host.read().unwrap() {
            PageHostSlot::Running(host) => work(host),
            _ => panic!("no host"),
        }
    }

    /// A window (re)loads over `slot` and opens page a.
    fn window(slot: &GraphSlot) -> Window {
        let (sender, mail) = mpsc::channel::<PageMail>();
        running(slot, |host| {
            host.retarget(move |mail| {
                let _ = sender.send(mail);
            });
            let reloaded = json(host.window_reloaded());
            let session = reloaded["session"].as_u64().unwrap();
            let id = reloaded["nextId"].as_u64().unwrap();
            let opened = host.open(session, id, &PageId::from("pages/a.md"), "a");
            let key = json(opened.unwrap())["key"].as_str().unwrap().to_owned();
            loop {
                let mail = json(mail.recv_timeout(Duration::from_secs(20)).unwrap());
                if mail["answer"]["id"].as_u64() == Some(id) {
                    let version = mail["page"]["version"].as_u64().unwrap();
                    return Window {
                        session,
                        key,
                        version,
                        next: id + 1,
                    };
                }
            }
        })
    }

    fn submit(
        slot: &GraphSlot,
        window: &Window,
        body: &str,
    ) -> Result<(), tine_store::PageRefusal> {
        running(slot, |host| {
            let dto = host_dto(slot, body);
            let kinds = [EditKind::SaveBlock];
            host.submit(
                window.session,
                window.next,
                &window.key,
                &dto,
                window.version,
                None,
                &kinds,
            )
        })
    }

    fn host_dto(slot: &GraphSlot, body: &str) -> tine_core::model::PageDto {
        let page = PageId::from("pages/a.md");
        slot.store.page_of(&page, body.as_bytes()).unwrap().doc
    }

    fn hold(slot: &GraphSlot) -> tine_store::Reservation {
        running(slot, |host| {
            host.reserve(|| vec![PageId::from("pages/a.md")], Input::Refuse)
                .unwrap()
        })
    }

    /// Plan v3 §3 (B2), S2: a window destroyed while its host holds an
    /// admitted request hands the binding to its retirement, which keeps
    /// the root owned (Waiting past a bound), applies and saves the request,
    /// and only then closes the Store. A late user of the old Arc then
    /// finds no host and a closed Store (the existing closed-store error).
    #[test]
    fn a_released_host_saves_its_admitted_input_before_its_store_closes() {
        let f = Fixture::new("admitted");
        let mut registry = GraphRegistry::default();
        let slot = f.bind(&mut registry, "graph-1");
        let window = window(&slot);
        let reservation = hold(&slot);
        submit(&slot, &window, "- two\n").unwrap();
        assert!(
            registry.remove("graph-1").is_none(),
            "the retirement keeps a running host"
        );
        let retirement = registry.retirement.clone();
        assert!(!retirement.is_idle());
        assert_eq!(
            retirement.wait_idle(Duration::from_millis(300)),
            Err(vec![f.root.clone()]),
            "Waiting past the bound: the root stays retiring"
        );
        assert!(!f.closed());
        drop(reservation);
        assert_eq!(retirement.wait_idle(Duration::from_secs(20)), Ok(()));
        assert_eq!(f.disk(), "- two\n", "the admitted input is saved first");
        assert!(f.closed(), "then the Store closes");
        assert!(matches!(*slot.host_slot().unwrap(), PageHostSlot::Off));
    }

    /// Plan v3 §3 (B2): a stop that cannot keep custody (neither the save
    /// nor the draft can be written) is aborted, the host stays alive with
    /// the input, and the stop is retried until it completes.
    #[cfg(unix)]
    #[test]
    fn an_aborted_retirement_keeps_the_host_alive_and_retries() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new("aborted");
        let mut registry = GraphRegistry::default();
        let slot = f.bind(&mut registry, "graph-1");
        let window = window(&slot);
        let drafts = f.app_data.join("drafts-v2/test-graph");
        let pages = f.root.join("pages");
        let mode = |dir: &std::path::Path, mode| {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode)).unwrap()
        };
        mode(&pages, 0o555);
        mode(&drafts, 0o555);
        submit(&slot, &window, "- two\n").unwrap();
        assert!(registry.remove("graph-1").is_none());
        let retirement = registry.retirement.clone();
        assert!(retirement.wait_idle(Duration::from_millis(2500)).is_err());
        assert!(
            slot.host_slot().unwrap().running().is_some(),
            "the host stays alive"
        );
        assert!(!f.closed());
        mode(&pages, 0o755);
        mode(&drafts, 0o755);
        assert_eq!(retirement.wait_idle(Duration::from_secs(30)), Ok(()));
        assert_eq!(f.disk(), "- two\n");
        assert!(f.closed());
    }

    /// Plan v3 §3, S2: reopening a retiring root adopts its Store and host
    /// (one driver) under a fresh binding; the old binding's late release
    /// and its last Arc touch neither, and the old session is not admitted.
    #[test]
    fn a_reopened_root_adopts_its_retiring_host_under_a_fresh_binding() {
        let f = Fixture::new("adopt");
        let mut registry = GraphRegistry::default();
        let old = f.bind(&mut registry, "graph-1");
        let stale = window(&old);
        let reservation = hold(&old);
        assert!(registry.remove("graph-1").is_none());
        let retirement = registry.retirement.clone();
        let (store, host) = retirement.adopt(&f.root).expect("the retiring host");
        assert!(Arc::ptr_eq(&store, &f.store), "one Store");
        assert!(
            matches!(host, PageHostSlot::Running(_)),
            "the same host: one driver"
        );
        assert!(retirement.is_idle());
        assert!(retirement.adopt(&f.root).is_none(), "adopted once");
        let mut fresh = GraphSlot::new(store, f.root.clone());
        *fresh.host.get_mut().unwrap() = host;
        let fresh = Arc::new(fresh);
        assert_ne!(fresh.binding_generation, old.binding_generation);
        assert!(registry
            .bind("graph-2".into(), fresh.clone())
            .unwrap()
            .is_none());
        assert!(registry
            .release_binding("graph-1", old.binding_generation)
            .is_none());
        assert!(registry
            .slot("graph-2")
            .is_some_and(|slot| Arc::ptr_eq(&slot, &fresh)));
        drop(old);
        std::thread::sleep(Duration::from_millis(400));
        assert!(
            !f.closed(),
            "neither the handed-over slot nor its retirement closes the Store"
        );
        drop(reservation);
        let window = window(&fresh);
        assert!(
            submit(&fresh, &stale, "- stale\n").is_err(),
            "S2: the old owner's session is not admitted"
        );
        submit(&fresh, &window, "- adopted\n").unwrap();
        assert!(registry.remove("graph-2").is_none());
        assert_eq!(retirement.wait_idle(Duration::from_secs(20)), Ok(()));
        assert_eq!(f.disk(), "- adopted\n");
        assert!(f.closed());
    }

    /// An adoption over a stop the retirement already began: the adopting
    /// window's reload (`window_crash`) aborts it, and its input is admitted.
    #[test]
    fn an_adopted_host_admits_again_after_a_begun_stop() {
        let f = Fixture::new("adopt-stop");
        let mut registry = GraphRegistry::default();
        let old = f.bind(&mut registry, "graph-1");
        assert_eq!(
            running(&old, PageHost::orphan_stop),
            tine_store::StopState::Ready
        );
        // Retiring, as `retire` leaves it, with no thread to finish it.
        let retirement = registry.retirement.clone();
        retirement
            .0
            .retiring
            .lock()
            .unwrap()
            .insert(f.root.clone(), old.clone());
        let (store, host) = retirement.adopt(&f.root).unwrap();
        let mut fresh = GraphSlot::new(store, f.root.clone());
        *fresh.host.get_mut().unwrap() = host;
        let window = window(&fresh);
        assert_eq!(submit(&fresh, &window, "- adopted\n"), Ok(()));
        *fresh.host.write().unwrap() = PageHostSlot::Off;
    }

    /// The registry hands a host-less slot straight back, for the caller to
    /// drop outside the lock (today's path).
    #[test]
    fn a_slot_without_a_host_is_returned_for_the_caller_to_drop() {
        let f = Fixture::new("off");
        let mut registry = GraphRegistry::default();
        let slot = Arc::new(GraphSlot::new(f.store.clone(), f.root.clone()));
        registry.bind("graph-1".into(), slot.clone()).unwrap();
        let released = registry.remove("graph-1").expect("returned");
        assert!(Arc::ptr_eq(&released, &slot));
        assert!(registry.retirement.is_idle());
    }

    /// A retiring host whose page input is admitted but held: it cannot stop
    /// until `reservation` drops. Bound as `graph-1` in `graphs`, then
    /// released.
    fn retiring(f: &Fixture, graphs: &RwLock<GraphRegistry>) -> tine_store::Reservation {
        let slot = f.bind(&mut graphs.write().unwrap(), "graph-1");
        let window = window(&slot);
        let reservation = hold(&slot);
        submit(&slot, &window, "- two\n").unwrap();
        assert!(graphs.write().unwrap().remove("graph-1").is_none());
        reservation
    }

    /// Manager rule after P1: an open of a root overlapping a retiring
    /// graph waits for it to save and close, says so once, and holds no
    /// registry lock meanwhile (other windows' commands and closes go on).
    #[test]
    fn an_overlapping_open_waits_outside_the_registry_lock_until_the_retirement_ends() {
        let f = Fixture::new("overlap-wait");
        let graphs = RwLock::new(GraphRegistry::default());
        let reservation = retiring(&f, &graphs);
        let inner = f.root.join("pages");
        let (began, waiting) = mpsc::channel();
        std::thread::scope(|scope| {
            let open = scope.spawn(|| {
                wait_for_retiring_overlaps(&graphs, &inner, Duration::from_secs(20), |saving| {
                    began.send(saving.to_path_buf()).unwrap()
                })
            });
            assert_eq!(
                waiting.recv_timeout(Duration::from_secs(20)).unwrap(),
                f.root
            );
            let deadline = Instant::now() + Duration::from_secs(5);
            while graphs.try_write().is_err() {
                assert!(
                    Instant::now() < deadline,
                    "the open waits holding the registry lock"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(!open.is_finished(), "the open waits for the retirement");
            drop(reservation);
            assert_eq!(open.join().unwrap(), Ok(()));
        });
        assert_eq!(f.disk(), "- two\n");
        assert!(f.closed(), "the open went on only once the Store closed");
    }

    /// Past the bound the open is refused, naming the graph still saving;
    /// an open that raced past the wait is refused by `bind`, which counts
    /// a retiring root as owned. The same root is adopted, never waited for.
    #[test]
    fn a_stuck_retirement_refuses_an_overlapping_open_naming_the_saving_graph() {
        let f = Fixture::new("overlap-stuck");
        let late = Fixture::new("overlap-late");
        let graphs = RwLock::new(GraphRegistry::default());
        let reservation = retiring(&f, &graphs);
        let inner = f.root.join("pages");
        let saving = f.root.display().to_string();
        let mut told = None;
        let error =
            wait_for_retiring_overlaps(&graphs, &inner, Duration::from_millis(300), |root| {
                told = Some(root.to_path_buf())
            })
            .unwrap_err();
        assert_eq!(told.as_deref(), Some(f.root.as_path()));
        assert!(
            error.contains(&saving) && error.contains("still saving"),
            "{error}"
        );
        let slot = Arc::new(GraphSlot::new(late.store.clone(), inner));
        let Err(error) = graphs.write().unwrap().bind("graph-2".into(), slot) else {
            panic!("a retiring root is owned");
        };
        assert!(
            error.contains(&saving) && error.contains("still saving"),
            "{error}"
        );
        assert_eq!(
            wait_for_retiring_overlaps(&graphs, &f.root, Duration::ZERO, |_| {
                panic!("the same root is adopted, not waited for")
            }),
            Ok(())
        );
        assert!(!f.closed());
        drop(reservation);
        let retirement = graphs.read().unwrap().retirement.clone();
        retirement.wait_idle(Duration::from_secs(20)).unwrap();
        assert!(f.closed());
    }

    /// Old-engine conflict resolution of page a to `body`, the census
    /// writer's shape (`commands/concord.rs`), on `slot` as a worker that
    /// captured it finds it.
    fn late_writer(slot: &GraphSlot, body: &str) -> Result<(), String> {
        let rev = String::from(slot.store.page(&PageId::from("pages/a.md")).unwrap().rev);
        tine_graph_features::live_conflict::resolve_live_conflict(
            &slot.store,
            slot.host_slot()?.running(),
            "pages/a.md",
            &host_dto(slot, body),
            &rev,
            None,
            &[],
            &Default::default(),
            "union",
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
    }

    /// REVIEW-3b-P1 B1: a retained writer that captured the old slot before
    /// its root was adopted gets the stale-binding outcome under the gate.
    /// It never takes the no-host arm past the adopted host's reservation.
    #[test]
    fn review_p1_late_old_slot_writer_cannot_bypass_adopted_reservation() {
        let f = Fixture::new("late-writer");
        let mut registry = GraphRegistry::default();
        let old = f.bind(&mut registry, "graph-1");
        let reservation = hold(&old);
        assert!(registry.remove("graph-1").is_none());
        let retirement = registry.retirement.clone();
        let (store, host) = retirement.adopt(&f.root).unwrap();
        let mut fresh = GraphSlot::new(store, f.root.clone());
        *fresh.host.get_mut().unwrap() = host;
        let fresh = Arc::new(fresh);
        assert!(registry
            .bind("graph-2".into(), fresh.clone())
            .unwrap()
            .is_none());
        assert_eq!(
            late_writer(&old, "- stale queued command\n"),
            Err(STALE_BINDING.to_owned())
        );
        assert_eq!(f.disk(), "- one\n", "the adopted reservation holds");
        drop(reservation);
        assert_eq!(
            late_writer(&old, "- stale queued command\n"),
            Err(STALE_BINDING.to_owned()),
            "revoked for good, not only while the page is reserved"
        );
        assert!(fresh.host_slot().unwrap().running().is_some());
        assert!(registry.remove("graph-2").is_none());
        assert_eq!(retirement.wait_idle(Duration::from_secs(20)), Ok(()));
        assert_eq!(f.disk(), "- one\n");
        assert!(f.closed());
    }

    /// B1: a writer already under the gate keeps custody. The adoption waits
    /// for it, and only then revokes the slot for writers still to come.
    #[test]
    fn a_writer_inside_the_gate_finishes_before_its_slot_is_adopted() {
        let f = Fixture::new("inside-gate");
        let mut old = GraphSlot::new(f.store.clone(), f.root.clone());
        let host = PageHost::start_for_tests(&f.store, &f.app_data).unwrap();
        *old.host.get_mut().unwrap() = PageHostSlot::Running(host);
        let old = Arc::new(old);
        // Retiring, as `retire` leaves it, with no thread to finish it.
        let retirement = HostRetirement::default();
        retirement
            .0
            .retiring
            .lock()
            .unwrap()
            .insert(f.root.clone(), old.clone());
        let (entered, release) = (Barrier::new(2), Barrier::new(2));
        let adopted = std::thread::scope(|scope| {
            let writer = scope.spawn(|| {
                let gate = old.host_slot().unwrap();
                entered.wait();
                release.wait();
                let rev = String::from(f.store.page(&PageId::from("pages/a.md")).unwrap().rev);
                tine_graph_features::live_conflict::resolve_live_conflict(
                    &old.store,
                    gate.running(),
                    "pages/a.md",
                    &host_dto(&old, "- inside\n"),
                    &rev,
                    None,
                    &[],
                    &Default::default(),
                    "union",
                )
                .map(|_| ())
                .map_err(|error| error.to_string())
            });
            entered.wait();
            let adopt = scope.spawn(|| retirement.adopt(&f.root));
            std::thread::sleep(Duration::from_millis(200));
            assert!(!adopt.is_finished(), "the adoption waits for the writer");
            release.wait();
            assert_eq!(writer.join().unwrap(), Ok(()));
            adopt.join().unwrap()
        });
        assert_eq!(f.disk(), "- inside\n", "the writer under the gate finished");
        let (_, host) = adopted.expect("then the adoption");
        assert!(matches!(host, PageHostSlot::Running(_)));
        assert_eq!(late_writer(&old, "- late\n"), Err(STALE_BINDING.to_owned()));
        drop(host);
    }

    fn app_state() -> AppState {
        AppState {
            graphs: RwLock::new(GraphRegistry::default()),
            graph_load: Mutex::new(()),
            last_focused: Mutex::new(None),
            capture_graph: Mutex::new(Default::default()),
            #[cfg(desktop)]
            next_window: std::sync::atomic::AtomicU64::new(1),
        }
    }

    /// The exit left pending by the last window's close (as `lib.rs`'s
    /// `Destroyed` hook leaves it), run while `open` runs; whether it
    /// exited, and whether `exit` ran.
    fn pending_exit(state: &AppState, open: impl FnOnce()) -> (bool, bool) {
        let exited = AtomicBool::new(false);
        std::thread::scope(|scope| {
            let waiter = scope.spawn(|| {
                exit_when_unowned(state, Duration::from_secs(20), || {
                    exited.store(true, std::sync::atomic::Ordering::SeqCst)
                })
            });
            open();
            (
                waiter.join().unwrap(),
                exited.load(std::sync::atomic::Ordering::SeqCst),
            )
        })
    }

    /// Adopt `f`'s retiring root as an open does, and bind it to graph-2;
    /// `paused` runs between the adoption and the bind.
    fn reopen(f: &Fixture, state: &AppState, paused: impl FnOnce()) {
        let _load = state.graph_load.lock().unwrap();
        let retirement = state.graphs.read().unwrap().retirement.clone();
        let (store, host) = retirement.adopt(&f.root).unwrap();
        paused();
        let mut fresh = GraphSlot::new(store, f.root.clone());
        *fresh.host.get_mut().unwrap() = host;
        let mut graphs = state.graphs.write().unwrap();
        assert!(graphs
            .bind("graph-2".into(), Arc::new(fresh))
            .unwrap()
            .is_none());
    }

    /// Tear down a state whose graph-2 adopted `f`'s host.
    fn close_reopened(f: &Fixture, state: &AppState, reservation: tine_store::Reservation) {
        drop(reservation);
        assert!(state.graphs.write().unwrap().remove("graph-2").is_none());
        let retirement = state.graphs.read().unwrap().retirement.clone();
        assert_eq!(retirement.wait_idle(Duration::from_secs(20)), Ok(()));
        assert!(f.closed());
    }

    /// Positive control: with no open, the pending exit runs once the
    /// retirement ends.
    #[test]
    fn a_pending_exit_runs_once_the_last_retirement_ends() {
        let f = Fixture::new("exit-runs");
        let state = app_state();
        let reservation = retiring(&f, &state.graphs);
        assert_eq!(
            pending_exit(&state, || {
                std::thread::sleep(Duration::from_millis(200));
                drop(reservation);
            }),
            (true, true)
        );
        assert_eq!(f.disk(), "- two\n");
        assert!(f.closed());
    }

    /// REVIEW-3b-P1 B2: reopening the retiring root adopts it, which empties
    /// the retirement and wakes the pending exit; the bound graph cancels it.
    #[test]
    fn review_p1_b2_a_same_root_adoption_cancels_the_pending_exit() {
        let f = Fixture::new("exit-adopt");
        let state = app_state();
        let reservation = retiring(&f, &state.graphs);
        assert_eq!(
            pending_exit(&state, || reopen(&f, &state, || ())),
            (false, false)
        );
        close_reopened(&f, &state, reservation);
    }

    /// B2: an open paused between its adoption and its bind while the exit
    /// wakes. The exit waits for the open (`graph_load`), then cancels.
    #[test]
    fn review_p1_b2_an_open_paused_between_adopt_and_bind_cancels_the_pending_exit() {
        let f = Fixture::new("exit-paused");
        let state = app_state();
        let reservation = retiring(&f, &state.graphs);
        assert_eq!(
            pending_exit(&state, || reopen(&f, &state, || {
                assert!(state.graphs.read().unwrap().retirement.is_idle());
                std::thread::sleep(Duration::from_millis(300));
            })),
            (false, false)
        );
        close_reopened(&f, &state, reservation);
    }

    /// B2: another root bound while the last retirement runs; the retirement
    /// then ends normally, and the bound graph cancels the exit.
    #[test]
    fn review_p1_b2_an_other_root_bind_cancels_the_pending_exit() {
        let (f, other) = (Fixture::new("exit-a"), Fixture::new("exit-b"));
        let state = app_state();
        let reservation = retiring(&f, &state.graphs);
        let opened = pending_exit(&state, || {
            {
                let _load = state.graph_load.lock().unwrap();
                other.bind(&mut state.graphs.write().unwrap(), "graph-2");
            }
            drop(reservation);
        });
        assert_eq!(opened, (false, false));
        assert!(f.closed(), "the retirement ended");
        assert!(state.graphs.write().unwrap().remove("graph-2").is_none());
        let retirement = state.graphs.read().unwrap().retirement.clone();
        assert_eq!(retirement.wait_idle(Duration::from_secs(20)), Ok(()));
    }
}
