#[cfg(unix)]
mod unix {
    use std::os::unix::process::CommandExt;
    use std::process::Command;
    use std::thread;
    use std::time::{Duration, Instant};

    const CHILD_ENV: &str = "TINE_LOGSEQ_PDF_EDN_CHILD";
    const SIDE_CAR: &str = r#"{:highlights [{:id #uuid "6a5604f8-a337-4336-a711-2ba6bc14fbfd"
        :page 1
        :position {:bounding {:x1 292.1 :y1 488.4 :x2 555.5 :y2 535.1
                              :width 822 :height 1063.7}
                   :rects ({:x1 292.1 :y1 488.4 :x2 555.5 :y2 535.1
                            :width 822 :height 1063.7})
                   :page 1}
        :content {:text "MyLifeOrganized"}
        :properties {:color "yellow"}}]
        :extra {:page 1}}"#;

    #[test]
    fn current_logseq_sidecar_parses_with_bounded_memory() {
        if std::env::var_os(CHILD_ENV).is_some() {
            let highlights = tine_core::pdf::parse_highlights(SIDE_CAR);
            assert_eq!(highlights.len(), 1);
            assert_eq!(highlights[0].id, "6a5604f8-a337-4336-a711-2ba6bc14fbfd");
            return;
        }

        let current_exe = std::env::current_exe().expect("locate integration-test binary");
        let mut command = Command::new(current_exe);
        command
            .arg("--exact")
            .arg("unix::current_logseq_sidecar_parses_with_bounded_memory")
            .arg("--nocapture")
            .env(CHILD_ENV, "1");

        // The regression was an EDN collection loop that returned a value without
        // consuming input and allocated until the process or host ran out of memory.
        // Prove termination in a child with a hard address-space limit so the
        // fail-before test remains safe even when run against the vulnerable code.
        unsafe {
            command.pre_exec(|| {
                let limit = libc::rlimit {
                    rlim_cur: 256 * 1024 * 1024,
                    rlim_max: 256 * 1024 * 1024,
                };
                if libc::setrlimit(libc::RLIMIT_AS, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let mut child = command.spawn().expect("spawn bounded parser child");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = child.try_wait().expect("poll parser child") {
                assert!(status.success(), "bounded parser child failed: {status}");
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("Logseq PDF sidecar parsing did not terminate within 5 seconds");
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
}

#[test]
fn og_sidecar_edit_preserves_untouched_shapes_and_reads_legacy_tine_shapes() {
    const OG_SIDECAR: &str = r#"{:highlights
      [{:id #uuid "11111111-1111-1111-1111-111111111111"
        :page 1
        :position {:page 1
                   :bounding {:x1 10 :y1 20 :x2 30 :y2 40 :width 600 :height 800}
                   :rects ({:x1 10 :y1 20 :x2 30 :y2 40 :width 600 :height 800})}
        :content {:text "OG text"}
        :properties {:color "yellow"}
        :plugin-field "text-shape-stays"}
       {:id #uuid "22222222-2222-2222-2222-222222222222"
        :page 2
        :position {:page 2
                   :bounding {:x1 50 :y1 60 :x2 150 :y2 160 :width 600 :height 800}
                   :rects ()}
        :content {:text "[:span]" :image 1659920114630}
        :properties {:color "blue"}
        :plugin-field "area-shape-stays"}]
      :extra {:page 2 :plugin "keep"}
      :root-plugin-field 42}"#;
    const LEGACY_TINE_SIDECAR: &str = r#"{:highlights
      [{:id "legacy-text" :page 3
        :position {:page 3 :bounding {:top 1 :left 2 :width 3 :height 4} :rects []}
        :content {:text "legacy" :image nil} :properties {:color "green"}}
       {:id "legacy-area" :page 4
        :position {:page 4 :bounding {:top 5 :left 6 :width 7 :height 8} :rects []}
        :content {:text "" :image 1234} :properties {:color "red"}}]}"#;

    let legacy = tine_core::pdf::parse_highlights(LEGACY_TINE_SIDECAR);
    assert_eq!(legacy.len(), 2);
    assert_eq!(legacy[0].text.as_deref(), Some("legacy"));
    assert_eq!(legacy[0].image, None);
    assert_eq!(legacy[1].text, None);
    assert_eq!(legacy[1].image, Some(1234));

    let mut highlights = tine_core::pdf::parse_highlights(OG_SIDECAR);
    assert_eq!(highlights.len(), 2);
    highlights[0].color = "purple".to_string();
    let out = tine_core::pdf::write_highlights(&highlights, OG_SIDECAR);
    let root = tine_core::edn::parse_strict(&out).unwrap();
    let stored = root
        .get("highlights")
        .and_then(tine_core::edn::Edn::as_vec)
        .unwrap();

    let text_content = stored[0].get("content").unwrap();
    assert_eq!(
        text_content
            .get("text")
            .and_then(tine_core::edn::Edn::as_str),
        Some("OG text")
    );
    assert_eq!(
        text_content.get("image"),
        None,
        "untouched OG text field shape changed: {out}"
    );
    assert_eq!(
        stored[0]
            .get("plugin-field")
            .and_then(tine_core::edn::Edn::as_str),
        Some("text-shape-stays")
    );

    let area_content = stored[1].get("content").unwrap();
    assert_eq!(
        area_content
            .get("text")
            .and_then(tine_core::edn::Edn::as_str),
        Some("[:span]")
    );
    assert_eq!(
        area_content
            .get("image")
            .and_then(tine_core::edn::Edn::as_i64),
        Some(1659920114630)
    );
    assert_eq!(
        stored[1]
            .get("plugin-field")
            .and_then(tine_core::edn::Edn::as_str),
        Some("area-shape-stays")
    );
    assert_eq!(
        root.get("root-plugin-field")
            .and_then(tine_core::edn::Edn::as_i64),
        Some(42)
    );
}
