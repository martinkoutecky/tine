//! Quick share phase 2 against a mocked GitHub: a local HTTP server that
//! records every request. Nothing here reaches the network.

use super::github::Endpoints;
use super::records::{Records, ShareRow, ShareState, ACCOUNT_FILE, SHARES_FILE};
use super::*;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

/// Every `log` line of this process (tests scan it for secrets).
pub(super) static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

#[derive(Clone, Debug)]
struct Req {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Req {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
    fn form(&self) -> BTreeMap<String, String> {
        String::from_utf8_lossy(&self.body)
            .split('&')
            .filter_map(|kv| kv.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.replace("%3A", ":")))
            .collect()
    }
    fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap()
    }
}

#[derive(Clone)]
struct Resp {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    delay: Duration,
}

fn resp(status: u16, body: impl Into<Vec<u8>>) -> Resp {
    Resp {
        status,
        headers: vec![],
        body: body.into(),
        delay: Duration::ZERO,
    }
}

fn json(status: u16, value: serde_json::Value) -> Resp {
    resp(status, serde_json::to_vec(&value).unwrap())
}

impl Resp {
    fn with(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
    fn delayed(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

type Handler = Arc<dyn Fn(&Req) -> Resp + Send + Sync>;

struct Mock {
    base: String,
    requests: Arc<Mutex<Vec<Req>>>,
}

impl Mock {
    fn start(handler: impl Fn(&Req) -> Resp + Send + Sync + 'static) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let handler: Handler = Arc::new(handler);
        let seen = Arc::clone(&requests);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (handler, seen) = (Arc::clone(&handler), Arc::clone(&seen));
                std::thread::spawn(move || serve(stream, &*handler, &seen));
            }
        });
        Mock { base, requests }
    }

    fn requests(&self) -> Vec<Req> {
        self.requests.lock().unwrap().clone()
    }

    fn count(&self, method: &str, path_prefix: &str) -> usize {
        self.requests()
            .iter()
            .filter(|r| r.method == method && r.path.starts_with(path_prefix))
            .count()
    }

    fn endpoints(&self) -> Endpoints {
        Endpoints {
            oauth: self.base.clone(),
            api: self.base.clone(),
            raw: self.base.clone(),
            https_only: false,
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_millis(1500),
        }
    }
}

fn serve(
    stream: TcpStream,
    handler: &(dyn Fn(&Req) -> Resp + Send + Sync),
    seen: &Mutex<Vec<Req>>,
) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_owned();
    let path = parts.next().unwrap_or("").to_owned();
    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).unwrap();
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((k, v)) = header.split_once(':') {
            headers.push((k.trim().to_owned(), v.trim().to_owned()));
        }
    }
    let length = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    let req = Req {
        method,
        path,
        headers,
        body,
    };
    seen.lock().unwrap().push(req.clone());
    let answer = handler(&req);
    std::thread::sleep(answer.delay);
    let mut out = stream;
    let mut head = format!(
        "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
        answer.status,
        answer.body.len()
    );
    for (k, v) in &answer.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let _ = out.write_all(head.as_bytes());
    let _ = out.write_all(&answer.body);
}

const TOKEN: &str = "gho_mockTokenThatMustNeverLeakRust0001";
const LOGIN: &str = "octo-cat";
const ACCOUNT_ID: u64 = 4242;

fn user_ok(req: &Req) -> Resp {
    assert_eq!(
        req.header("authorization"),
        Some(format!("Bearer {TOKEN}").as_str())
    );
    json(200, serde_json::json!({ "login": LOGIN, "id": ACCOUNT_ID }))
        .with("X-OAuth-Scopes", "gist, read:user")
}

struct Fixture {
    service: Arc<Service>,
    dir: tempfile::TempDir,
}

fn fixture(endpoints: Endpoints) -> Fixture {
    fixture_with_unit(endpoints, Duration::from_millis(1))
}

/// A fixture whose device-flow second lasts `poll_unit` (real time).
fn fixture_with_unit(endpoints: Endpoints, poll_unit: Duration) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let service = Arc::new(Service::new(endpoints, Records::new(dir.path()), poll_unit).unwrap());
    Fixture { service, dir }
}

