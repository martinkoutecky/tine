use super::*;
use serde_json::{json, Value};

fn config(profile: &str, mutant: &str) -> Config {
    Config {
        pages: 3,
        r1: profile == "R1" || profile == "all",
        weak: profile == "weak" || profile == "all",
        mutant: mutant.into(),
    }
}

// Evaluate the original scenario predicates, including intermediate expects.
// Unsupported syntax is a harness error, never a passing assertion.
fn eval(e: &Value, x: &State) -> Value {
    let tag = e[0].as_str().unwrap();
    match tag {
        "id" => {
            let n = e[1].as_str().unwrap();
            match n {
                "s" => json!(x.s),
                "g" => json!(x.g),
                "true" => json!(true),
                "false" => json!(false),
                "ok" | "guarantee" => json!(guarantee(x)),
                "canSwitch" => json!(can_switch(x)),
                "ABSENT" => json!(ABSENT),
                "NONE" => json!(NONE),
                "MUTANT" => json!(x.config.mutant),
                "PAGES" => json!((0..x.s.pages.len()).collect::<Vec<_>>()),
                "UNKNOWN" => json!(UNKNOWN),
                _ if n.starts_with('"') => json!(n.trim_matches('"')),
                _ => json!(n.parse::<i64>().expect("unsupported identifier")),
            }
        }
        "neg" => json!(-eval(&e[1], x).as_i64().unwrap()),
        "dot" => eval(&e[1], x)[e[2].as_str().unwrap()].clone(),
        "if" => eval(
            if eval(&e[1], x).as_bool().unwrap() {
                &e[2]
            } else {
                &e[3]
            },
            x,
        ),
        "bin" => {
            let a = eval(&e[2], x);
            let b = eval(&e[3], x);
            match e[1].as_str().unwrap() {
                "and" => json!(a.as_bool().unwrap() && b.as_bool().unwrap()),
                "or" => json!(a.as_bool().unwrap() || b.as_bool().unwrap()),
                "==" => json!(a == b),
                "!=" => json!(a != b),
                ">" => json!(a.as_i64().unwrap() > b.as_i64().unwrap()),
                "<" => json!(a.as_i64().unwrap() < b.as_i64().unwrap()),
                ">=" => json!(a.as_i64().unwrap() >= b.as_i64().unwrap()),
                "<=" => json!(a.as_i64().unwrap() <= b.as_i64().unwrap()),
                "+" => json!(a.as_i64().unwrap() + b.as_i64().unwrap()),
                "-" => json!(a.as_i64().unwrap() - b.as_i64().unwrap()),
                "->" => json!([a, b]),
                op => panic!("unsupported operator {op}"),
            }
        }
        "call" => {
            let f = &e[1];
            let args = e[2].as_array().unwrap();
            if f[0] == "id" && (f[1] == "Set" || f[1] == "Map") {
                let mut values: Vec<_> = args.iter().map(|e| eval(e, x)).collect();
                if f[1] == "Set" {
                    values.sort_by_key(|v| v.to_string());
                    values.dedup();
                }
                return json!(values);
            }
            if f[0] == "dot" {
                let method = f[2].as_str().unwrap();
                if method == "forall" {
                    let domain = eval(&f[1], x);
                    let lambda = &args[0];
                    assert_eq!(lambda[1], "=>");
                    return json!(domain.as_array().unwrap().iter().all(|v| {
                        fn substitute(e: &Value, name: &str, value: &Value) -> Value {
                            if e.is_array() && e[0] == "id" && e[1] == name {
                                return json!(["id", value.to_string()]);
                            }
                            match e {
                                Value::Array(a) => json!(a
                                    .iter()
                                    .map(|e| substitute(e, name, value))
                                    .collect::<Vec<_>>()),
                                _ => e.clone(),
                            }
                        }
                        eval(
                            &substitute(&lambda[3], lambda[2][1].as_str().unwrap(), v),
                            x,
                        )
                        .as_bool()
                        .unwrap()
                    }));
                }
                if method == "get" {
                    return eval(&f[1], x)[eval(&args[0], x).as_u64().unwrap() as usize].clone();
                }
                if method == "contains" && f[1][1] == "RACES" {
                    return json!(match eval(&args[0], x).as_str().unwrap() {
                        "R1" => x.config.r1,
                        "weak" => x.config.weak,
                        "crash" | "power" => true,
                        race => panic!("unknown race {race}"),
                    });
                }
                panic!("unsupported method {method}");
            }
            let a = eval(&args[0], x);
            match f[1].as_str().unwrap() {
                "not" => json!(!a.as_bool().unwrap()),
                "clean" => json!(clean(&serde_json::from_value(a).unwrap())),
                "dirty" => json!(dirty(&serde_json::from_value(a).unwrap())),
                "draftEntry" => json!(draft_entry(&serde_json::from_value(a).unwrap())),
                "opClean" => json!(op_clean(&x.s, eval(&args[1], x).as_u64().unwrap() as usize)),
                function => panic!("unsupported function {function}"),
            }
        }
        _ => panic!("unsupported AST {e}"),
    }
}

