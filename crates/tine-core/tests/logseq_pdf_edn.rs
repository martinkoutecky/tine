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
            assert_eq!(
                highlights[0].id,
                "6a5604f8-a337-4336-a711-2ba6bc14fbfd"
            );
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