impl Fixture {
    fn connect(&self) {
        let _guard = records::lock();
        self.service
            .records
            .store_account(&Account {
                login: LOGIN.into(),
                account_id: ACCOUNT_ID,
                token: Token::new(TOKEN.into()),
                scopes: vec!["gist".into()],
                connected_at: "2026-10-17T10:00:00Z".into(),
            })
            .unwrap();
    }
    fn rows(&self) -> Vec<ShareRow> {
        self.service.records.shares().unwrap()
    }
    fn prepared(&self, payload: &[u8]) -> String {
        let id = random_id();
        self.service.prepared.lock().unwrap().push(Prepared {
            id: id.clone(),
            share_id: share::fresh_share_id().unwrap(),
            payload: payload.to_vec(),
            title: "Project".into(),
            pages: 1,
            expires: Instant::now() + PREPARED_TTL,
        });
        id
    }
    fn wait_status(&self, attempt: &str, done: impl Fn(&ConnectStatus) -> bool) -> ConnectStatus {
        for _ in 0..500 {
            let status = self.service.connect_status(attempt);
            if done(&status) {
                return status;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("device flow did not settle");
    }
    fn waits(&self) -> Vec<u64> {
        self.service
            .flow()
            .attempt
            .as_ref()
            .map(|a| a.waits.clone())
            .unwrap_or_default()
    }
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tauri::async_runtime::block_on(future)
}

fn device_code_ok() -> Resp {
    json(
        200,
        serde_json::json!({
            "device_code": "dc-secret-3584d83530557fdd1f46af8289938c8ef79f9dc5",
            "user_code": "WDJB-MJHT",
            "verification_uri": "https://github.com/login/device",
            "expires_in": 900,
            "interval": 5
        }),
    )
}

/// A GitHub whose token endpoint answers the scripted sequence, then the
/// last entry forever.
fn oauth_mock(script: Vec<&'static str>) -> Mock {
    let polls = Arc::new(AtomicUsize::new(0));
    Mock::start(move |req| match req.path.as_str() {
        "/login/device/code" => device_code_ok(),
        "/login/oauth/access_token" => {
            let n = polls.fetch_add(1, Ordering::SeqCst).min(script.len() - 1);
            match script[n] {
                "token" => json(
                    200,
                    serde_json::json!({ "access_token": TOKEN, "token_type": "bearer", "scope": "gist" }),
                ),
                error => json(
                    200,
                    serde_json::json!({ "error": error, "error_description": "x" }),
                ),
            }
        }
        "/user" => user_ok(req),
        _ => resp(404, "{}"),
    })
}

#[test]
fn device_flow_connects_and_slow_down_adds_five_seconds() {
    let mock = oauth_mock(vec![
        "authorization_pending",
        "slow_down",
        "authorization_pending",
        "token",
    ]);
    let f = fixture(mock.endpoints());
    let start = block_on(f.service.connect_start()).unwrap();
    let wire = serde_json::to_string(&start).unwrap();
    assert!(
        !wire.contains("dc-secret"),
        "device_code never leaves Rust: {wire}"
    );
    assert_eq!(start.user_code, "WDJB-MJHT");
    let status = f.wait_status(&start.attempt_id, |s| *s != ConnectStatus::Pending);
    assert_eq!(
        status,
        ConnectStatus::Connected {
            login: LOGIN.into()
        }
    );
    assert_eq!(f.waits(), vec![5, 5, 10, 10]);
    let device = mock
        .requests()
        .into_iter()
        .find(|r| r.path == "/login/device/code")
        .unwrap();
    assert_eq!(device.form()["client_id"], github::CLIENT_ID);
    assert_eq!(device.form()["scope"], "gist");
    let poll = mock
        .requests()
        .into_iter()
        .find(|r| r.path == "/login/oauth/access_token")
        .unwrap();
    assert_eq!(
        poll.form()["grant_type"],
        "urn:ietf:params:oauth:grant-type:device_code"
    );
    assert!(poll.header("user-agent").unwrap().starts_with("Tine/"));
    let account = f.service.records.account().unwrap();
    assert_eq!(
        (account.login.as_str(), account.account_id),
        (LOGIN, ACCOUNT_ID)
    );
    assert_eq!(account.token.expose(), TOKEN);
    assert_no_secret(&format!("{account:?}"));
    assert_no_secret(&serde_json::to_string(&f.service.listing()).unwrap());
    assert_no_secret(&serde_json::to_string(&status).unwrap());
    assert_log_clean();
}

#[test]
fn device_flow_stops_on_expired_denied_and_its_own_expiry() {
    for (script, want) in [
        (
            vec!["authorization_pending", "expired_token"],
            ConnectStatus::Expired,
        ),
        (vec!["access_denied"], ConnectStatus::Denied),
        (
            vec!["unsupported_grant_type"],
            ConnectStatus::Failed {
                reason: "refused".into(),
            },
        ),
    ] {
        let mock = oauth_mock(script);
        let f = fixture(mock.endpoints());
        let start = block_on(f.service.connect_start()).unwrap();
        assert_eq!(
            f.wait_status(&start.attempt_id, |s| *s != ConnectStatus::Pending),
            want
        );
        assert!(f.service.records.account().is_none());
    }
    // Pending forever: the returned `expires_in` ends the attempt.
    let mock = Mock::start(|req| match req.path.as_str() {
        "/login/device/code" => json(
            200,
            serde_json::json!({ "device_code": "dc", "user_code": "AB-CD", "verification_uri": "https://github.com/login/device", "expires_in": 12, "interval": 5 }),
        ),
        _ => json(200, serde_json::json!({ "error": "authorization_pending" })),
    });
    // A 10 ms "second": the code's 120 ms lifetime holds polls at 50 and
    // 100 ms with room for the local round trips.
    let f = fixture_with_unit(mock.endpoints(), Duration::from_millis(10));
    let start = block_on(f.service.connect_start()).unwrap();
    assert_eq!(
        f.wait_status(&start.attempt_id, |s| *s != ConnectStatus::Pending),
        ConnectStatus::Expired
    );
    assert_eq!(mock.count("POST", "/login/oauth/access_token"), 2);
}

/// A device code that lives `expires_in` 10 ms "seconds" (1 ms each), polled
/// every 1, against `access` and `user` answers.
fn short_lived_code(access: Resp, user: Option<Duration>) -> Mock {
    Mock::start(move |req| match req.path.as_str() {
        "/login/device/code" => json(
            200,
            serde_json::json!({ "device_code": "dc", "user_code": "AB-CD", "verification_uri": "https://github.com/login/device", "expires_in": 10, "interval": 1 }),
        ),
        "/login/oauth/access_token" => access.clone(),
        "/user" => user_ok(req).delayed(user.unwrap_or_default()),
        _ => resp(404, "{}"),
    })
}

/// REVIEW-P2 2: the code's lifetime is elapsed time, network waits
/// included. A slow pending answer that outlives the code ends the attempt:
/// no further poll is sent.
#[test]
fn a_slow_answer_that_outlives_the_code_expires_the_attempt_and_polls_no_more() {
    let pending = json(200, serde_json::json!({ "error": "authorization_pending" }))
        .delayed(Duration::from_millis(40));
    let mock = short_lived_code(pending, None);
    let f = fixture(mock.endpoints());
    let start = block_on(f.service.connect_start()).unwrap();
    assert_eq!(
        f.wait_status(&start.attempt_id, |s| *s != ConnectStatus::Pending),
        ConnectStatus::Expired
    );
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(
        mock.count("POST", "/login/oauth/access_token"),
        1,
        "no poll after the code expired"
    );
}

/// REVIEW-P2 2: a token whose answer (or account lookup) completes after
/// the code expired is dropped, never stored.
#[test]
fn a_token_completing_after_the_code_expired_is_not_stored() {
    let token = || {
        json(
            200,
            serde_json::json!({ "access_token": TOKEN, "token_type": "bearer", "scope": "gist" }),
        )
    };
    for (access, user) in [
        (token().delayed(Duration::from_millis(40)), None),
        (token(), Some(Duration::from_millis(40))),
    ] {
        let mock = short_lived_code(access, user);
        let f = fixture(mock.endpoints());
        let start = block_on(f.service.connect_start()).unwrap();
        assert_eq!(
            f.wait_status(&start.attempt_id, |s| *s != ConnectStatus::Pending),
            ConnectStatus::Expired
        );
        std::thread::sleep(Duration::from_millis(100));
        assert!(
            f.service.records.account().is_none(),
            "an expired code's token is not stored"
        );
        assert!(!f.dir.path().join(ACCOUNT_FILE).exists());
    }
}

#[test]
fn device_flow_refuses_a_verification_page_off_github() {
    let mock = Mock::start(|_| {
        json(
            200,
            serde_json::json!({ "device_code": "dc", "user_code": "AB-CD", "verification_uri": "https://evil.example/login", "expires_in": 900, "interval": 5 }),
        )
    });
    let f = fixture(mock.endpoints());
    assert_eq!(
        block_on(f.service.connect_start()).unwrap_err(),
        QsError::Unavailable
    );
}

#[test]
fn cancel_stops_polling() {
    let mock = oauth_mock(vec!["authorization_pending"]);
    let f = fixture(mock.endpoints());
    let start = block_on(f.service.connect_start()).unwrap();
    while mock.count("POST", "/login/oauth/access_token") < 2 {
        std::thread::sleep(Duration::from_millis(5));
    }
    f.service.connect_cancel(&start.attempt_id);
    assert_eq!(
        f.service.connect_status(&start.attempt_id),
        ConnectStatus::Failed {
            reason: "superseded".into()
        }
    );
    std::thread::sleep(Duration::from_millis(100));
    let after = mock.count("POST", "/login/oauth/access_token");
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        mock.count("POST", "/login/oauth/access_token"),
        after,
        "polling stopped"
    );
}

/// A GitHub whose token endpoint answers the token only once `release` fires.
fn held_token_mock() -> (Mock, mpsc::Sender<()>, mpsc::Receiver<()>) {
    let (release, held) = mpsc::channel::<()>();
    let (arrived, polled) = mpsc::channel::<()>();
    let held = Mutex::new(held);
    let arrived = Mutex::new(arrived);
    let mock = Mock::start(move |req| match req.path.as_str() {
        "/login/device/code" => device_code_ok(),
        "/login/oauth/access_token" => {
            let _ = arrived.lock().unwrap().send(());
            held.lock().unwrap().recv().unwrap();
            json(200, serde_json::json!({ "access_token": TOKEN }))
        }
        "/user" => user_ok(req),
        _ => resp(404, "{}"),
    });
    (mock, release, polled)
}

#[test]
fn a_token_for_a_cancelled_attempt_is_dropped_never_stored() {
    let (mock, release, polled) = held_token_mock();
    let f = fixture(mock.endpoints());
    let start = block_on(f.service.connect_start()).unwrap();
    polled.recv_timeout(Duration::from_secs(5)).unwrap();
    f.service.connect_cancel(&start.attempt_id);
    release.send(()).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(f.service.records.account().is_none(), "late token stored");
    assert!(!f.dir.path().join(ACCOUNT_FILE).exists());
    assert_eq!(
        mock.count("GET", "/user"),
        0,
        "a dropped token is never used"
    );
}

#[test]
fn a_superseded_attempt_drops_its_token_and_the_new_one_connects() {
    let (mock, release, polled) = held_token_mock();
    let f = fixture(mock.endpoints());
    let first = block_on(f.service.connect_start()).unwrap();
    polled.recv_timeout(Duration::from_secs(5)).unwrap();
    let second = block_on(f.service.connect_start()).unwrap();
    assert_eq!(
        f.service.connect_status(&first.attempt_id),
        ConnectStatus::Failed {
            reason: "superseded".into()
        }
    );
    release.send(()).unwrap(); // the first attempt's token
    polled.recv_timeout(Duration::from_secs(5)).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        f.service.records.account().is_none(),
        "first attempt's token stored"
    );
    release.send(()).unwrap(); // the second attempt's token
    assert_eq!(
        f.wait_status(&second.attempt_id, |s| *s != ConnectStatus::Pending),
        ConnectStatus::Connected {
            login: LOGIN.into()
        }
    );
    assert_eq!(mock.count("GET", "/user"), 1);
}

