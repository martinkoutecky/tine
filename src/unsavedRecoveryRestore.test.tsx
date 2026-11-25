// A rename's source whose pair a Use Disk cancelled after the source was
// removed: its original text is only in the rename's recovery draft, and the
// unsaved-changes recovery panel (GH #540) lists it as "kept after a cancelled
// rename" with Restore beside Copy draft. The host names the list (with its
// record of each restore) in page mail whenever it changes (`binding.rs`
// `mail_restorable`) and in each session's reload reply; a newly kept source
// opens the panel, and a restore's outcome is the host's record.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "./backend";
import { bindHost, resetStore } from "./document";
import { bindTestHost, type TestHost } from "./document/host/wiring.test.support";
import { UnsavedRecovery } from "./components/UnsavedRecovery";
import { closeUnsavedRecovery, unsavedRecoveryOpen } from "./unsavedRecovery";
import { restorableSources, setRestorableSources } from "./restorableSources";
import { paletteCommands, runGlobalCommand } from "./keybindings";
import { setToasts, toasts } from "./toasts";
import type { RestorableSource, RestoreState } from "./document/host/protocol";

const KEPT: RestorableSource = { key: "pages/a.md", path: "pages/a.md", text: "- old\n" };
const flush = async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); };

let host: TestHost;
let root: HTMLDivElement;
let dispose: () => void = () => {};
let generation = 10;
const button = (label: string) => [...root.querySelectorAll("button")].find((b) => b.textContent === label);
/** The host's list at a new generation, as a mail of its own (no page has mail). */
const listed = (sources: RestorableSource[], options: { session?: number; generation?: number } = {}) =>
  host.deliver({ key: "", page: null, answer: null, restorable: { generation: options.generation ?? ++generation, sources }, session: options.session });
const recorded = (state: RestoreState): RestorableSource => ({ ...KEPT, state });
const showPanel = () => { dispose = render(() => <UnsavedRecovery />, root); };

beforeEach(async () => {
  resetStore();
  setToasts([]);
  host = await bindTestHost();
  root = document.createElement("div");
  document.body.appendChild(root);
});
afterEach(() => {
  dispose();
  dispose = () => {};
  closeUnsavedRecovery();
  vi.restoreAllMocks();
  resetStore();
  setToasts([]);
  document.body.innerHTML = "";
});

