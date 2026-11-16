//! C2/C3 escape-only probes of the current S4 PagePlan::apply/apply_move
//! primitives. Transactions remain oracle = false; these are not refinement.
use super::*;

#[test]
fn m3_transaction_apply_and_apply_move_c2_c3_escape_only() {
    let mut count = 0;
    for area in ["pages", "assets"] {
        for operation in ["create", "replace", "move", "rewrite-move"] {
            for source_parent in [false, true] {
                for swap in [false, true] {
                    if swap && cfg!(target_os = "windows") {
                        continue;
                    }
                    let dir = match std::env::var_os("TINE_HOST_FS_ROOT") {
                        Some(base) => tempfile::tempdir_in(base).unwrap(),
                        None => tempfile::tempdir().unwrap(),
                    };
                    let root = dir.path().join("graph");
                    let outside = dir.path().join("outside");
                    fs::create_dir_all(root.join(format!("{area}/source"))).unwrap();
                    fs::create_dir_all(root.join(format!("{area}/target"))).unwrap();
                    fs::create_dir(&outside).unwrap();
                    let src = FileId::from(format!("{area}/source/a.md"));
                    let dst = FileId::from(format!("{area}/target/b.md"));
                    let old = b"- old\n";
                    let new = if operation == "move" {
                        old.as_slice()
                    } else {
                        b"- new\n"
                    };
                    if operation != "create" {
                        fs::write(root.join(src.as_str()), old).unwrap();
                    }
                    let store = Store::open(&root, Default::default()).unwrap().0;
                    // Freeze the prepared bytes before the environment step,
                    // then call the real private apply boundaries directly.
                    let moving = operation.contains("move");
                    let plan = Prepared {
                        src: if operation == "create" {
                            dst.clone()
                        } else {
                            src.clone()
                        },
                        dst: moving.then(|| dst.clone()),
                        old: (operation != "create").then(|| old.to_vec()),
                        new: Some(new.to_vec()),
                        saved_page: None,
                        opaque_rev: None,
                    };
                    let mut undo = Undo {
                        kind: UndoKind::Replace,
                        src: plan.src.clone(),
                        dst: plan.dst.clone(),
                        trash: None,
                        old: plan.old.clone(),
                        new: None,
                        new_rev: None,
                        opaque_rev: None,
                        created: false,
                        moved: false,
                    };
                    let parent = root.join(format!(
                        "{area}/{}",
                        if source_parent { "source" } else { "target" }
                    ));
                    let moved_parent = root.join(format!("{area}/moved"));
                    fs::rename(&parent, &moved_parent).unwrap();
                    if swap {
                        symlink(&outside, &parent);
                    }
                    let tx = store.page_plan(None);
                    let result = if moving {
                        tx.apply_move(&plan, &mut undo, 0)
                    } else {
                        let mut step = if operation == "create" {
                            Step::Create {
                                file: dst.clone(),
                                content: Content::Bytes(new.to_vec()),
                            }
                        } else {
                            Step::Replace {
                                file: src.clone(),
                                expected: FileRev::from_bytes(old),
                                bytes: new.to_vec(),
                            }
                        };
                        tx.apply(
                            0,
                            &mut step,
                            &plan,
                            &mut undo,
                            &mut Vec::new(),
                            &HashSet::new(),
                        )
                    };
                    assert_eq!(
                        fs::read_dir(&outside).unwrap().count(),
                        0,
                        "oracle=false: {area}/{operation}/source={source_parent}/C2={swap}"
                    );
                    if swap
                        && ((source_parent && operation != "create")
                            || (!source_parent && (moving || operation == "create")))
                    {
                        assert!(result.is_err(), "C2 checked target must refuse");
                    }
                    if source_parent && operation != "create" {
                        assert_eq!(fs::read(moved_parent.join("a.md")).unwrap(), old);
                        assert!(
                            result.is_err(),
                            "C3 stale source must not be recreated as current"
                        );
                    }
                    count += 1;
                }
            }
        }
    }
    assert_eq!(count, if cfg!(target_os = "windows") { 16 } else { 32 });
}

#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "android"
))]
fn symlink(target: &Path, path: &Path) {
    std::os::unix::fs::symlink(target, path).unwrap();
}

#[cfg(target_os = "windows")]
fn symlink(_: &Path, _: &Path) {
    panic!("C2: NATIVE_MODELFS_ONLY, no unprivileged Windows junction fixture");
}
