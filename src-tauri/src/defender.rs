//! Windows Defender real-time protection: a flag for the diagnostics report, and
//! the one-time "Defender is slowing this graph down" hint with its opt-in
//! exclusion (GH #623).
//!
//! **Question answered.** Is Microsoft Defender scanning files as Tine opens
//! them ([`probe`]), is this graph's cold load slow enough to say so
//! ([`hint_eligible`]), and did the user's click add a scan exclusion for the
//! graph folder ([`add_exclusion`])?
//!
//! **Why it exists.** Measured on Windows Server 2025 (the 2026-10-02 A/B,
//! `specs/notes/2026-10-02-windows-defender-ab`): Defender moves exactly one
//! cost, the per-file open and first read, and only for files it has not yet
//! scanned. A 25.8k-file graph's cold load went from 6.5 s (off) to 34.6 s (on,
//! unseen files); the same files again, with Defender's verdicts cached, took
//! 7.4 s.
//!
//! **Three rules.**
//! 1. Probing is read-only and needs no administrator (WMI
//!    `MSFT_MpComputerStatus`). The report carries a closed token, never a
//!    path (I-5).
//! 2. Nothing changes Defender's configuration without a click. The exclusion
//!    runs `Add-MpPreference -ExclusionPath <graph root>` in a UAC-elevated
//!    child; declining the prompt is an ordinary, reported outcome.
//! 3. The platform split names every shipped target (AGENTS.md section 2). A
//!    bare `not(windows)` stub would also compile for a target Tine forgot;
//!    `every_shipped_target_is_named_exactly_once` pins the set.

use serde::Serialize;

/// What the probe could establish about Defender real-time protection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Realtime {
    On,
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    Off,
    /// Windows, but the query failed or answered something unexpected (no
    /// Defender at all, a third-party antivirus owning protection, no
    /// PowerShell).
    Unknown,
    /// Not a Windows build: Defender does not exist here.
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    NotApplicable,
}

impl Realtime {
    /// The closed token the diagnostics report carries.
    pub(crate) fn token(self) -> &'static str {
        match self {
            Realtime::On => "on",
            Realtime::Off => "off",
            Realtime::Unknown => "unknown",
            Realtime::NotApplicable => "not-applicable",
        }
    }
}

/// A cold graph load slower than this, with real-time protection on, earns the
/// hint. 15 s sits between the slowest Defender-neutral reading of the A/B
/// (9.3 s: Defender off, or on with verdicts cached) and the fastest slow one
/// (the reporter's real 23.2 s; 34.6 s measured with unseen files), so a load
/// that is slow for another reason (a very large graph on a quiet machine) does
/// not blame Defender unless it is also past the reporter's own number's
/// neighbourhood.
pub(crate) const SLOW_COLD_LOAD_MS: f64 = 15_000.0;

/// Parse the PowerShell answer to
/// `(Get-CimInstance ... MSFT_MpComputerStatus).RealTimeProtectionEnabled`.
/// Anything but a clean `True`/`False` line is `Unknown`: never guess.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))] // only the Windows module calls this; tests cover it everywhere
pub(crate) fn parse_realtime(output: &str) -> Realtime {
    match output.trim().to_ascii_lowercase().as_str() {
        "true" => Realtime::On,
        "false" => Realtime::Off,
        _ => Realtime::Unknown,
    }
}

/// The cold-load slowness test of the hint, with the dismissal and the probe
/// folded in: show only when real-time protection is on, the load was slow and
/// the user has not dismissed it for this graph.
pub(crate) fn hint_eligible(realtime: Realtime, ready_ms: Option<f64>, dismissed: bool) -> bool {
    realtime == Realtime::On
        && !dismissed
        && ready_ms.is_some_and(|ms| ms.is_finite() && ms >= SLOW_COLD_LOAD_MS)
}

/// A `\\?\` verbatim prefix is how `canonicalize` spells a Windows path;
/// Defender's exclusion list wants the ordinary form.
pub(crate) fn exclusion_path(root: &std::path::Path) -> String {
    let text = root.display().to_string();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else {
        text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
    }
}

