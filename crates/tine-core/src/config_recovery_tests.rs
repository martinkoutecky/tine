//! EDN model R4b witnesses: where local config reading resumes after a value the reader
//! refuses, and the occurrence rule that keeps any refusal from exposing a hidden page.
//! The witnesses and outcomes are the round-2 model review's table
//! (`model-review-v3.3.md`) with its amendments as model v3.4 settles them.

use super::Config;

/// The seven root spellings of `config_root_spellings_recover_the_same_entries`.
const ROOTS: [(&str, &str); 7] = [
    ("", ""),
    ("#:demo", "_/"),
    ("^:meta ", ""),
    ("#^:meta ", ""),
    ("^:meta #:demo", "_/"),
    ("#^:meta #:demo", "_/"),
    ("^:meta #^{:note true} #:demo", "_/"),
];
const NEWLINES: [&str; 3] = ["\n", "\r\n", "\r"];
const SECRET: &str = "pages/Secret.md";

#[derive(Clone, Copy, PartialEq, Debug)]
enum Outcome {
    /// `Secret` is hidden and every other page visible.
    Keep,
    /// The config fails closed: every page is hidden.
    Close,
    /// The model allows either; `Secret` is never exposed.
    Either,
}
use Outcome::{Close, Either, Keep};

/// Upstream reads these values to their end and admits them: extent controls.
const ADMITTED: &[&str] = &[
    "#_ #^:m [] :after",
    "#_ #_ 1 2 :after",
    "#uuid #_ 1 \"x\"",
    "# inst \"2020-01-01\"",
    "#; comment\ninst \"2020-01-01\"",
    "# ^:m inst \"2020-01-01\"",
    "# #_ 0 inst \"2020-01-01\"",
    "#_ #:demo ,{:_/hidden []} :after",
    "#_ (1#{} 2) :after",
    r"#_ (a^:m [] \; 1:kw) :after",
    "##Inf",
    "## Inf",
];

/// A damaged value of `:bad`, with the outcome when it comes before the exclusion and after it.
const DAMAGED: &[(&str, Outcome, Outcome)] = &[
    // `##` reads one operand; every complete operand but NaN/Inf/-Inf is refused (06deeae37 #2).
    ("##unknown", Keep, Keep),
    ("##nil", Keep, Keep),
    ("##true", Keep, Keep),
    ("##false", Keep, Keep),
    ("##1", Keep, Keep),
    ("##:kw", Keep, Keep),
    ("##\"s\"", Keep, Keep),
    (r"##\c", Keep, Keep),
    ("##[]", Keep, Keep),
    ("##()", Keep, Keep),
    ("##{}", Keep, Keep),
    ("###{}", Keep, Keep),
    // The `#` fallback reads a tag and an object, then refuses (b21ad7a3a #2).
    ("#unknown []", Keep, Keep),
    ("#1 []", Keep, Keep),
    ("#nil []", Keep, Keep),
    ("#false []", Keep, Keep),
    ("#\"s\" []", Keep, Keep),
    (r"#\x []", Keep, Keep),
    ("#[1] []", Keep, Keep),
    ("#(x) []", Keep, Keep),
    ("#=x []", Keep, Keep),
    ("#'x []", Keep, Keep),
    ("#? (:cljs [])", Keep, Keep),
    // Completion carries outward through the final operand of `##`.
    ("## #1 []", Keep, Keep),
    ("## #nil []", Keep, Keep),
    ("## #false []", Keep, Keep),
    ("## #\"s\" []", Keep, Keep),
    (r"## #\x []", Keep, Keep),
    ("## #[1] []", Keep, Keep),
    ("## #(x) []", Keep, Keep),
    ("## #=x []", Keep, Keep),
    ("## #'x []", Keep, Keep),
    ("## #? (:cljs [])", Keep, Keep),
    // A character token read to its end and refused.
    (r"\malformed", Keep, Keep),
    // A completed discard stays trivia: `1` is the value.
    ("#_ #inst \"2020-01-01\" 1", Keep, Keep),
    // A set ends at `}`; the leftover `[]` shifts the pairing.
    ("#{1} []", Close, Keep),
    ("## #{1} []", Close, Keep),
    // The fallback's object is the next form: the exclusion key, or a closer it cannot read.
    ("#()", Close, Keep),
    ("#\"r\"", Close, Keep),
    ("#'x", Close, Keep),
    ("## #()", Close, Keep),
    ("## #\"r\"", Close, Keep),
    ("## #'x", Close, Keep),
    ("#() [] 1", Close, Keep),
    // Upstream throws inside these: no extent, reading stops.
    ("#<x>", Close, Keep),
    ("#<", Close, Keep),
    ("#< ", Close, Keep),
    ("#?@ (:cljs [])", Close, Keep),
    ("## #<x>", Close, Keep),
    ("## #?@ (:cljs [])", Close, Keep),
    ("#:kw []", Close, Keep),
    ("#:demo []", Close, Keep),
    ("#:demo #_ [] {:x 1}", Close, Keep),
    ("#:demo ;comment\n{:x 1}", Close, Keep),
    ("#:demo", Close, Keep),
    ("#:bad/ns{}", Close, Keep),
    ("#:/bad {}", Close, Keep),
    ("#:bad: {}", Close, Keep),
    ("#:{}", Close, Keep),
    ("## #:kw []", Close, Keep),
    ("## #:demo []", Close, Keep),
    ("## #:bad/ns{}", Close, Keep),
    ("## #:/bad {}", Close, Keep),
    ("## #:bad: {}", Close, Keep),
    ("## #:{}", Close, Keep),
    ("[) ]", Close, Keep),
    ("\"\\u\"", Close, Keep),
    // An invalid escape throws inside the string (escape-char): no extent.
    ("\"bad\\q\"", Close, Keep),
    ("\"\\u12\"", Close, Keep),
    // `#()` nested in a collection, metadata, a tag, a discard or `##` (b21ad7a3a #2).
    ("[#()]", Close, Keep),
    ("[#() []]", Either, Keep),
    ("(#())", Close, Keep),
    ("{:x #()}", Close, Keep),
    ("#{#() []}", Either, Keep),
    ("^:m #()", Close, Keep),
    ("^:m #() []", Either, Keep),
    ("#^:m #()", Close, Keep),
    ("#^:m #() []", Either, Keep),
    ("#x #()", Close, Keep),
    ("#x #() []", Either, Keep),
    ("#_ #()", Close, Keep),
    ("#_ #() []", Either, Keep),
    ("## ## #()", Close, Keep),
    ("## ## #() []", Either, Keep),
];

