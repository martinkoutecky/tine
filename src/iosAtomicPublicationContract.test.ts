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

  it("carries an iOS-rebased graph path across the Rust mobile-plugin bridge", () => {
    const bridge = readFileSync("src-tauri/src/ios_folder_picker.rs", "utf8");
    expect(bridge).toMatch(/struct PrepareGraphFolderResult\s*\{[\s\S]*?path:\s*Option<String>/);
  });
});