fn action(name: &str, args: &[Value]) -> Action {
    let p = || args[0].as_u64().unwrap() as usize;
    let v = |i: usize| args[i].as_i64().unwrap();
    let b = |i: usize| args[i].as_bool().unwrap_or_else(|| v(i) == 1);
    match name {
        "wOpen" => Action::WOpen(p()),
        "wEdit" => Action::WEdit(p(), v(1)),
        "wSend" => Action::WSend(p()),
        "wResolve" => Action::WResolve(p(), v(1)),
        "wDiscard" => Action::WDiscard(p()),
        "wOp" => Action::WOp(p(), v(1), v(2)),
        "wOpTo" => Action::WOpTo(p(), v(1) as usize, v(2), v(3)),
        "opDelete" => Action::OpDelete(p()),
        "flushDel" => Action::FlushDel(p()),
        "opRename" => Action::OpRename(
            p(),
            v(1) as usize,
            serde_json::from_value(args[2].clone()).unwrap(),
            args[3]
                .as_array()
                .unwrap()
                .iter()
                .map(|pair| {
                    (
                        pair[0].as_u64().unwrap() as usize,
                        pair[1].as_i64().unwrap(),
                    )
                })
                .collect(),
        ),
        "powerK" => Action::PowerK(serde_json::from_value(args[0].clone()).unwrap()),
        "powerKBits" => Action::PowerK((0..3).filter(|&i| b(i)).collect()),
        "opRenamePacked" => Action::OpRename(
            p(),
            v(1) as usize,
            (0..3)
                .filter(|&i| i != p() && i != v(1) as usize && b(2 + i))
                .collect(),
            (0..3).map(|i| (i, v(5 + i))).collect(),
        ),
        "opRenameRaw" => Action::OpRename(
            p(),
            v(1) as usize,
            (0..3).filter(|&i| b(2 + i)).collect(),
            (0..3).map(|i| (i, v(5 + i))).collect(),
        ),
        "wClose" => Action::WClose(p()),
        "wRecv" => Action::WRecv(p()),
        "deliverUp" => Action::DeliverUp(b(0)),
        "observe" => Action::Observe(p()),
        "flush" => Action::Flush(p()),
        "check" => Action::Check,
        "rename" => Action::Rename,
        "dirSync" => Action::DirSync(b(0)),
        "saveFail" => Action::SaveFail,
        "draftSync" => Action::DraftSync(p()),
        "switchReq" => Action::SwitchReq,
        "switchFin" => Action::SwitchFin,
        "extWrite" => Action::ExtWriteD(p(), v(1), true),
        "extWriteD" => Action::ExtWriteD(p(), v(1), b(2)),
        "crash" => Action::Crash,
        "windowCrash" => Action::WindowCrash,
        "power" => Action::Power(b(0), b(1)),
        "launch" => Action::Launch,
        _ => panic!("unknown action {name}"),
    }
}

