import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

// Ported from master src/iosAtomicPublicationContract.test.ts (8c2e839e1,
// 71fa94a50), adapted to og: every atomic publication lives in tine-store, whose
// no-replace owner is pinned per target by crates/tine-store/tests/i16_platform_arms.rs.
// This adds master's stricter rule — no Darwin arm in the storage crate may be
// macOS-only, because a `cfg(target_os = "macos")` arm compiles fine for iOS and
// silently selects the unsupported fallback there (AGENTS.md §2, rename_noreplace).
function rustSources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(dir, entry.name);
    if (entry.isDirectory()) return rustSources(file);
    return file.endsWith(".rs") ? [file] : [];
  });
}

const MACOS_ONLY = /#\[cfg\(\s*target_os\s*=\s*"macos"\s*\)\]/;

describe("iOS atomic publication platform boundary", () => {
  it("routes every Darwin storage arm through iOS as well as macOS", () => {
    const sources = rustSources("crates/tine-store/src");
    expect(sources.length).toBeGreaterThan(0);
    for (const file of sources) {
      expect(readFileSync(file, "utf8"), `I-16: ${file} has a macOS-only storage arm; exemplar no_replace.rs any(macos, ios)`)
        .not.toMatch(MACOS_ONLY);
    }
  });

  it("the macOS-only scan catches a bare macOS arm", () => {
    expect('    #[cfg(target_os = "macos")]\n    fn publish() {}').toMatch(MACOS_ONLY);
    expect('    #[cfg(any(target_os = "macos", target_os = "ios"))]').not.toMatch(MACOS_ONLY);
  });

  // Named by behaviour, not by shell variable (master aedbfdc39).
  it("exercises Guide copy inside the iOS Simulator rather than only launching the app", () => {
    const workflow = readFileSync(".github/workflows/ios-probe.yml", "utf8");
    // The launch asks the app to copy the Guide in, and the assertion afterwards
    // proves a non-empty Guide page landed in the container graph.
    expect(workflow).toContain("--tine-ci-copy-guide");
    expect(workflow).toMatch(/guide_page="?\$\(find .*-name '\*Tine Guide.md'/i);
    expect(workflow).toMatch(/test -s "\$guide_page"/i);
    // The graph handed to the app is a container path whose Application UUID is
    // stale, so the probe also proves the app rebases it instead of trusting it.
    // It travels through TINE_GRAPH: og reads positional launch paths only on
    // desktop (src-tauri/src/graph.rs resolve_root).
    expect(workflow).toMatch(/stale_graph/i);
    expect(workflow).toMatch(/00000000-0000-4000-8000-000000000000/);
    expect(workflow).toContain('SIMCTL_CHILD_TINE_GRAPH="$stale_graph"');
    const graphRs = readFileSync("src-tauri/src/graph.rs", "utf8");
    expect(graphRs).toContain('for var in ["TINE_GRAPH"]');
    expect(graphRs).toMatch(/#\[cfg\(all\(target_os = "ios", target_abi = "sim"\)\)\]\s*if std::env::args\(\)\.any\(\|argument\| argument == "--tine-ci-copy-guide"\)/);
  });

  it("carries an iOS-rebased graph path across the Rust mobile-plugin bridge", () => {
    const bridge = readFileSync("src-tauri/src/ios_folder_picker.rs", "utf8");
    expect(bridge).toMatch(/struct PrepareGraphFolderResult\s*\{[\s\S]*?path:\s*Option<String>/);
  });
});