describe("restoring a source a cancelled rename kept (GH #540, option B)", () => {
  it("a newly kept source opens the panel; Restore calls the host, and the host's record says Restoring, then Restored", async () => {
    const restore = vi.spyOn(backend(), "pageRestoreSource").mockResolvedValue("pending");
    const write = vi.spyOn(backend(), "writeText").mockResolvedValue();
    listed([KEPT]);
    expect(unsavedRecoveryOpen()).toBe(true);
    showPanel();
    expect(root.querySelector("h3")?.textContent).toBe("pages/a.md — kept after a cancelled rename");
    expect(root.querySelector("pre")?.textContent).toBe("- old\n");
    expect(root.textContent).not.toContain("No pending page drafts");

    button("Copy draft")!.click();
    await flush();
    expect(write).toHaveBeenCalledWith("- old\n");

    button("Restore")!.click();
    await flush();
    expect(restore).toHaveBeenCalledWith(host.session, "pages/a.md");
    expect(toasts()).toEqual([]);

    listed([recorded({ kind: "restoring", problem: null })]);
    await flush();
    expect(root.textContent).toContain("Restoring…");
    expect(button("Restore")).toBeUndefined();

    // Its own save published the text: the host's record says so.
    listed([{ ...KEPT, text: "", state: { kind: "restored" } }]);
    await flush();
    expect(root.textContent).toContain("Restored pages/a.md.");
    expect(root.textContent).not.toContain("kept after a cancelled rename");
    expect(root.textContent).toContain("No pending page drafts");
  });

  // Blocker 2 (review of 7f00b9a25): a restore refused or failing after its
  // admission. Before, the panel said "Restoring…" from the command's admission
  // and nothing ever ended it.
  it("a restore that fails after admission shows the host's reason, and a refusal ends it with Restore offered again", async () => {
    vi.spyOn(backend(), "pageRestoreSource").mockResolvedValue("pending");
    listed([KEPT]);
    showPanel();
    button("Restore")!.click();
    await flush();
    listed([recorded({ kind: "restoring", problem: { reason: "claimed", file: "pages/a.org" } })]);
    await flush();
    expect(root.textContent).toContain("Not restored yet: pages/a.org claims the page pages/a.md. Tine keeps trying");
    expect(root.textContent).not.toContain("Restoring…");
    listed([recorded({ kind: "restoring", problem: { reason: "failed", operation: "write temp", osError: 28 } })]);
    await flush();
    expect(root.textContent).toContain("Not restored yet: writing pages/a.md failed (write temp, os error 28). Tine keeps trying");
    listed([recorded({ kind: "refused", reason: "occupied" })]);
    await flush();
    expect(root.textContent).toContain("Couldn't restore pages/a.md: a file with that name exists again.");
    expect(button("Restore")).toBeDefined();
    expect(root.querySelector("pre")?.textContent).toBe("- old\n");
    expect(toasts()).toEqual([]);
  });

  it("a refusal the host did not record is an error toast, and the original text stays to copy", async () => {
    vi.spyOn(backend(), "pageRestoreSource").mockResolvedValue("refused");
    listed([KEPT]);
    showPanel();
    button("Restore")!.click();
    await vi.waitFor(() => expect(toasts()).toHaveLength(1));
    expect(toasts()[0]).toMatchObject({ kind: "error" });
    expect(toasts()[0].message).toContain("couldn't start restoring pages/a.md");
    expect(root.querySelector("pre")?.textContent).toBe("- old\n");
    expect(restorableSources()).toEqual([KEPT]);
  });

  it("a failed command is shown too, never swallowed", async () => {
    vi.spyOn(backend(), "pageRestoreSource").mockRejectedValue(new Error("the page host is stopping"));
    listed([KEPT]);
    showPanel();
    button("Restore")!.click();
    await vi.waitFor(() => expect(toasts()).toHaveLength(1));
    expect(toasts()[0]).toMatchObject({ kind: "error" });
    expect(toasts()[0].message).toContain("the page host is stopping");
  });

  it("the graph-open list does not open the panel, and the same list mailed again does not either", () => {
    setRestorableSources({ generation: 1, sources: [KEPT] }, true);
    expect(unsavedRecoveryOpen()).toBe(false);
    listed([KEPT]);
    expect(unsavedRecoveryOpen()).toBe(false);
    expect(restorableSources()).toEqual([KEPT]);
  });

  it("a list mailed for another session is dropped, and an empty-key mail reaches no page", () => {
    listed([KEPT], { session: host.session + 1 });
    expect(restorableSources()).toEqual([]);
    expect(unsavedRecoveryOpen()).toBe(false);
  });

  // Follow-up (review of 7f00b9a25, DS-12): stashed page mail replays its list
  // after newer mail; the generation keeps the newer one.
  it("an older list replayed after a newer one changes nothing", () => {
    listed([KEPT], { generation: 50 });
    listed([], { generation: 51 });
    listed([KEPT], { generation: 50 });
    expect(restorableSources()).toEqual([]);
  });

  // Follow-up (review of 7f00b9a25, DS-12): a same-binding rebind to a host
  // whose list emptied meanwhile. The reload reply now carries the list.
  it("a rebind takes the host's list from its reload reply, an empty one included", async () => {
    listed([KEPT], { generation: 60 });
    expect(restorableSources()).toEqual([KEPT]);
    vi.spyOn(backend(), "pageWindowReloaded").mockResolvedValue({ session: host.session + 7, nextId: 1, restorable: { generation: 2, sources: [] } });
    await bindHost();
    expect(restorableSources()).toEqual([]);
  });
});

// Blocker 1 (review of 7f00b9a25): a standing way in. Before, closing the panel
// (or dismissing the launch notice) left no route back to a kept source short of
// another save problem or reopening the graph.
describe("the standing entry to kept sources", () => {
  const offered = () => paletteCommands().some((c) => c.id === "recovery/review-kept-originals");

  it("the command palette offers Review pages kept after a cancelled rename while the list names one, and it opens the panel", () => {
    expect(offered()).toBe(false);
    listed([KEPT]);
    closeUnsavedRecovery();
    expect(offered()).toBe(true);
    expect(paletteCommands().find((c) => c.id === "recovery/review-kept-originals")?.english).toBe("Review pages kept after a cancelled rename");
    expect(runGlobalCommand("recovery/review-kept-originals")).toBe(true);
    expect(unsavedRecoveryOpen()).toBe(true);
  });

  it("is gone once the list names only finished restores, or nothing", () => {
    listed([{ ...KEPT, text: "", state: { kind: "restored" } }]);
    expect(offered()).toBe(false);
    listed([]);
    expect(offered()).toBe(false);
  });
});