#[test]
fn disconnect_invalidates_a_sign_in_and_removes_the_account() {
    let (mock, release, polled) = held_token_mock();
    let f = fixture(mock.endpoints());
    f.connect();
    let start = block_on(f.service.connect_start()).unwrap();
    polled.recv_timeout(Duration::from_secs(5)).unwrap();
    f.service.disconnect().unwrap();
    assert!(!f.dir.path().join(ACCOUNT_FILE).exists());
    release.send(()).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(f.service.records.account().is_none());
    assert_eq!(
        f.service.connect_status(&start.attempt_id),
        ConnectStatus::Failed {
            reason: "superseded".into()
        }
    );
    assert!(f.service.listing().account.is_none());
}

#[test]
fn a_token_without_the_gist_scope_fails_the_connection() {
    let mock = Mock::start(|req| match req.path.as_str() {
        "/login/device/code" => device_code_ok(),
        "/login/oauth/access_token" => json(200, serde_json::json!({ "access_token": TOKEN })),
        "/user" => json(200, serde_json::json!({ "login": LOGIN, "id": ACCOUNT_ID }))
            .with("X-OAuth-Scopes", "read:user, gist_extra"),
        _ => resp(404, "{}"),
    });
    let f = fixture(mock.endpoints());
    let start = block_on(f.service.connect_start()).unwrap();
    assert_eq!(
        f.wait_status(&start.attempt_id, |s| *s != ConnectStatus::Pending),
        ConnectStatus::Failed {
            reason: "scope".into()
        }
    );
    assert!(f.service.records.account().is_none());
}

// ---- create ----

fn gist_store() -> Arc<Mutex<Vec<serde_json::Value>>> {
    Arc::new(Mutex::new(Vec::new()))
}

