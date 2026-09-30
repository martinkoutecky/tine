import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";

const native = (name: string) => readFileSync(`src-tauri/src/${name}.rs`, "utf8");
const RULE = "I-20: native startup work belongs to its binding/show and waits for signals; exemplar src-tauri/src/state.rs";

describe("OG-R3B startup ownership", () => {
  it("has one signal-producing warm and cancellation door", () => {
    const graph = native("graph");
    const backup = native("backup");
    expect(graph, RULE).toContain("slot.begin_startup_warm()");
    expect(graph, RULE).toContain("finish_startup_warm(warm_generation)");
    expect(graph, RULE).not.toMatch(/warm_done\.store/);
    expect(backup, RULE).toContain("if !wait_launch_backup(&slot)");
    expect(backup, RULE).toContain("slot.wait_startup_idle(LAUNCH_BACKUP_QUIET, LAUNCH_BACKUP_DEADLINE)");
    expect(backup, RULE).not.toContain("std::thread::sleep");
    const state = native("state");
    expect(state, RULE).toMatch(/self\s*\.startup_changed\s*\.wait_timeout/);
    expect(state, RULE).toContain("old.cancel_background()");
    expect(state, RULE).toContain("slot.cancel_background()");
    expect(state.match(/background_cancelled\s*\.store\(true/g), RULE).toHaveLength(1);
  });

  it("owns presentation and delayed focus with the same show generation", () => {
    // lib.rs carries earlier test modules, so read the complete file here.
    const lib = readFileSync("src-tauri/src/lib.rs", "utf8");
    expect(lib, RULE).toContain("let show_generation = state.begin_capture_show()");
    expect(lib, RULE).toContain("state.pending_capture_show() == Some(show_generation)");
    expect(lib, RULE).toContain("slot.binding_generation == binding_generation");
    expect(lib, RULE).toContain("capture_show_is_current(show_generation)");
    expect(lib, RULE).toContain("activate_capture_window(&main_thread_app, show_generation)");
    expect(lib, RULE).not.toContain("activate_capture_window(&main_thread_app);");
    expect(native("graph"), RULE).toContain("crate::complete_pending_capture_show");
  });
});
