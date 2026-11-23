use super::*;

#[test]
fn duplicate_detection_uses_values_and_ignores_authored_spelling() {
    for source in [
        r#"#{:k :k}"#,
        r#"{[1 2] :a (1 2) :b}"#,
        r#"#{^:private [1] (1.0)}"#,
        r#"#{#queue [1] [1.0]}"#,
        r#"#{#{1 2} #{2 1}}"#,
        r#"{{:x 1 :y 2} :a {:y 2 :x 1} :b}"#,
        r#"#{1N 1.0M}"#,
        r#"#{0x12 18}"#,
        r#"#{0x10000000000000801 18446744073709555712}"#,
        r#"#{022 18.0M}"#,
        r#"{2r10 0 4/2 1}"#,
        r#"#{\o101 \u0041}"#,
        r#"#{#inst "2020-02-29T00:30+01:00" #inst "2020-02-28T23:30Z"}"#,
        r#"#{#uuid "ABC" #uuid "abc"}"#,
        r#"#:ns{:a 1 :ns/a 2}"#,
        r#"#{#:ns{:_/a 1 b 2} {:a 1 ns/b 2}}"#,
        r#"#{\a "\u0061"}"#,
        r#"#{#_ {:x 1 :x 2} :public}"#,
    ] {
        assert_eq!(parse_strict(source), None, "{source}");
    }
    assert_eq!(parse_strict("#{1 #_ 1 2}"), parse_strict("#{1 2}"));
    let distinct_js = parse_strict("#{#js [1] #js [1] [1]}");
    assert!(matches!(distinct_js, Some(Edn::Set(items)) if items.len() == 3));
    assert!(parse_strict("#{0x10000000000000801 18446744073709551616}").is_some());
}

#[test]
fn reader_validates_restored_literals_and_decodes_their_values() {
    for source in [
        r"\u+000",
        r"\u+041",
        r"\u+FFF",
        r"\uD800",
        r"\u00000",
        r"\o8",
        r"\o0000",
        r"\o+01",
        "08",
        "0x+",
        "2r2",
        "2r",
        "37r1",
        "1.1N",
        "1/-2",
        r#"#inst "2025-02-29""#,
        r#"#inst "2024-12-31T23:58:60Z""#,
        r#"#inst "2024-01T12Z""#,
        r#"#inst "2024-01-02T12:34.5Z""#,
        r#"#inst "2024-01-02T12:34:56.""#,
        "#inst :value",
        "#js (:a)",
        "#js {1 :a}",
        "#uuid :bad",
        "#:ns #{:a}",
        "#:ns/name{:a 1}",
        "#:ns{:a}",
    ] {
        assert_eq!(parse_strict(source), None, "{source}");
    }
    for (source, expected) in [
        ("0x12N", 18.0),
        ("-0X12", -18.0),
        ("+022", 18.0),
        ("00N", 0.0),
        ("2R10z", 2.0),
        ("16r0x12", 18.0),
        ("36rN", 23.0),
        ("+1/2", 0.5),
        ("01M", 1.0),
        ("08.5", 8.5),
        ("01e2", 100.0),
        ("2.e-1", 0.2),
        ("1/0", f64::INFINITY),
    ] {
        assert_eq!(
            number(source).and_then(|value| value.as_f64()),
            Some(expected),
            "{source}"
        );
    }
    assert_eq!(
        parse_strict("#:ns {:a 1 b 2 :_/c 3 :other/d 4}"),
        parse_strict("{:ns/a 1 ns/b 2 :c 3 :other/d 4}")
    );
    assert_eq!(parse_strict(r"\o400"), Some(Edn::Str("Ā".into())));
    for (left, right) in [
        ("2024Z", "2024-01-01T00:00:00.000+00:00"),
        ("2024-01:00", "2024-01-01T01Z"),
        ("2024-12-31T23:59:60Z", "2025Z"),
        ("2024-02-29T23:59:59.123456Z", "2024-02-29T23:59:59.123Z"),
    ] {
        let value = instant(left).expect(left);
        assert_eq!(instant(right), Some(value));
    }
}

#[test]
fn unsupported_dispatch_and_invalid_metadata_refuse_the_whole_form() {
    for source in [
        "[#unknown :value :public]",
        "[#_ #unknown :value :public]",
        "[#?(:cljs :private) :public]",
        "[^:meta :keyword]",
        "[^true []]",
        "[bad/ns/name]",
        "[#uuid :not-a-string]",
    ] {
        assert_eq!(parse_strict(source), None, "{source}");
    }
    assert_eq!(
        parse_strict("#^:private ^{:doc :private} [1 #_ ^:private [] 2]"),
        parse_strict("[1 2]")
    );
}