/// A GitHub that keeps created gists and serves them back as listings and
/// raw files. `create` decides the create answer.
fn gist_mock(
    gists: Arc<Mutex<Vec<serde_json::Value>>>,
    create: impl Fn(&Req) -> Resp + Send + Sync + 'static,
) -> Mock {
    Mock::start(move |req| {
        let base = format!("http://{}", req.header("host").unwrap());
        match (req.method.as_str(), req.path.as_str()) {
            ("POST", "/gists") => {
                let body = req.json();
                let id = format!("{:032x}", gists.lock().unwrap().len() + 0xabc);
                let files: serde_json::Map<String, serde_json::Value> = body["files"]
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(name, file)| {
                        (
                            name.clone(),
                            serde_json::json!({ "filename": name, "content": file["content"], "raw_url": format!("{base}/{LOGIN}/{id}/raw/0/{name}") }),
                        )
                    })
                    .collect();
                gists.lock().unwrap().insert(
                    0,
                    serde_json::json!({ "id": id, "description": body["description"], "public": body["public"], "created_at": now_rfc3339(), "owner": { "id": ACCOUNT_ID, "login": LOGIN }, "files": files }),
                );
                let answer = create(req);
                if answer.status == 201 && answer.body.is_empty() {
                    json(201, serde_json::json!({ "id": id })).delayed(answer.delay)
                } else {
                    answer
                }
            }
            ("GET", path) if path.starts_with("/gists?") => {
                let listed: Vec<_> = gists
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|g| {
                        let mut g = g.clone();
                        for file in g["files"].as_object_mut().unwrap().values_mut() {
                            file.as_object_mut().unwrap().remove("content");
                        }
                        g
                    })
                    .collect();
                json(200, serde_json::Value::Array(listed))
            }
            ("GET", path) if path.contains("/raw/") => {
                let name = path.rsplit('/').next().unwrap();
                let id = path.split('/').nth(2).unwrap();
                let gists = gists.lock().unwrap();
                match gists.iter().find(|g| g["id"] == id) {
                    Some(g) => resp(
                        200,
                        g["files"][name]["content"]
                            .as_str()
                            .unwrap()
                            .as_bytes()
                            .to_vec(),
                    ),
                    None => resp(404, "Not Found"),
                }
            }
            ("GET", path) if path.starts_with("/gists/") => {
                let id = &path["/gists/".len()..];
                match gists.lock().unwrap().iter().find(|g| g["id"] == id) {
                    Some(g) => json(200, g.clone()),
                    None => json(404, serde_json::json!({ "message": "Not Found" })),
                }
            }
            ("DELETE", path) if path.starts_with("/gists/") => {
                let id = &path["/gists/".len()..];
                let mut gists = gists.lock().unwrap();
                let before = gists.len();
                gists.retain(|g| g["id"] != id);
                if gists.len() < before {
                    resp(204, "")
                } else {
                    json(404, serde_json::json!({ "message": "Not Found" }))
                }
            }
            _ => json(404, serde_json::json!({ "message": "Not Found" })),
        }
    })
}

/// Stand-in for a prepared `P`: create uploads these bytes as they are.
fn payload() -> Vec<u8> {
    b"\x1f\x8b prepared share document bytes".to_vec()
}

fn key_of(link: &str) -> [u8; 32] {
    let key = link.rsplit('/').next().unwrap();
    share::b64u_decode(key).unwrap().try_into().unwrap()
}

#[test]
fn create_uploads_the_prepared_bytes_once_and_the_row_goes_pending_then_live() {
    let gists = gist_store();
    let dir = tempfile::tempdir().unwrap();
    let shares_path = dir.path().join(SHARES_FILE);
    let mock = gist_mock(Arc::clone(&gists), move |_| {
        // Before POST /gists the row is on disk as pending, with its key.
        let rows: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&shares_path).unwrap()).unwrap();
        assert_eq!(rows["shares"][0]["state"], "pending");
        assert_eq!(rows["shares"][0]["key"].as_str().unwrap().len(), 43);
        resp(201, "")
    });
    let service = Arc::new(
        Service::new(
            mock.endpoints(),
            Records::new(dir.path()),
            Duration::from_millis(1),
        )
        .unwrap(),
    );
    let f = Fixture { service, dir };
    f.connect();
    let bytes = payload();
    let id = f.prepared(&bytes);
    let mut link = String::new();
    let created = block_on(f.service.create_share(&id, |l| {
        link = l.to_owned();
        true
    }))
    .unwrap();
    assert!(created.copied);
    let wire = serde_json::to_string(&created).unwrap();
    let key = key_of(&link);
    assert!(
        !wire.contains(&share::b64u(&key)),
        "the key stays out of command results"
    );
    assert!(link.starts_with(&format!(
        "https://tine.page/s/1/#{LOGIN}/{}/",
        created.gist_id
    )));
    // The uploaded envelope opens to exactly the prepared bytes.
    let post = mock
        .requests()
        .into_iter()
        .find(|r| r.method == "POST")
        .unwrap();
    let body = post.json();
    assert_eq!(body["public"], false);
    assert_eq!(body["description"], GIST_DESCRIPTION);
    assert_eq!(body["files"][README_FILE]["content"], README_TEXT);
    assert_eq!(body["files"].as_object().unwrap().len(), 2);
    let envelope = body["files"][SHARE_FILE]["content"].as_str().unwrap();
    assert_eq!(share::open(envelope, &key).unwrap(), bytes);
    assert_eq!(post.header("x-github-api-version"), Some("2022-11-28"));
    let rows = f.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].state, rows[0].gist_id.as_deref()),
        (ShareState::Live, Some(created.gist_id.as_str()))
    );
    assert_eq!(
        rows[0].share_id,
        share::b64u(&envelope_share_id(envelope).unwrap())
    );
    // At most once: the prepared id is consumed.
    assert_eq!(
        block_on(f.service.create_share(&id, |_| true)).unwrap_err(),
        QsError::PreviewExpired
    );
    assert_eq!(mock.count("POST", "/gists"), 1);
    // Copy link reproduces the link; the listing carries no key.
    let mut again = String::new();
    f.service
        .copy_link(&created.share_id, |l| {
            again = l.to_owned();
            true
        })
        .unwrap();
    assert_eq!(again, link);
    let listing = serde_json::to_string(&f.service.listing()).unwrap();
    assert!(!listing.contains(&share::b64u(&key)));
    assert_no_secret(&listing);
    assert!(LOG
        .lock()
        .unwrap()
        .iter()
        .all(|l| !l.contains(&share::b64u(&key))));
    assert_log_clean();
}

#[test]
fn create_failures_known_before_sending_remove_the_row() {
    // A closed port: connection refused, nothing reached GitHub.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let base = format!("http://127.0.0.1:{port}");
    let f = fixture(Endpoints {
        oauth: base.clone(),
        api: base.clone(),
        raw: base,
        https_only: false,
        connect_timeout: Duration::from_secs(2),
        read_timeout: Duration::from_secs(2),
    });
    f.connect();
    let id = f.prepared(&payload());
    assert_eq!(
        block_on(f.service.create_share(&id, |_| true)).unwrap_err(),
        QsError::Network
    );
    assert!(f.rows().is_empty());
}