/// PowerShell single-quoted literal: `'` doubles, nothing else is special.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))] // only the Windows module calls this; tests cover it everywhere
fn ps_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// The script the elevated child runs. The path is one quoted literal, so a
/// folder name cannot add a command.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))] // only the Windows module calls this; tests cover it everywhere
pub(crate) fn inner_script(path: &str) -> String {
    format!(
        "$ErrorActionPreference='Stop'; Add-MpPreference -ExclusionPath {}",
        ps_quote(path)
    )
}

/// The unelevated parent: starts the inner script elevated (the UAC prompt),
/// waits, and exits with the child's code. A declined or impossible elevation
/// exits [`EXIT_NOT_ELEVATED`], so "you said no" is distinguishable from
/// "Defender refused".
#[cfg_attr(not(target_os = "windows"), allow(dead_code))] // only the Windows module calls this; tests cover it everywhere
pub(crate) fn outer_script(inner_b64: &str) -> String {
    format!(
        "$ErrorActionPreference='Stop'; try {{ $p = Start-Process -FilePath powershell.exe \
         -Verb RunAs -Wait -PassThru -WindowStyle Hidden \
         -ArgumentList @('-NoProfile','-NonInteractive','-EncodedCommand','{inner_b64}'); \
         exit $p.ExitCode }} catch {{ exit {EXIT_NOT_ELEVATED} }}"
    )
}

pub(crate) const EXIT_NOT_ELEVATED: i32 = 2;

/// `-EncodedCommand` wants base64 of UTF-16LE.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))] // only the Windows module calls this; tests cover it everywhere
pub(crate) fn encode_command(script: &str) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// How the user's click on "Add an exclusion" ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "outcome")]
pub(crate) enum ExclusionOutcome {
    /// Defender accepted the exclusion.
    Added,
    /// The user declined the administrator prompt, or elevation was not
    /// possible; nothing changed.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    Declined,
    /// The elevated command ran and failed (policy-managed Defender, tamper
    /// protection, third-party antivirus). `code` is the exit code, if any.
    Failed { code: Option<i32>, message: String },
}

/// Map the outer process's exit code.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))] // only the Windows module calls this; tests cover it everywhere
pub(crate) fn outcome_from_exit(code: Option<i32>) -> ExclusionOutcome {
    match code {
        Some(0) => ExclusionOutcome::Added,
        Some(EXIT_NOT_ELEVATED) => ExclusionOutcome::Declined,
        other => ExclusionOutcome::Failed {
            code: other,
            message: "Windows Defender did not accept the exclusion. It may be managed by your organization or by another antivirus."
                .into(),
        },
    }
}

// ---- Platform split: every shipped target is named. ----

/// Targets with a Defender: Windows. The rest are named so the split is a
/// decision, not an omission.
#[cfg(target_os = "windows")]
mod platform {
    use super::*;
    use std::os::windows::process::CommandExt as _;
    use std::process::{Command, Stdio};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    fn powershell(script: &str) -> Command {
        let mut command = Command::new("powershell.exe");
        command
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .stdin(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW);
        command
    }

    pub(super) fn probe() -> Realtime {
        let script = "(Get-CimInstance -Namespace root/Microsoft/Windows/Defender \
                      -ClassName MSFT_MpComputerStatus).RealTimeProtectionEnabled";
        match powershell(script).stderr(Stdio::null()).output() {
            Ok(output) if output.status.success() => {
                parse_realtime(&String::from_utf8_lossy(&output.stdout))
            }
            _ => Realtime::Unknown,
        }
    }

    pub(super) fn add_exclusion(path: &str) -> ExclusionOutcome {
        let inner = encode_command(&inner_script(path));
        let outer = outer_script(&inner);
        match powershell(&outer)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        {
            Ok(status) => outcome_from_exit(status.code()),
            Err(error) => ExclusionOutcome::Failed {
                code: None,
                message: format!("Could not start PowerShell: {error}"),
            },
        }
    }
}

#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "android"
))]
mod platform {
    use super::*;

    pub(super) fn probe() -> Realtime {
        Realtime::NotApplicable
    }

    pub(super) fn add_exclusion(_path: &str) -> ExclusionOutcome {
        ExclusionOutcome::Failed {
            code: None,
            message: "Windows Defender exclusions exist only on Windows.".into(),
        }
    }
}

/// Query real-time protection now (a PowerShell start, about a second on
/// Windows). Blocking: call from a blocking thread.
pub(crate) fn probe() -> Realtime {
    platform::probe()
}