fn program(ops: &[Value], x: &mut State) -> Result<(), &'static str> {
    for op in ops {
        match op[0].as_str().unwrap() {
            "init" => *x = init(x.config.clone()),
            "expect" => {
                if !eval(&op[1], x).as_bool().unwrap() {
                    return Err("assertion");
                }
            }
            "if" => program(
                op[if eval(&op[1], x).as_bool().unwrap() {
                    2
                } else {
                    3
                }]
                .as_array()
                .unwrap(),
                x,
            )?,
            "fail" => {
                let mut probe = x.clone();
                if program(op[1].as_array().unwrap(), &mut probe).is_ok() {
                    return Err("false");
                }
            }
            "action" => {
                let args: Vec<_> = op[2]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|e| eval(e, x))
                    .collect();
                *x = step(x, action(op[1].as_str().unwrap(), &args)).ok_or("disabled")?;
            }
            _ => panic!("unknown instruction {op}"),
        }
    }
    Ok(())
}

fn scenarios() -> Value {
    serde_json::from_str(include_str!("../../tests/fixtures/s2/scenarios.json")).unwrap()
}

#[test]
fn scenario_outcomes_equal_quint_in_all_profiles_and_mutants() {
    let fixture = scenarios();
    assert_eq!(fixture["model_sha256"], MODEL_SHA);
    assert_eq!(
        fixture["scenario_sha256"],
        "b446ab25e60e140c16ebd1bf4e73054e3de78928f0087ed712ed4151d2cdf530"
    );
    let scenarios = fixture["scenarios"].as_object().unwrap();
    assert_eq!(scenarios.len(), 120);
    assert_eq!(fixture["oracles"].as_array().unwrap().len(), 31);
    let mut comparisons = 0;
    for oracle in fixture["oracles"].as_array().unwrap() {
        let profile = oracle["profile"].as_str().unwrap();
        let mutant = oracle["mutant"].as_str().unwrap();
        for (name, ops) in scenarios {
            let mut x = init(config(profile, mutant));
            let result = program(ops.as_array().unwrap(), &mut x);
            let status = result.err().unwrap_or("pass");
            assert_eq!(
                status,
                oracle["outcomes"][name].as_str().unwrap(),
                "{profile}/{mutant}/{name}: bad={:?}",
                x.g.bad
            );
            comparisons += 1;
        }
    }
    assert_eq!(comparisons, 3720);
}

#[test]
#[cfg(test)]
fn every_declared_mutant_is_caught() {
    let fixture = scenarios();
    let mut count = 0;
    for oracle in fixture["oracles"].as_array().unwrap() {
        let mutant = oracle["mutant"].as_str().unwrap();
        if mutant == "none" {
            continue;
        }
        let mut x = init(config(oracle["profile"].as_str().unwrap(), mutant));
        let result = program(
            fixture["scenarios"][format!("m{mutant}")]
                .as_array()
                .unwrap(),
            &mut x,
        );
        assert_eq!(result, Err("assertion"), "{mutant}");
        // MDE's own scenario checks risk before the later power-loss suffix.
        // Run that suffix to demonstrate the actual A violation as well.
        if mutant == "MDE" {
            if x.s.drafts[0] != draft_entry(&x.s.pages[0]) {
                x = draft_sync(&x, 0).unwrap();
            }
            x = power(&x, false, true).unwrap();
        }
        if mutant != "MRN5" {
            assert!(!guarantee(&x), "{mutant} failed only a scenario predicate");
        }
        eprintln!(
            "{mutant}: caught; A={} B={} Bprime={} C={} G={} accepted={} bad={:?}",
            no_loss(&x),
            clause_b(&x),
            clause_b_prime(&x),
            clause_c(&x),
            clause_g(&x),
            accepted(&x),
            x.g.bad
        );
        count += 1;
    }
    assert_eq!(count, 27);
}