#[test]
fn create_failures_after_sending_keep_the_row_pending_and_never_retry() {
    for (answer, label) in [
        (resp(502, "bad gateway"), "5xx"),
        (
            resp(201, "").delayed(Duration::from_secs(3)),
            "timeout after send",
        ),
        (resp(201, "not json"), "unreadable 201"),
    ] {
        let answer = Arc::new(Mutex::new(Some(answer)));
        let mock = gist_mock(gist_store(), move |_| {
            answer.lock().unwrap().take().unwrap()
        });
        let f = fixture(mock.endpoints());
        f.connect();
        let id = f.prepared(&payload());
        let error = block_on(
            f.service
                .create_share(&id, |_| panic!("no link for an unknown outcome")),
        )
        .unwrap_err();
        let rows = f.rows();
        assert_eq!(
            error,
            QsError::MaybeUploaded {
                share_id: rows[0].share_id.clone()
            },
            "{label}"
        );
        assert_eq!(rows[0].state, ShareState::Pending, "{label}");
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(mock.count("POST", "/gists"), 1, "{label}: never retried");
    }
}

#[test]
fn create_refusals_from_github_remove_the_row_with_the_mapped_error() {
    for (answer, want) in [
        (json(401, serde_json::json!({ "message": "Bad credentials" })), QsError::Reconnect),
        (json(403, serde_json::json!({ "message": "slow" })).with("Retry-After", "120"), QsError::RateLimited { minutes: 2 }),
        (
            json(403, serde_json::json!({ "message": "limit" })).with("x-ratelimit-remaining", "0").with("x-ratelimit-reset", &(github_now() + 600).to_string()),
            QsError::RateLimited { minutes: 10 },
        ),
        (json(429, serde_json::json!({ "message": "slow" })).with("Retry-After", "30"), QsError::RateLimited { minutes: 1 }),
        (
            json(403, serde_json::json!({ "message": format!("Resource not accessible by integration {TOKEN}") })).with("x-ratelimit-remaining", "4999").with("x-ratelimit-reset", "1"),
            QsError::GitHub { message: "Resource not accessible by integration <redacted>".into() },
        ),
        (json(422, serde_json::json!({ "message": "Validation Failed" })), QsError::GitHub { message: "GitHub refused the request (422)".into() }),
    ] {
        let answer = Arc::new(Mutex::new(Some(answer)));
        let mock = gist_mock(gist_store(), move |_| answer.lock().unwrap().take().unwrap());
        let f = fixture(mock.endpoints());
        f.connect();
        let id = f.prepared(&payload());
        let error = block_on(f.service.create_share(&id, |_| true)).unwrap_err();
        assert_eq!(error, want);
        assert_no_secret(&serde_json::to_string(&error).unwrap());
        assert!(f.rows().is_empty(), "{want:?}");
    }
}

fn github_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[test]
fn create_needs_a_connected_account_and_readable_records() {
    let mock = gist_mock(gist_store(), |_| resp(201, ""));
    let f = fixture(mock.endpoints());
    let id = f.prepared(&payload());
    assert_eq!(
        block_on(f.service.create_share(&id, |_| true)).unwrap_err(),
        QsError::NotConnected
    );
    f.connect();
    // A damaged shares file holds keys found nowhere else: never overwritten,
    // and nothing is uploaded.
    std::fs::write(f.dir.path().join(SHARES_FILE), b"{\"v\":1,\"shares\":[{tor").unwrap();
    let id = f.prepared(&payload());
    assert!(matches!(
        block_on(f.service.create_share(&id, |_| true)).unwrap_err(),
        QsError::Records { .. }
    ));
    assert_eq!(
        std::fs::read(f.dir.path().join(SHARES_FILE)).unwrap(),
        b"{\"v\":1,\"shares\":[{tor"
    );
    assert_eq!(mock.count("POST", "/gists"), 0);
    // A newer format is left alone too.
    std::fs::write(f.dir.path().join(SHARES_FILE), b"{\"v\":2,\"shares\":[]}").unwrap();
    assert!(f.service.listing().error.is_some());
    assert!(f.service.forget("x").is_err());
}

#[test]
fn an_expired_or_discarded_preview_does_not_upload() {
    let mock = gist_mock(gist_store(), |_| resp(201, ""));
    let f = fixture(mock.endpoints());
    f.connect();
    let id = f.prepared(&payload());
    f.service.discard_prepared(&id);
    assert_eq!(
        block_on(f.service.create_share(&id, |_| true)).unwrap_err(),
        QsError::PreviewExpired
    );
    let id = f.prepared(&payload());
    f.service.prepared.lock().unwrap()[0].expires = Instant::now() - Duration::from_secs(1);
    assert_eq!(
        block_on(f.service.create_share(&id, |_| true)).unwrap_err(),
        QsError::PreviewExpired
    );
    assert_eq!(mock.count("POST", "/gists"), 0);
}