/// The probe once per process: the hint asks at every graph load.
pub(crate) fn probe_cached() -> Realtime {
    static CACHE: std::sync::OnceLock<Realtime> = std::sync::OnceLock::new();
    *CACHE.get_or_init(probe)
}

/// Run the elevated exclusion for `root`. Blocking until the user answers the
/// prompt and the child exits.
pub(crate) fn add_exclusion(root: &std::path::Path) -> ExclusionOutcome {
    platform::add_exclusion(&exclusion_path(root))
}

/// What the frontend needs to decide whether to show the hint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DefenderHint {
    pub(crate) show: bool,
}

const NOTICE_KEY: &str = "windows-defender-hint";

/// The cold-load duration of this graph, once its first load has finished.
fn ready_ms(diagnostics: &serde_json::Value) -> Option<f64> {
    diagnostics["launch"]["readyMs"].as_f64()
}

fn graph_context(
    ctx: &crate::state::GraphContext<'_>,
    app: &tauri::AppHandle,
) -> Result<(std::sync::Arc<crate::state::GraphSlot>, std::path::PathBuf), String> {
    use tauri::Manager as _;
    let slot = crate::state::slot_for_context(ctx)?;
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    Ok((slot, dir))
}

/// Should the one-time Defender hint show for the bound graph? Cheap on every
/// platform but Windows (a constant); on Windows the PowerShell probe runs once
/// per process, and the diagnostics snapshot is read only when protection is on
/// and the hint has not been dismissed.
#[tauri::command]
pub(crate) async fn defender_hint(
    app: tauri::AppHandle,
    ctx: crate::state::GraphContext<'_>,
) -> Result<DefenderHint, String> {
    let (slot, dir) = graph_context(&ctx, &app)?;
    let shown = tauri::async_runtime::spawn_blocking(move || {
        if crate::settings::notice_dismissed(&dir, &slot.root_key, NOTICE_KEY) {
            return false;
        }
        let realtime = probe_cached();
        if realtime != Realtime::On {
            return false;
        }
        hint_eligible(realtime, ready_ms(&slot.store.diagnostics()), false)
    })
    .await
    .map_err(|error| error.to_string())?;
    Ok(DefenderHint { show: shown })
}

/// "Dismiss", or "not now": one per graph, never shown again for it.
#[tauri::command]
pub(crate) async fn dismiss_defender_hint(
    app: tauri::AppHandle,
    ctx: crate::state::GraphContext<'_>,
) -> Result<(), String> {
    let (slot, dir) = graph_context(&ctx, &app)?;
    crate::state::off_ui(move || {
        crate::settings::set_notice_at(&dir, &slot.root_key, NOTICE_KEY, true)
    })
    .await
}

