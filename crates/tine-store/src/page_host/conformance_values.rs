//! Model-value encoding for the conformance driver: harness page keys,
//! the model's integer text labels, action argument decoding, and the JSON
//! set/promise updates the abstract state uses. Split from `conformance.rs`.
use super::*;

pub(super) const ABSENT: i64 = -1;
pub(super) const NONE: i64 = -2;
pub(super) const UNKNOWN: i64 = -3;

pub(super) fn key(p: usize) -> String {
    format!("{p:04}.md")
}

/// Pages a model action names through the host: the window's own page,
/// an operation's endpoints and referrers. External writes, faults and
/// whole-host actions name none.
pub(super) fn named(name: &str, args: &[Value]) -> Vec<usize> {
    let at = |i: usize| args[i].as_u64().unwrap() as usize;
    let flag = |i: usize| {
        args[i]
            .as_bool()
            .unwrap_or_else(|| args[i].as_i64() == Some(1))
    };
    match name {
        "wOpen" | "wSend" | "wDiscard" | "wClose" | "wRecv" | "flush" | "observe" | "draftSync"
        | "wEdit" | "wResolve" | "opDelete" | "flushDel" | "load" => vec![at(0)],
        "wOp" => vec![at(0), 1 - at(0)],
        "wOpTo" => vec![at(0), at(1)],
        "opRename" => {
            let refs: Vec<usize> = serde_json::from_value(args[2].clone()).unwrap();
            [at(0), at(1)].into_iter().chain(refs).collect()
        }
        "opRenameRaw" | "opRenamePacked" => [at(0), at(1)]
            .into_iter()
            .chain((0..3).filter(|&i| flag(2 + i)))
            .collect(),
        "opApply" | "opApplyPacked" => apply_args(name, args).0.into_keys().collect(),
        _ => vec![],
    }
}

/// An `opApply`'s arguments, in either encoding: the changed pages' new
/// texts, the planned (expected) text of every page, then (dst, src).
pub(super) type ApplyArgs = (BTreeMap<usize, i64>, BTreeMap<usize, i64>, i64, i64);

pub(super) fn apply_args(name: &str, args: &[Value]) -> ApplyArgs {
    let v = |i: usize| args[i].as_i64().unwrap();
    if name == "opApplyPacked" {
        let b = |i: usize| args[i].as_bool().unwrap_or_else(|| v(i) == 1);
        let eff = (0..3).filter(|&i| b(i)).map(|i| (i, v(3 + i))).collect();
        return (eff, (0..3).map(|i| (i, v(6 + i))).collect(), v(9), v(10));
    }
    let map = |i: usize| -> BTreeMap<usize, i64> {
        (args[i].as_array().unwrap().iter())
            .map(|pair| {
                (
                    pair[0].as_u64().unwrap() as usize,
                    pair[1].as_i64().unwrap(),
                )
            })
            .collect()
    };
    (map(0), map(1), v(2), v(3))
}

/// The model page a harness key names.
pub(super) fn index(key: &str) -> usize {
    key.trim_end_matches(".md").parse().unwrap()
}

pub(super) fn text(label: i64) -> Text {
    (label != ABSENT).then(|| Arc::from(label.to_string().into_bytes()))
}

pub(super) fn label(bytes: &Text) -> i64 {
    bytes
        .as_ref()
        .map_or(ABSENT, |b| std::str::from_utf8(b).unwrap().parse().unwrap())
}

pub(super) fn base(base: &Base) -> i64 {
    match base {
        Base::Known(bytes) => label(bytes),
        Base::Unknown => UNKNOWN,
    }
}

pub(super) fn insert(set: &mut Value, value: Value) {
    let mut values = set.as_array().unwrap().clone();
    if !values.contains(&value) {
        values.push(value);
    }
    // All set values here are numbers, pairs or tags. Use numeric order for
    // pairs so crossing the diagnostic version bounds remains correct.
    values.sort_by(|a, b| match (a, b) {
        (Value::Number(a), Value::Number(b)) => a.as_i64().cmp(&b.as_i64()),
        (Value::Array(a), Value::Array(b)) => a
            .iter()
            .map(|v| v.as_i64().unwrap())
            .cmp(b.iter().map(|v| v.as_i64().unwrap())),
        _ => a.to_string().cmp(&b.to_string()),
    });
    *set = json!(values);
}

pub(super) fn promise(pr: &mut Value, bytes: i64, ver: i64, saved: bool, ep: u64) {
    let old = pr["ver"].as_i64().unwrap();
    if ver > old || (ver == old && pr["on"] == true && saved && pr["saved"] == false) {
        *pr = json!({"on":true,"bytes":bytes,"ver":ver,"saved":saved,"ep":ep});
    }
}