#[test]
fn prepare_then_create_uploads_the_previewed_text_not_a_later_edit() {
    let root = tempfile::tempdir().unwrap();
    let graph = root.path().join("graph");
    std::fs::create_dir_all(graph.join("pages")).unwrap();
    std::fs::write(graph.join("pages/Project.md"), "- FIRST-VERSION text\n").unwrap();
    let store = tine_store::Store::open(&graph, Default::default())
        .unwrap()
        .0;
    store.whole_graph().unwrap();
    let scratch = root.path().join("scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    let mock = gist_mock(gist_store(), |_| resp(201, ""));
    let f = fixture(mock.endpoints());
    f.connect();
    let request = QueryExportRequest {
        argument: "@page and name = 'Project'".into(),
        dialect: tine_core::query::wire_parse::QueryTextDialect::MacroTql,
        properties: vec![],
        current_page: Some("Project".into()),
        name: "Project".into(),
        host_block_id: None,
        folder: None,
        replace: false,
        asset_budget_bytes: None,
        include_query_text: false,
    };
    let prepared = f.service.prepare_share(&store, &request, &scratch).unwrap();
    assert_eq!(prepared.preview.pages.len(), 1);
    assert_eq!(std::fs::read_dir(&scratch).unwrap().count(), 0);
    // An edit after the preview is not in the share.
    std::fs::write(graph.join("pages/Project.md"), "- EDITED-LATER text\n").unwrap();
    let mut link = String::new();
    let created = block_on(f.service.create_share(&prepared.prepare_id, |l| {
        link = l.to_owned();
        true
    }))
    .unwrap();
    let post = mock
        .requests()
        .into_iter()
        .find(|r| r.method == "POST")
        .unwrap();
    let envelope = post.json()["files"][SHARE_FILE]["content"]
        .as_str()
        .unwrap()
        .to_owned();
    let p = share::open(&envelope, &key_of(&link)).unwrap();
    let document =
        String::from_utf8(share::gunzip_bounded(&p, share::MAX_DOCUMENT_BYTES).unwrap()).unwrap();
    assert!(document.starts_with("{\"quick_share\":1,"));
    assert!(document.contains("FIRST-VERSION") && !document.contains("EDITED-LATER"));
    let row = &f.rows()[0];
    assert_eq!(
        (row.title.as_str(), row.pages, row.gist_id.as_deref()),
        ("Project", 1, Some(created.gist_id.as_str()))
    );
    // D-20261018-QS3: the record id prepare answered (the webview's
    // `pending/<id>` marker) is the row's and the envelope's share id.
    assert_eq!(created.share_id, prepared.share_id);
    assert_eq!(row.share_id, prepared.share_id);
    let header = share::b64u_decode(&envelope).unwrap();
    assert_eq!(share::b64u(&header[5..21]), prepared.share_id);
    assert!(!share::looks_like_gist_id(&prepared.share_id));
}

// ---- check, find, delete, forget ----

/// A pending row whose gist GitHub did create (the 5xx after a create).
fn pending_share(gists: &Arc<Mutex<Vec<serde_json::Value>>>) -> (Mock, Fixture, String) {
    let mock = gist_mock(Arc::clone(gists), |_| resp(500, "oops"));
    let f = fixture(mock.endpoints());
    f.connect();
    let id = f.prepared(&payload());
    let QsError::MaybeUploaded { share_id } =
        block_on(f.service.create_share(&id, |_| true)).unwrap_err()
    else {
        panic!("expected an unknown outcome");
    };
    (mock, f, share_id)
}

/// A gist that carries the marker, under the given envelope.
fn decoy(id: &str, created_at: &str, envelope: &str, base: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id, "description": GIST_DESCRIPTION, "created_at": created_at, "owner": { "id": ACCOUNT_ID },
        "files": {
            README_FILE: { "content": README_TEXT, "raw_url": format!("{base}/{LOGIN}/{id}/raw/0/{README_FILE}") },
            SHARE_FILE: { "content": envelope, "raw_url": format!("{base}/{LOGIN}/{id}/raw/0/{SHARE_FILE}") },
        }
    })
}

#[test]
fn check_confirms_a_pending_share_by_share_id_and_key() {
    let gists = gist_store();
    let (mock, f, share_id) = pending_share(&gists);
    let wanted: [u8; 16] = share::b64u_decode(&share_id).unwrap().try_into().unwrap();
    // Newer decoys: same share id under another key, and another share.
    let forged = share::seal(b"forged", &[9; 32], &wanted, &[1; 12]);
    let other = share::seal_fresh(b"other").unwrap().envelope;
    gists.lock().unwrap().insert(
        0,
        decoy(&"e".repeat(32), &now_rfc3339(), &forged, &mock.base),
    );
    gists.lock().unwrap().insert(
        0,
        decoy(&"d".repeat(32), &now_rfc3339(), &other, &mock.base),
    );
    let real = gists.lock().unwrap()[2]["id"].as_str().unwrap().to_owned();
    assert_eq!(
        block_on(f.service.check_share(&share_id)).unwrap(),
        CheckOutcome::Live
    );
    let rows = f.rows();
    assert_eq!(
        (rows[0].state, rows[0].gist_id.as_deref()),
        (ShareState::Live, Some(real.as_str()))
    );
    // The listing is authenticated; raw reads carry no credentials.
    for r in mock.requests() {
        if r.path.contains("/raw/") {
            assert_eq!(r.header("authorization"), None);
        } else if r.path.starts_with("/gists") {
            assert!(r.header("authorization").is_some());
        }
    }
}

#[test]
fn check_follows_pages_stops_a_day_before_and_leaves_absence_pending() {
    let gists = gist_store();
    let (mock, f, share_id) = pending_share(&gists);
    let row_created = parse_time(&f.rows()[0].created_at).unwrap();
    let real = gists.lock().unwrap().remove(0);
    let base = mock.base.clone();
    // Page 1: a non-share gist and a Link to page 2, which holds the share.
    let paged = Mock::start(move |req| {
        let base2 = format!("http://{}", req.header("host").unwrap());
        match req.path.as_str() {
            "/gists?per_page=100" => json(200, serde_json::json!([{ "id": "f".repeat(32), "description": "notes", "created_at": now_rfc3339(), "files": { "a.md": {} } }]))
                .with("Link", &format!("<{base2}/gists?per_page=100&page=2>; rel=\"next\"")),
            "/gists?per_page=100&page=2" => {
                let mut g = real.clone();
                for file in g["files"].as_object_mut().unwrap().values_mut() {
                    let url = file["raw_url"].as_str().unwrap().replace(&base, &base2);
                    file["raw_url"] = serde_json::Value::String(url);
                }
                json(200, serde_json::json!([g]))
            }
            path if path.contains("/raw/") => resp(200, real["files"][SHARE_FILE]["content"].as_str().unwrap().as_bytes().to_vec()),
            _ => resp(404, "{}"),
        }
    });
    let f2 = Fixture {
        service: Arc::new(
            Service::new(
                paged.endpoints(),
                Records::new(f.dir.path()),
                Duration::from_millis(1),
            )
            .unwrap(),
        ),
        dir: tempfile::tempdir().unwrap(),
    };
    assert_eq!(
        block_on(f2.service.check_share(&share_id)).unwrap(),
        CheckOutcome::Live
    );
    // Absence is not proof: an empty listing leaves the row pending.
    let gists = gist_store();
    let (mock, f, share_id) = pending_share(&gists);
    gists.lock().unwrap().clear();
    assert_eq!(
        block_on(f.service.check_share(&share_id)).unwrap(),
        CheckOutcome::NotFoundYet
    );
    assert_eq!(f.rows()[0].state, ShareState::Pending);
    drop(mock);
    // A gist made more than a day before the row ends the search.
    let old = (row_created - chrono::Duration::days(2)).to_rfc3339();
    let stop = Mock::start(move |req| {
        match req.path.as_str() {
        "/gists?per_page=100" => json(200, serde_json::json!([{ "id": "c".repeat(32), "description": GIST_DESCRIPTION, "created_at": old, "files": {} }]))
            .with("Link", "<http://127.0.0.1:1/gists?page=2>; rel=\"next\""),
        _ => resp(404, "{}"),
    }
    });
    let f3 = Fixture {
        service: Arc::new(
            Service::new(
                stop.endpoints(),
                Records::new(f.dir.path()),
                Duration::from_millis(1),
            )
            .unwrap(),
        ),
        dir: tempfile::tempdir().unwrap(),
    };
    assert_eq!(
        block_on(f3.service.check_share(&share_id)).unwrap(),
        CheckOutcome::NotFoundYet
    );
    assert_eq!(stop.requests().len(), 1);
}