#[test]
fn reader_discards_preserve_collection_membership_and_limits() {
    assert_eq!(parse_strict("{#_ :ignored :a 1}"), parse_strict("{:a 1}"));
    assert_eq!(
        parse_strict("#_ [unused] {:a #_ 0 1}"),
        parse_strict("{:a 1}")
    );
    assert_eq!(parse_strict("[#_ 0 1 #_ 2]"), parse_strict("[1]"));
    assert_eq!(parse_strict("(1 #_ 2)"), parse_strict("(1)"));
    assert_eq!(parse_strict("{:a #_}"), None);
    assert_eq!(
        parse_strict(&format!("{}nil", "#_ ".repeat(MAX_DEPTH + 1))),
        None
    );
}

#[test]
fn roundtrip_basic() {
    let src = r#"{:highlights [{:id "abc" :page 42 :properties {:color "yellow"}}] :extra {}}"#;
    let e = parse(src).unwrap();
    let hl = e.get("highlights").unwrap().as_vec().unwrap();
    assert_eq!(hl.len(), 1);
    assert_eq!(hl[0].get("id").unwrap().as_str(), Some("abc"));
    assert_eq!(hl[0].get("page").unwrap().as_i64(), Some(42));
    // reparse our own output
    let again = parse(&to_string(&e)).unwrap();
    assert_eq!(again, e);
}

#[test]
fn parses_floats_and_nil() {
    let e = parse(r#"{:top 100.5 :image nil}"#).unwrap();
    assert_eq!(e.get("top").unwrap().as_f64(), Some(100.5));
    assert_eq!(e.get("image"), Some(&Edn::Nil));
}

#[test]
fn parses_and_preserves_logseq_uuid_tags_and_rect_lists() {
    let src = r#"{:id #uuid "6a5604f8-a337-4336-a711-2ba6bc14fbfd"
        :rects ({:x1 10 :y1 20 :x2 30 :y2 40})}"#;
    let parsed = parse_strict(src).unwrap();
    assert_eq!(
        parsed.get("id").and_then(Edn::as_str),
        Some("6a5604f8-a337-4336-a711-2ba6bc14fbfd")
    );
    assert_eq!(parsed.get("rects").and_then(Edn::as_vec).unwrap().len(), 1);
    assert_eq!(parse_strict(&to_string(&parsed)), Some(parsed));
}

#[test]
fn malformed_or_excessively_nested_collections_fail_closed() {
    for src in ["(", "[1 2", "{:a 1", "(#uuid", "]", ")", "}", ":"] {
        assert_eq!(parse_strict(src), None, "accepted malformed EDN: {src}");
    }

    let too_deep = format!(
        "{}nil{}",
        "[".repeat(MAX_DEPTH + 1),
        "]".repeat(MAX_DEPTH + 1)
    );
    assert_eq!(parse_strict(&too_deep), None);
}

#[test]
fn every_escape_decodes_or_fails_closed_without_panicking() {
    // `\` before every byte class, including multi-byte chars, at every
    // position of a short string and at end of input (C3U / L01 H3).
    let tails = [
        "é",
        "\u{1F600}",
        "日本",
        "a",
        "z",
        "0",
        "7",
        "8",
        "u",
        "u12",
        "uD83D",
        "uD83Dx",
        "uD83D\\uDE00",
        "uDE00",
        "u00e9",
        "400",
        "377",
        "\"",
        "\\",
        "b",
        "f",
        "t",
        "r",
        "n",
        "",
        " ",
        "\n",
    ];
    for tail in tails {
        for prefix in ["", "x", "é", "ab"] {
            for close in ["\"", ""] {
                let src = format!("\"{prefix}\\{tail}{close}");
                let parsed = std::panic::catch_unwind(|| parse_strict(&src));
                let parsed = parsed.unwrap_or_else(|_| panic!("parser panicked on {src:?}"));
                if let Some(value) = parsed {
                    assert_eq!(
                        parse_strict(&to_string(&value)).as_ref(),
                        Some(&value),
                        "write-back changed {src:?}"
                    );
                }
            }
        }
    }
    assert_eq!(parse_strict("\"\\é\""), None, "unknown escape is not EDN");
    assert_eq!(parse_strict("\"\\a\""), None, "unknown escape is not EDN");
}

