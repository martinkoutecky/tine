use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use tine_store::Store;

#[test]
fn a_killed_external_export_reopens_without_a_partial_site() {
    const CHILD: &str = "TINE_EXTERNAL_EXPORT_KILL_CHILD";
    if let Ok(base) = std::env::var(CHILD) {
        let base = std::path::PathBuf::from(base);
        let store = Store::open(&base.join("graph"), Default::default())
            .unwrap()
            .0;
        let _ = store.publish_site_external(&base.join("out"), "site", &mut |writer| {
            writer.write("index.html", b"partial")?;
            std::process::exit(71);
        });
        unreachable!();
    }
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let base = std::env::temp_dir().join(format!(
        "tine-export-kill-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(base.join("graph/pages")).unwrap();
    fs::create_dir_all(base.join("graph/journals")).unwrap();
    fs::create_dir_all(base.join("out")).unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("a_killed_external_export_reopens_without_a_partial_site")
        .env(CHILD, &base)
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(71));
    assert!(!base.join("out/site").exists());
    let store = Store::open(&base.join("graph"), Default::default())
        .unwrap()
        .0;
    let result = store
        .publish_site_external(&base.join("out"), "site", &mut |writer| {
            writer.write("index.html", b"complete")
        })
        .unwrap();
    assert_eq!(
        fs::read(result.site.join("index.html")).unwrap(),
        b"complete"
    );
    store.close();
}