/// Whole roots, written for the plain spelling: `{K}` is the effective exclusion key,
/// `{H}` the exclusion entry and `{N}` the line ending.
const ROOTED: &[(&str, Outcome)] = &[
    ("{{H}{N}:bad #<x>}", Keep),
    ("{:bad #<x>{N}{H}}", Close),
    ("{:bad #:demo {H}}", Close),
    ("{{H}{N}:bad #:demo ; {K}\n []}", Keep),
    ("{:bad \"\\u\" {H}}", Close),
    ("{:bad #{1} []{N}{H}}", Close),
    ("{:bad #(){N}{H}}", Close),
    ("{{K} []{N}{H}}", Close),
    ("{:bad ## #1 []{N}{H}}", Keep),
    ("{:bad #_ #inst \"2020-01-01\" 1{N}{H}}", Keep),
    ("{{H}{N}:note {K}{N}:bad #unknown []}", Close),
    ("{:bad #() [] 1{N}{H}}", Close),
    ("{{H}{N}:bad #() [] 1}", Keep),
    ("{:bad [) ]{N}{H}}", Close),
    ("{{K} #unknown []}", Close),
    ("{{K} [\"pages/Secret.md\"", Close),
    ("{{K} [\"pages/Secret.md\"]", Keep),
    ("{{H}{N}:bad", Keep),
    ("{{H}{N}:bad{N}", Keep),
    ("{:bad #<x> :note \\;{K} [\"pages/Secret.md\"]}", Close),
    ("{{H}{N}:bad #<x>{N}:note x#! {K} []}", Close),
    ("{{H}{N}:bad #<x>{N}:note 1#! {K} []\n}", Either),
    ("{:bad \"unfinished {K} [", Close),
    ("{{H}{N}:bad #<x>}{N}; {K}", Keep),
    ("{{H}{N}:bad #<x>}{N}; {K}{N}", Keep),
    ("{{H}{N}:bad #<x>}{N}#! {K}{N}", Keep),
    ("{{H}{N}:bad #<x>{N}:note \\c{K}}", Keep),
    ("{{H}{N}:bad #<x>{N}:note #?@{K}}", Close),
    ("{{H}{N}:bad #<x>{N}:note #<{K}}", Close),
    ("{{H}{N}:bad #?@{K} []}", Close),
    ("{:bad #?@{K} [\"pages/Secret.md\"]}", Close),
    // Comments after the exclusion: a CR alone does not end them.
    ("{{H}{N}:bad #unknown []{N}; trailing {K}{N}}", Keep),
    ("{{H}{N}:bad #unknown []{N}#! trailing {K}{N}}", Keep),
    ("{{H}} #unknown []", Keep),
    ("{{H}} #_ #_ 1", Keep),
];