fn replay_traces(fixture: &Value) -> (usize, usize) {
    assert_eq!(fixture["model_sha256"], MODEL_SHA);
    let mut states = 0;
    let traces = fixture["traces"].as_array().unwrap();
    for (ti, trace) in traces.iter().enumerate() {
        let profile = trace["profile"].as_str().unwrap();
        let mutant = trace["mutant"].as_str().unwrap_or("none");
        let mut cfg = config(profile, mutant);
        cfg.pages = trace["pages"].as_u64().unwrap_or(3) as usize;
        let mut x = init(cfg);
        let mut expected = Value::Null;
        for (si, entry) in trace["states"].as_array().unwrap().iter().enumerate() {
            let recorded = if let Some(index) = entry["a"].as_u64() {
                &fixture["actions"][index as usize]
            } else {
                &entry["action"]
            };
            let name = recorded["name"].as_str().unwrap();
            if si == 0 {
                assert_eq!(name, "init");
            } else {
                x = step(&x, action(name, recorded["args"].as_array().unwrap()))
                    .unwrap_or_else(|| panic!("disabled {profile}/{ti}/{si}/{name}"));
            }
            if entry["state"].is_object() {
                expected = entry["state"].clone();
            } else {
                for change in entry["delta"].as_array().unwrap() {
                    let mut target = &mut expected;
                    let path = &fixture["paths"][change[0].as_u64().unwrap() as usize];
                    for key in path.as_array().unwrap() {
                        target = if let Some(key) = key.as_str() {
                            &mut target[key]
                        } else {
                            &mut target[key.as_u64().unwrap() as usize]
                        };
                    }
                    if change[2].as_bool() == Some(true) {
                        target
                            .as_array_mut()
                            .unwrap()
                            .extend(change[1].as_array().unwrap().iter().cloned());
                    } else {
                        *target = change[1].clone();
                    }
                }
            }
            // All Sys and Ghost fields, including inactive padding, queue order,
            // durable bytes, promises, seen/wrote/owed, and transition tags.
            assert_eq!(json!(x), expected, "{profile}/trace {ti}/step {si}/{name}");
            if mutant == "none" {
                assert!(guarantee(&x), "{profile}/{ti}/{si}");
            }
            let bad = expected["g"]["bad"].as_array().unwrap();
            let has = |tag: &str| bad.iter().any(|v| v.as_str() == Some(tag));
            assert_eq!(clause_b(&x), !has("B-unsaved-replaced"));
            assert_eq!(clause_c(&x), !has("C-overwrote-external"));
            assert_eq!(clause_g(&x), !has("G-overwrote-unseen"));
            assert_eq!(
                clause_b_prime(&x),
                !has("B'-window-replaced") && !has("B'-submit-not-taken") && accepted(&x)
            );
            if entry["predicates"].is_object() {
                assert_eq!(no_loss(&x), entry["predicates"]["loss"].as_bool().unwrap());
                assert_eq!(
                    accepted(&x),
                    entry["predicates"]["accepted"].as_bool().unwrap()
                );
                assert_eq!(within(&x), entry["predicates"]["within"].as_bool().unwrap());
                assert_eq!(trashed(&x), entry["predicates"]["trash"].as_bool().unwrap());
                assert_eq!(
                    guarantee(&x),
                    entry["predicates"]["guarantee"].as_bool().unwrap()
                );
            }
            if let Some(expected) = entry["within"].as_bool() {
                assert_eq!(within(&x), expected);
            }
            if let Some(choices) = entry["enabled"].as_array() {
                for (ci, allowed) in choices.iter().enumerate() {
                    let choice = &fixture["choices"][ci];
                    let name = choice["name"].as_str().unwrap();
                    let selected = action(name, choice["args"].as_array().unwrap());
                    assert_eq!(
                        step(&x, selected).is_some(),
                        allowed.as_bool().unwrap(),
                        "{profile}/trace {ti}/step {si}/guard {choice}"
                    );
                }
            }
            states += 1;
        }
    }
    (traces.len(), states)
}

#[test]
fn committed_random_traces_match_every_state() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/s2/traces.json")).unwrap();
    let (traces, states) = replay_traces(&fixture);
    assert_eq!(traces, 32);
    assert!(states >= traces);
}

#[test]
fn targeted_quint_witnesses_match_states_and_disabled_choices() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/s2/witnesses.json")).unwrap();
    let (traces, states) = replay_traces(&fixture);
    assert!(traces > 0 && states > traces);
}

#[test]
#[cfg(test)]
fn generated_random_traces_match_every_state() {
    // Optional larger local corpus; normal CI always runs the committed sample.
    let Ok(path) = std::env::var("S3_TRACE_FIXTURE") else {
        return;
    };
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let (traces, states) = replay_traces(&fixture);
    assert!(traces >= 256);
    eprintln!("s3 full corpus: {traces} traces / {states} full states matched");
}