#[test]
fn check_and_delete_run_only_for_the_rows_account() {
    let gists = gist_store();
    let (mock, f, share_id) = pending_share(&gists);
    let before = mock.requests().len();
    {
        let _g = records::lock();
        f.service
            .records
            .store_account(&Account {
                login: "someone-else".into(),
                account_id: 7,
                token: Token::new("gho_other".into()),
                scopes: vec!["gist".into()],
                connected_at: "x".into(),
            })
            .unwrap();
    }
    let wrong = QsError::WrongAccount {
        login: LOGIN.into(),
    };
    assert_eq!(
        block_on(f.service.check_share(&share_id)).unwrap_err(),
        wrong
    );
    assert_eq!(
        block_on(f.service.delete(Some(&share_id), None)).unwrap_err(),
        wrong
    );
    assert_eq!(
        mock.requests().len(),
        before,
        "nothing was sent for another account's share"
    );
    assert!(!f.service.listing().shares[0].account_matches);
}

#[test]
fn delete_removes_a_live_share_and_a_404_marks_it_gone() {
    let gists = gist_store();
    let mock = gist_mock(Arc::clone(&gists), |_| resp(201, ""));
    let f = fixture(mock.endpoints());
    f.connect();
    let created = block_on(f.service.create_share(&f.prepared(&payload()), |_| true)).unwrap();
    let second = block_on(f.service.create_share(&f.prepared(&payload()), |_| true)).unwrap();
    assert_eq!(
        block_on(f.service.delete(Some(&created.share_id), None)).unwrap(),
        DeleteOutcome::Deleted
    );
    assert_eq!(f.rows().len(), 1);
    assert_eq!(
        mock.count("DELETE", &format!("/gists/{}", created.gist_id)),
        1
    );
    // Deleted on GitHub meanwhile: "No longer on GitHub"; the key stays until Forget.
    gists.lock().unwrap().clear();
    assert_eq!(
        block_on(f.service.delete(Some(&second.share_id), None)).unwrap(),
        DeleteOutcome::Gone
    );
    assert_eq!(f.rows().len(), 1);
    f.service.forget(&second.share_id).unwrap();
    assert!(f.rows().is_empty());
}

#[test]
fn find_lists_unknown_tine_shares_and_deletes_only_marked_gists_of_this_account() {
    let gists = gist_store();
    let mock = gist_mock(Arc::clone(&gists), |_| resp(201, ""));
    let f = fixture(mock.endpoints());
    f.connect();
    let known = block_on(f.service.create_share(&f.prepared(&payload()), |_| true)).unwrap();
    let env = share::seal_fresh(b"x").unwrap().envelope;
    let base = mock.base.clone();
    let elsewhere = "1".repeat(32);
    let mut extension = decoy(&"2".repeat(32), &now_rfc3339(), &env, &base);
    extension["files"]["a0123456789abcdef.bin"] = serde_json::json!({ "raw_url": "x" });
    let mut extra = decoy(&"3".repeat(32), &now_rfc3339(), &env, &base);
    extra["files"]["notes.md"] = serde_json::json!({ "raw_url": "x" });
    let mut described = decoy(&"4".repeat(32), &now_rfc3339(), &env, &base);
    described["description"] = serde_json::json!("Tine share");
    let mut foreign = decoy(&"5".repeat(32), &now_rfc3339(), &env, &base);
    foreign["owner"]["id"] = serde_json::json!(99);
    gists.lock().unwrap().extend([
        decoy(&elsewhere, &now_rfc3339(), &env, &base),
        extension,
        extra,
        described,
        foreign,
    ]);
    let found: Vec<String> = block_on(f.service.find_shares())
        .unwrap()
        .into_iter()
        .map(|s| s.gist_id)
        .collect();
    assert_eq!(
        found,
        vec![elsewhere.clone(), "2".repeat(32), "5".repeat(32)]
    );
    assert!(!found.contains(&known.gist_id));
    assert!(matches!(
        block_on(f.service.delete(None, Some(&"3".repeat(32)))).unwrap_err(),
        QsError::Refused { .. }
    ));
    assert!(matches!(
        block_on(f.service.delete(None, Some(&"5".repeat(32)))).unwrap_err(),
        QsError::Refused { .. }
    ));
    assert_eq!(mock.count("DELETE", "/gists/"), 0);
    assert_eq!(
        block_on(f.service.delete(None, Some(&elsewhere))).unwrap(),
        DeleteOutcome::Deleted
    );
    assert_eq!(mock.count("DELETE", "/gists/"), 1);
}

#[test]
fn rest_errors_map_to_their_sentences() {
    for (answer, want) in [
        (
            json(401, serde_json::json!({ "message": "Bad credentials" })),
            QsError::Reconnect,
        ),
        (
            json(403, serde_json::json!({ "message": "x" })).with("Retry-After", "600"),
            QsError::RateLimited { minutes: 10 },
        ),
        (
            json(
                403,
                serde_json::json!({ "message": "Must have admin rights" }),
            )
            .with("x-ratelimit-remaining", "10")
            .with("x-ratelimit-reset", "99999999999"),
            QsError::GitHub {
                message: "Must have admin rights".into(),
            },
        ),
        (
            resp(403, "<html>"),
            QsError::GitHub {
                message: "GitHub refused the request".into(),
            },
        ),
        (resp(503, ""), QsError::Unavailable),
    ] {
        let answer = Arc::new(Mutex::new(Some(answer)));
        let mock = Mock::start(move |_| {
            answer
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(|| resp(500, ""))
        });
        let f = fixture(mock.endpoints());
        f.connect();
        assert_eq!(block_on(f.service.find_shares()).unwrap_err(), want);
    }
}

// ---- secrets and records ----

fn assert_no_secret(text: &str) {
    assert!(!text.contains(TOKEN), "token leaked: {text}");
    assert!(!text.contains("dc-secret"), "device code leaked: {text}");
}

fn assert_log_clean() {
    for line in LOG.lock().unwrap().iter() {
        assert_no_secret(line);
    }
}