#[test]
fn every_prefix_of_a_mixed_document_parses_or_fails_without_panicking() {
    let src = "{:a \"caf\\u00e9 \\t\\b\\f\\101 日本\" :b \\é :c #{1 2} :d ##NaN :e foo/bar :f 1N :g [\\( \\space]}";
    for end in (0..=src.len()).filter(|end| src.is_char_boundary(*end)) {
        let _ = std::panic::catch_unwind(|| parse_strict(&src[..end]))
            .unwrap_or_else(|_| panic!("parser panicked on prefix {end}"));
    }
    // Byte-level truncations that are not char boundaries cannot reach
    // the parser (it takes &str), so boundaries are the whole input space.
    let parsed = parse_strict(src).expect("valid EDN");
    assert_eq!(
        parsed.get("a").and_then(Edn::as_str),
        Some("café \t\u{8}\u{c}A 日本")
    );
}

#[test]
fn escapes_decode_like_the_edn_reader() {
    let cases = [
        (r#""\b\f""#, "\u{8}\u{c}"),
        (r#""\u00e9""#, "é"),
        (r#""\uD83D\uDE00""#, "\u{1F600}"),
        (r#""\101\0""#, "A\u{0}"),
        (r#""\400""#, "Ā"),
    ];
    for (src, want) in cases {
        assert_eq!(parse_strict(src), Some(Edn::Str(want.into())), "{src}");
    }
    for (source, unit) in [(r#""\uD83D""#, 0xd83d), (r#""\uDE00""#, 0xde00)] {
        let value = parse_strict(source).unwrap();
        assert_eq!(value, Edn::Utf16(vec![unit]));
        assert_eq!(parse_strict(&to_string(&value)), Some(value));
    }
    assert_eq!(parse_strict(r#""\u12""#), None);
}

#[test]
fn decoded_values_keep_their_meaning_on_write_back() {
    let src = "{:sym foo.bar/baz :ch \\a :paren \\( :nan ##NaN :inf ##-Inf :big 1N :dec 1.5M :word inf :set #{1 :a} :t \"a\\tb\"}";
    let parsed = parse_strict(src).unwrap();
    let written = to_string(&parsed);
    for token in [
        ":sym foo.bar/baz",
        r#":ch "a""#,
        r#":paren "(""#,
        ":nan ##NaN",
        ":inf ##-Inf",
        ":big 1",
        ":dec 1.5",
        ":word inf",
        ":set #{1 :a}",
        r#":t "a\tb""#,
    ] {
        assert!(written.contains(token), "{token} changed: {written}");
    }
    let reopened = parse_strict(&written).unwrap();
    assert_eq!(to_string(&reopened), written);
    assert!(reopened.get("nan").unwrap().as_f64().unwrap().is_nan());
}

#[test]
fn the_occurrence_scan_runs_once_after_damage_and_reports_keyword_spans() {
    let scans = || EXCLUSION_SCANS.with(|count| count.get());
    let before = scans();
    for clean in [
        r#"{:hidden ["S"] :a 1}"#,
        "; {:hidden []}",
        "#_ {:hidden []}",
        "",
    ] {
        assert!(config_entries(clean).is_ok(), "{clean}");
    }
    assert_eq!(scans(), before, "a read that succeeds never scans");
    assert!(config_entries(r#"{:hidden ["S"] :bad #unknown []}"#).is_ok());
    assert!(config_entries(r#"{:bad #unknown [] :x :hidden}"#).is_err());
    assert_eq!(scans(), before + 2, "one scan per damaged read");

    let text = "{:hidden 1 \"x :hidden\" ; :hidden\n :_/hidden x#:hidden ::hidden \\c:hidden \
                \\;:hidden :a/hidden :hidden? :hidden-x x#! :hidden 1#! :hidden\n}";
    let spans: Vec<&str> = exclusion_occurrences(text)
        .into_iter()
        .map(|span| &text[span])
        .collect();
    // `x#!` is a symbol, not a comment; `1#!` is counted too (plain text only adds).
    assert_eq!(
        spans,
        [
            ":hidden",
            ":_/hidden",
            ":hidden",
            ":hidden",
            ":hidden",
            ":a/hidden",
            ":hidden",
            ":hidden"
        ]
    );
    assert_eq!(
        text.find(":hidden 1"),
        exclusion_occurrences(text).first().map(|s| s.start)
    );
    // A CR alone does not end a comment; an unterminated string leaves plain text.
    assert!(exclusion_occurrences("; x\r:hidden").is_empty());
    assert_eq!(exclusion_occurrences("; x\n:hidden").len(), 1);
    assert_eq!(exclusion_occurrences("\"x :hidden").len(), 1);
    // Every colon of a long run is checked in one pass over the run.
    let run = format!("{}hidden", ":".repeat(200_000));
    assert_eq!(exclusion_occurrences(&run), [199_999..run.len()]);
}