/// The click on "Add an exclusion for this graph folder". Never called without
/// one. On success the hint is dismissed for this graph as well.
#[tauri::command]
pub(crate) async fn add_defender_exclusion(
    app: tauri::AppHandle,
    ctx: crate::state::GraphContext<'_>,
) -> Result<ExclusionOutcome, String> {
    let (slot, dir) = graph_context(&ctx, &app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let outcome = add_exclusion(&slot.root_key);
        if outcome == ExclusionOutcome::Added {
            crate::settings::set_notice_at(&dir, &slot.root_key, NOTICE_KEY, true)?;
        }
        Ok(outcome)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn realtime_output_parses_only_clean_booleans() {
        assert_eq!(parse_realtime("True\r\n"), Realtime::On);
        assert_eq!(parse_realtime("  false \n"), Realtime::Off);
        for junk in [
            "",
            "1",
            "N/A",
            "Get-CimInstance : Invalid namespace",
            "True False",
        ] {
            assert_eq!(parse_realtime(junk), Realtime::Unknown, "{junk:?}");
        }
    }

    #[test]
    fn hint_needs_protection_on_a_slow_cold_load_and_no_dismissal() {
        let slow = Some(SLOW_COLD_LOAD_MS);
        assert!(hint_eligible(Realtime::On, slow, false));
        assert!(!hint_eligible(
            Realtime::On,
            Some(SLOW_COLD_LOAD_MS - 1.0),
            false
        ));
        assert!(
            !hint_eligible(Realtime::On, None, false),
            "load not finished"
        );
        assert!(!hint_eligible(Realtime::On, Some(f64::NAN), false));
        assert!(!hint_eligible(Realtime::Off, slow, false));
        assert!(!hint_eligible(Realtime::Unknown, slow, false));
        assert!(!hint_eligible(Realtime::NotApplicable, slow, false));
        assert!(
            !hint_eligible(Realtime::On, slow, true),
            "dismissed for this graph"
        );
    }

    #[test]
    fn threshold_sits_between_the_neutral_and_the_slow_measurements() {
        // 2026-10-02 A/B: Defender off 7.7 s, on with verdicts cached 9.3 s;
        // reporter 23.2 s; on with unseen files 34.6 s.
        assert!(9_292.0 < SLOW_COLD_LOAD_MS && SLOW_COLD_LOAD_MS < 23_172.0);
    }

    #[test]
    fn exclusion_path_drops_the_verbatim_prefix() {
        let p = |text: &str| exclusion_path(std::path::Path::new(text));
        assert_eq!(p(r"\\?\C:\Users\me\graph"), r"C:\Users\me\graph");
        assert_eq!(p(r"\\?\UNC\server\share\graph"), r"\\server\share\graph");
        assert_eq!(p(r"D:\notes"), r"D:\notes");
    }

    #[test]
    fn a_hostile_folder_name_stays_one_literal() {
        let script = inner_script("C:\\it's; Remove-Item x");
        assert_eq!(
            script,
            "$ErrorActionPreference='Stop'; Add-MpPreference -ExclusionPath 'C:\\it''s; Remove-Item x'"
        );
    }

    #[test]
    fn encoded_command_is_base64_of_utf16le() {
        // "A" = 0x41 0x00 -> "QQA=" ; "hi" = 68 00 69 00 -> "aABpAA=="
        assert_eq!(encode_command("A"), "QQA=");
        assert_eq!(encode_command("hi"), "aABpAA==");
        assert_eq!(encode_command(""), "");
    }

    #[test]
    fn outcomes_distinguish_added_declined_and_failed() {
        assert_eq!(outcome_from_exit(Some(0)), ExclusionOutcome::Added);
        assert_eq!(
            outcome_from_exit(Some(EXIT_NOT_ELEVATED)),
            ExclusionOutcome::Declined
        );
        assert!(matches!(
            outcome_from_exit(Some(1)),
            ExclusionOutcome::Failed { code: Some(1), .. }
        ));
        assert!(matches!(
            outcome_from_exit(None),
            ExclusionOutcome::Failed { code: None, .. }
        ));
    }

    #[test]
    fn outer_script_exits_with_the_childs_code_and_a_distinct_decline() {
        let script = outer_script("QQA=");
        assert!(script.contains("-Verb RunAs") && script.contains("-Wait"));
        assert!(script.contains("exit $p.ExitCode"));
        assert!(script.contains(&format!("exit {EXIT_NOT_ELEVATED}")));
    }

    /// AGENTS.md section 2: a platform `cfg` list names every shipped target.
    /// Windows owns the real code; the other four are named in the stub arm.
    #[test]
    fn every_shipped_target_is_named_exactly_once() {
        let source = include_str!("defender.rs");
        let production = source.split("#[cfg(test)]").next().unwrap();
        let arms: Vec<&str> = production
            .lines()
            .filter(|line| {
                line.trim_start().starts_with("target_os = \"") || line.contains("#[cfg(target_os")
            })
            .collect();
        let mut named: Vec<String> = arms
            .iter()
            .flat_map(|line| {
                line.split('"')
                    .skip(1)
                    .step_by(2)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .collect();
        named.sort();
        assert_eq!(
            named,
            ["android", "ios", "linux", "macos", "windows"],
            "the platform split must name Linux, Windows, macOS, iOS and Android exactly once \
             (AGENTS.md section 2); a stub that omits a target silently turns a working feature off there"
        );
    }

    #[test]
    fn non_windows_builds_report_not_applicable() {
        if cfg!(target_os = "windows") {
            return;
        }
        assert_eq!(probe(), Realtime::NotApplicable);
        assert_eq!(Realtime::NotApplicable.token(), "not-applicable");
        assert!(matches!(
            add_exclusion(std::path::Path::new("/tmp/x")),
            ExclusionOutcome::Failed { code: None, .. }
        ));
    }
}