#[test]
fn secrets_print_redacted() {
    let token = Token::new(TOKEN.into());
    assert_eq!(format!("{token}"), "<redacted>");
    assert_eq!(format!("{token:?}"), "<redacted>");
    let row = ShareRow {
        share_id: "s".into(),
        state: ShareState::Live,
        account_id: 1,
        login: "l".into(),
        gist_id: None,
        created_at: "c".into(),
        title: "t".into(),
        pages: 1,
        key: "KEY-MATERIAL".into(),
    };
    assert!(!format!("{row:?}").contains("KEY-MATERIAL"));
    // A panic message built from these types carries no secret either.
    let panic = std::panic::catch_unwind(|| {
        panic!(
            "{:?} {}",
            Token::new(TOKEN.into()),
            Token::new(TOKEN.into())
        )
    })
    .unwrap_err();
    assert_no_secret(panic.downcast_ref::<String>().unwrap());
}

#[cfg(unix)]
#[test]
fn records_are_written_owner_only_through_an_atomic_rename() {
    use std::os::unix::fs::PermissionsExt;
    let mock = gist_mock(gist_store(), |_| resp(201, ""));
    let f = fixture(mock.endpoints());
    f.connect();
    // A pre-existing world-readable file is replaced by an owner-only one.
    std::fs::write(f.dir.path().join(SHARES_FILE), b"{\"v\":1,\"shares\":[]}").unwrap();
    std::fs::set_permissions(
        f.dir.path().join(SHARES_FILE),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    block_on(f.service.create_share(&f.prepared(&payload()), |_| true)).unwrap();
    for name in [ACCOUNT_FILE, SHARES_FILE] {
        let mode = std::fs::metadata(f.dir.path().join(name))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "{name}");
    }
    let leftovers: Vec<_> = std::fs::read_dir(f.dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    let shares_bytes = std::fs::read(f.dir.path().join(SHARES_FILE)).unwrap();
    let shares: serde_json::Value = serde_json::from_slice(&shares_bytes).unwrap();
    assert_eq!(shares["v"], 1);
    let row = &shares["shares"][0];
    // I-25 (docs/contracts/quick-share.md "Device records"): the measured sizes.
    eprintln!(
        "I-25 quick share: quick-shares.json {} B with one row ({} B row); quick-share-account.json {} B",
        shares_bytes.len(),
        serde_json::to_vec(row).unwrap().len(),
        std::fs::metadata(f.dir.path().join(ACCOUNT_FILE)).unwrap().len()
    );
    for field in [
        "share_id",
        "state",
        "account_id",
        "login",
        "gist_id",
        "created_at",
        "title",
        "pages",
        "key",
    ] {
        assert!(row.get(field).is_some(), "{field}");
    }
    let account: serde_json::Value =
        serde_json::from_slice(&std::fs::read(f.dir.path().join(ACCOUNT_FILE)).unwrap()).unwrap();
    assert_eq!(account["v"], 1);
    assert_eq!(account["account_id"], ACCOUNT_ID);
}

#[cfg(unix)]
#[test]
fn the_private_temporary_is_owner_only_from_creation() {
    // A umask of 0 would leave a later chmod's window open; the mode is set
    // when the file is created, so the temp is 0600 even then.
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secret.json");
    let old = unsafe { libc::umask(0) };
    let result = crate::device_io::atomic_write_private(&path, b"{}");
    unsafe { libc::umask(old) };
    result.unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

// ---- guards ----

const QUICK_SHARE_SOURCES: [(&str, &str); 3] = [
    ("mod.rs", include_str!("mod.rs")),
    ("github.rs", include_str!("github.rs")),
    ("records.rs", include_str!("records.rs")),
];
const FIVE: [&str; 5] = ["linux", "windows", "macos", "ios", "android"];

/// Every platform branch in quick share names all five shipped targets
/// (AGENTS.md §2): a `cfg` that names a target_os belongs to a group whose
/// branches together cover the five, with the rest a compile error.
#[test]
fn quick_share_platform_cfgs_name_all_five_targets() {
    let atomic = include_str!("../../../crates/tine-store/src/atomic_file.rs");
    let private = &atomic[atomic.find("pub(crate) fn atomic_write_private").unwrap()..];
    let private = &private[..private.find("\n}\n").unwrap()];
    for (name, source) in QUICK_SHARE_SOURCES
        .iter()
        .copied()
        .chain([("atomic_write_private", private)])
    {
        let named: std::collections::BTreeSet<&str> = FIVE
            .iter()
            .copied()
            .filter(|os| source.contains(&format!("target_os = \"{os}\"")))
            .collect();
        if named.is_empty() {
            continue;
        }
        assert_eq!(named.len(), 5, "{name} names only {named:?}");
        assert!(
            source.contains("#[cfg(not(any("),
            "{name} has no fallback for other targets"
        );
    }
    assert!(include_str!("github.rs").contains("compile_error!"));
}

/// DESIGN §3.3: Windows takes native-tls with the system proxy; the other four
/// targets take rustls (platform verifier, ring provider).
#[test]
fn quick_share_tls_is_declared_for_every_shipped_target() {
    let manifest = include_str!("../../Cargo.toml");
    let mut section = "";
    let mut lines = Vec::new();
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            section = line;
        } else if line.starts_with("reqwest") || line.starts_with("rustls ") {
            lines.push((section, line));
        }
    }
    assert_eq!(
        lines,
        vec![
            (
                "[target.'cfg(any(target_os = \"linux\", target_os = \"macos\", target_os = \"android\", target_os = \"ios\"))'.dependencies]",
                "reqwest = { version = \"0.13\", default-features = false, features = [\"rustls-no-provider\", \"system-proxy\"] }",
            ),
            (
                "[target.'cfg(any(target_os = \"linux\", target_os = \"macos\", target_os = \"android\", target_os = \"ios\"))'.dependencies]",
                "rustls = { version = \"0.23\", default-features = false, features = [\"ring\"] }",
            ),
            (
                "[target.'cfg(windows)'.dependencies]",
                "reqwest = { version = \"0.13\", default-features = false, features = [\"native-tls\", \"system-proxy\"] }",
            ),
        ]
    );
    let github = include_str!("github.rs");
    assert!(github
        .contains("#[cfg(target_os = \"windows\")]\n    {\n        builder.tls_backend_native()"));
    assert!(github.contains("builder.tls_backend_rustls()"));
    assert!(github.contains(".https_only(endpoints.https_only)") && Endpoints::github().https_only);
    assert!(github.contains("redirect::Policy::none()"));
}
