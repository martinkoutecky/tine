//! G3: Quint's bounded progress witnesses, with all work scheduled by the
//! real driver and default Progress. No test calls start_save/begin_draft/poll.
use super::*;
use serde_json::Value;
use std::sync::atomic::AtomicBool;

/// Order an external write after Submit applies and before the next driver
/// step, at the normal delivery boundary (the sink holds no host/path lock).
/// Only conflict setup arms this finite barrier. Save urgency and Progress
/// remain unchanged; all later work runs through the production driver.
struct OrderedChannel {
    channel: Channel,
    pause: Arc<AtomicBool>,
    resume: mpsc::Receiver<()>,
}

impl Sink for OrderedChannel {
    fn deliver(
        &mut self,
        delivery: Delivery,
        owner: &dyn Fn(&str) -> Owner,
        wait: bool,
    ) -> Result<Vec<(Publication, Indexing)>, Delivery> {
        let submitted = delivery
            .mail
            .iter()
            .any(|(_, mail, _)| mail.answer.as_ref().is_some_and(|a| a.took));
        let pause = submitted && self.pause.swap(false, Ordering::SeqCst);
        let result = self.channel.deliver(delivery, owner, wait);
        if pause {
            self.resume
                .recv_timeout(Duration::from_secs(5))
                .expect("G3 conflict interference releases the delivery barrier");
        }
        result
    }
}

fn key(p: usize) -> String {
    format!("{p:04}.md")
}

fn value(t: &Text) -> i64 {
    t.as_ref()
        .map_or(-1, |b| std::str::from_utf8(b).unwrap().parse().unwrap())
}

fn model_host(profile: &str) -> Host<ModelFs> {
    let mut fs = ModelFs {
        weak_graph: matches!(profile, "weak" | "all"),
        ..Default::default()
    };
    fs.external(&key(0), text("1"), true);
    fs.external(&key(1), text("2"), true);
    Host::new(
        fs,
        (0..3).map(|p| (key(p), Arc::new(Mutex::new(())))).collect(),
    )
}

fn submit(d: &Test, receive: &mpsc::Receiver<Delivered>, p: usize, bytes: &str) {
    let version = d.shared.state_for_tests().progress.host.pages[&key(p)].version;
    let id = admit(
        d,
        &key(p),
        RequestKind::Submit {
            bytes: text(bytes),
            version,
            resolve: None,
        },
    );
    answered(receive, id);
}

fn matches_endpoint(h: &Host<ModelFs>, endpoint: &Value) -> bool {
    // Inspect physical durable vehicles independently of the logical index.
    let drafts = drafts::scan(h.fs.draft_files(true)).logical;
    for p in 0..3 {
        let key = key(p);
        for (map, expected) in [
            (&h.fs.files, &endpoint["disk"][p]),
            (&h.fs.stable, &endpoint["stable"][p]),
        ] {
            if value(&map.get(&format!("graph/{key}")).cloned()) != expected.as_i64().unwrap() {
                return false;
            }
        }
        let pg = &endpoint["pages"][p];
        if pg["held"] == false {
            if h.pages.contains_key(&key) {
                return false;
            }
            continue;
        }
        let Some(page) = h.pages.get(&key) else {
            return false;
        };
        let base = match &page.base {
            Base::Known(bytes) => value(bytes),
            Base::Unknown => -3,
        };
        if value(&page.buf) != pg["buf"].as_i64().unwrap()
            || base != pg["base"].as_i64().unwrap()
            || page.conflict != pg["conflict"].as_bool().unwrap()
            || page.risk != pg["risk"].as_bool().unwrap()
            || page.typed != pg["typed"].as_bool().unwrap()
        {
            return false;
        }
        let expected = &endpoint["drafts"][p];
        match drafts.get(&key) {
            None if expected["bytes"] == -2 => {}
            Some(record)
                if value(&record.bytes) == expected["bytes"].as_i64().unwrap()
                    && record.base == page.base
                    && record.version == page.version => {}
            _ => return false,
        }
    }
    h.job.is_none() && h.worker.is_none()
}

/// Weak fairness here is explicit: clocks advance, the driver is repeatedly
/// woken and gets CPU, locks are released, and draft I/O succeeds. Each goal
/// has a 5s wall bound and at most 10,000ms virtual-time advance. These are
/// bounded semantic replay checkpoints, not a temporal or OS fairness proof.
fn eventually(d: &Test, clock: &SharedClock, goal: impl Fn(&Host<ModelFs>) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let initial = clock.now_ms();
    loop {
        if goal(&d.shared.state_for_tests().progress.host) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "G3 progress bound: no model endpoint"
        );
        clock
            .0
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |now| {
                Some((now + 100).min(initial + 10_000))
            })
            .unwrap();
        d.shared.with_state_for_tests(|_| {});
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn g3_default_progress_replays_quint_endpoints_through_driver() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/s2/s36-policy.json")).unwrap();
    let mut cases = 0;
    for case in fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["name"].as_str().unwrap().starts_with("ownProgress"))
    {
        let name = case["name"].as_str().unwrap();
        let clock = SharedClock::default();
        let (send, receive) = mpsc::channel();
        let (resume, release) = mpsc::channel();
        let conflict = matches!(name, "ownProgressConflict" | "ownProgressRefresh");
        let pause = Arc::new(AtomicBool::new(conflict));
        let mut d = Driver::spawn(
            model_host(case["profile"].as_str().unwrap()),
            clock.clone(),
            OrderedChannel {
                channel: Channel(send),
                pause,
                resume: release,
            },
        );
        if name == "ownProgressFailure" {
            // Faults are ready before admission, so an immediately urgent
            // production Submit cannot race the failure setup.
            d.shared.with_state_for_tests(|s| {
                s.progress
                    .host
                    .fs
                    .inject(Phase::PageTemp, [Fault::Before; 3]);
            });
        }
        let count = if name == "ownProgressThree" { 3 } else { 1 };
        for p in 0..count {
            let id = admit(&d, &key(p), RequestKind::Open);
            answered(&receive, id);
            submit(
                &d,
                &receive,
                p,
                if name.contains("Conflict") || name.contains("Refresh") {
                    "2"
                } else {
                    "3"
                },
            );
        }
        if conflict {
            d.shared.with_state_for_tests(|s| {
                let h = &mut s.progress.host;
                assert!(
                    !h.test.submits_wait,
                    "production submit urgency stays enabled"
                );
                assert!(h.job.is_none(), "delivery barrier precedes the save");
                assert_eq!(h.pages[&key(0)].buf, text("2"));
                assert_eq!(h.fs.files["graph/0000.md"], text("1").unwrap());
                h.fs.external(&key(0), text("3"), true);
                s.observe.insert(key(0), Default::default());
            });
            resume.send(()).unwrap();
        }
        if name == "ownProgressRefresh" {
            eventually(&d, &clock, |h| {
                drafts::scan(h.fs.draft_files(true))
                    .logical
                    .get(&key(0))
                    .is_some_and(|r| r.bytes == text("2"))
            });
            submit(&d, &receive, 0, "1");
        }
        eprintln!(
            "G3 {}/{name}: {} model steps",
            case["profile"], case["model_steps"]
        );
        eventually(&d, &clock, |h| matches_endpoint(h, &case["endpoint"]));
        assert!(polls(&d) > 0, "the driver actually ran");
        d.join();
        cases += 1;
    }
    assert_eq!(cases, 20);
}