fn outcome(source: &str) -> Result<Outcome, String> {
    let cfg = Config::parse(source);
    if cfg.hidden_parse_failed_closed {
        Ok(Close)
    } else if cfg.hidden == [SECRET] {
        Ok(Keep)
    } else {
        Err(format!("EXPOSED hidden={:?}", cfg.hidden))
    }
}

fn check(source: &str, expected: Outcome, failures: &mut Vec<String>) {
    match outcome(source) {
        Ok(got) if expected == Either || got == expected => (),
        Ok(got) => failures.push(format!("{source:?}: expected {expected:?}, got {got:?}")),
        Err(exposed) => failures.push(format!("{source:?}: {exposed}")),
    }
}

fn report(failures: &[String]) {
    assert!(
        failures.is_empty(),
        "{} R4b witness failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn a_refused_value_resumes_at_its_upstream_extent_and_never_exposes_a_page() {
    let mut failures = Vec::new();
    for (prefix, ns) in ROOTS {
        let hidden = format!(r#":{ns}hidden ["{SECRET}"]"#);
        let pages = format!(r#":{ns}pages-directory "notes""#);
        let namespaced = if ns.is_empty() {
            ""
        } else {
            r#":hidden [] :pages-directory "namespace-only""#
        };
        let cases = ADMITTED
            .iter()
            .map(|value| (*value, Keep, Keep, true))
            .chain(DAMAGED.iter().map(|&(value, b, a)| (value, b, a, false)));
        for (value, before, after, admitted) in cases {
            let bad = format!(":bad {value}");
            for (first, second, expected) in [(&bad, &hidden, before), (&hidden, &bad, after)] {
                for nl in NEWLINES {
                    let source = format!(
                        "{prefix}{{{nl}{namespaced}{nl}{first}{nl}{second}{nl}{pages}{nl}}}"
                    );
                    check(&source, expected, &mut failures);
                    let strict = Config::parse_for_publication(&source);
                    if admitted != strict.is_some() {
                        failures.push(format!("{source:?}: strict admission {}", strict.is_some()));
                    }
                }
            }
        }
    }
    report(&failures);
}

#[test]
fn whole_root_witnesses_follow_the_occurrence_rule_under_every_root_spelling() {
    let mut failures = Vec::new();
    for (prefix, ns) in ROOTS {
        let key = format!(":{ns}hidden");
        let hidden = format!(r#"{key} ["{SECRET}"]"#);
        for &(template, expected) in ROOTED {
            for nl in NEWLINES {
                let source = template
                    .replace("{H}", &hidden)
                    .replace("{K}", &key)
                    .replace("{N}", nl);
                let source = format!("{prefix}{source}");
                check(&source, expected, &mut failures);
                if Config::parse_for_publication(&source).is_some_and(|cfg| cfg.hidden != [SECRET])
                {
                    failures.push(format!(
                        "{source:?}: strict admission without the exclusion"
                    ));
                }
            }
        }
    }
    report(&failures);
}

#[test]
fn damage_without_an_exclusion_keyword_neither_hides_nor_closes() {
    for source in [
        "#_ #_ 1",
        "{:bad }",
        "{:bad \\",
        "{:bad #<x> :note \\c:hidden [\"pages/Secret.md\"]}",
        "{:bad #<x> :note \":hidden\"}",
        "{:bad #<x> :hidden-other [] :hidden? []}",
        "{:pages-directory \"notes\" :bad #unknown [] :other ; :hidden\n 1}",
    ] {
        let cfg = Config::parse(source);
        assert!(!cfg.hidden_parse_failed_closed, "{source:?}");
        assert!(cfg.hidden.is_empty(), "{source:?}");
    }
    // Entries before a stop are admitted settings.
    let cfg = Config::parse("{:pages-directory \"notes\" :bad #<x> :journals-directory \"j\"}");
    assert_eq!(cfg.pages_dir, "notes");
}
