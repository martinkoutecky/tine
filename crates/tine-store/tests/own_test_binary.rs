//! Tests that start their own binary again survive a rebuild that replaces it
//! (flakes-1110: two owner tests failed `NotFound` at spawn in a gate whose
//! cargo target another build was relinking). `tine_store::test_binary`
//! owns the answer; this pins its outcome and that every test asks it.

use std::path::Path;

/// The child's binary file is replaced, as a relink replaces a test binary,
/// before the child starts the test binary again: the grandchild still runs.
#[cfg(any(target_os = "linux", target_os = "android"))]
#[test]
fn a_test_binary_replaced_while_running_still_starts_its_child() {
    use std::process::Command;
    const STAGE: &str = "TINE_OWN_TEST_BINARY_STAGE";
    const LINK: &str = "TINE_OWN_TEST_BINARY_LINK";
    const NAME: &str =
        "own_test_binary::a_test_binary_replaced_while_running_still_starts_its_child";
    let rerun = |stage: &str| {
        let mut command = Command::new(tine_store::test_binary::own_test_binary());
        command
            .args(["--exact", NAME, "--nocapture", "--test-threads=1"])
            .env(STAGE, stage);
        command
    };
    match std::env::var(STAGE).as_deref() {
        // Started from the replaced binary's image.
        Ok("grandchild") => {
            println!("TINE-OWN-TEST-BINARY-RAN");
            return;
        }
        Ok("child") => {
            // This process runs from `link`; put other bytes there.
            let link = std::path::PathBuf::from(std::env::var_os(LINK).unwrap());
            let new = link.with_extension("new");
            std::fs::write(&new, b"not a test binary").unwrap();
            std::fs::rename(&new, &link).unwrap();
            let output = rerun("grandchild").output().unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success() && stdout.contains("TINE-OWN-TEST-BINARY-RAN"),
                "{stdout}"
            );
            println!("TINE-OWN-TEST-BINARY-CHILD-OK");
            return;
        }
        _ => {}
    }
    let _integration_guard = crate::integration_lock();
    // A second name for the running binary that the child may replace; a
    // hard link costs no copy when the temporary directory shares the
    // binary's file system, a copy is the fallback.
    let dir = tempfile::tempdir().unwrap();
    let link = dir.path().join("integration");
    let linked = std::env::current_exe()
        .and_then(|exe| std::fs::hard_link(exe, &link))
        .is_ok();
    if !linked {
        std::fs::copy("/proc/self/exe", &link).unwrap();
    }
    let output = Command::new(&link)
        .args(["--exact", NAME, "--nocapture", "--test-threads=1"])
        .env(STAGE, "child")
        .env(LINK, &link)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("TINE-OWN-TEST-BINARY-CHILD-OK"),
        "the child could not start the test binary again after it was replaced:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Every test that re-executes itself spawns `own_test_binary()`; the other
/// `current_exe()` callers are the helper, the test above and the app's own.
#[test]
fn no_test_spawns_current_exe_itself() {
    let _integration_guard = crate::integration_lock();
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    // Each file that may name `current_exe()`, with its count.
    let allowed = [
        ("crates/tine-store/src/test_binary.rs", 1),
        // Hard-links the running binary above; it spawns the link.
        ("crates/tine-store/tests/own_test_binary.rs", 1),
        ("src-tauri/src/lib.rs", 1),
        ("src-tauri/src/linux_window_identity.rs", 1),
        ("src-tauri/src/owner/client.rs", 1),
        ("src-tauri/src/portable.rs", 1),
    ];
    let mut roots = vec![
        workspace.join("src-tauri/src"),
        workspace.join("src-tauri/tests"),
    ];
    for entry in std::fs::read_dir(workspace.join("crates")).unwrap() {
        let crate_dir = entry.unwrap().path();
        roots.extend([crate_dir.join("src"), crate_dir.join("tests")]);
    }
    let needle = ["current_exe", "()"].concat();
    let mut found = Vec::new();
    let mut stack: Vec<_> = roots.into_iter().filter(|r| r.is_dir()).collect();
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let count = std::fs::read_to_string(&path)
                    .unwrap()
                    .lines()
                    .filter(|line| line.split("//").next().unwrap().contains(&needle))
                    .count();
                if count > 0 {
                    let relative = path.strip_prefix(&workspace).unwrap();
                    found.push((relative.to_string_lossy().replace('\\', "/"), count));
                }
            }
        }
    }
    found.sort();
    let allowed: Vec<_> = allowed.iter().map(|(f, n)| (f.to_string(), *n)).collect();
    assert_eq!(
        found, allowed,
        "a test that starts its own binary again spawns \
         tine_store::test_binary::own_test_binary(), not `current_exe`"
    );
}

/// AGENTS §2: the helper's two cfg lists name all five shipped targets
/// between them, so no target compiles without it.
#[test]
fn own_test_binary_names_all_five_targets() {
    let _integration_guard = crate::integration_lock();
    let source = include_str!("../src/test_binary.rs");
    for cfg in [
        "#[cfg(any(target_os = \"linux\", target_os = \"android\"))]\npub fn own_test_binary()",
        "#[cfg(any(target_os = \"macos\", target_os = \"ios\", target_os = \"windows\"))]\npub fn own_test_binary()",
    ] {
        assert!(source.contains(cfg), "{cfg}");
    }
}
