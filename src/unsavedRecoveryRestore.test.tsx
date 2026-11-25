// A rename's source whose pair a Use Disk cancelled after the source was
// removed: its original text is only in the rename's recovery draft, and the
// unsaved-changes recovery panel (GH #540) lists it as "kept after a cancelled
// rename" with Restore beside Copy draft. The host names the list in page mail
// whenever it changes (`binding.rs` `mail_restorable`); a newly kept source
// opens the panel, and a restored one leaves it once its save publishes.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "./backend";
import { resetStore } from "./document";
import { bindTestHost, type TestHost } from "./document/host/wiring.test.support";
import { UnsavedRecovery } from "./components/UnsavedRecovery";
import { closeUnsavedRecovery, unsavedRecoveryOpen } from "./unsavedRecovery";
import { restorableSources, setRestorableSources } from "./restorableSources";
import { setToasts, toasts } from "./toasts";
import type { RestorableSource } from "./document/host/protocol";

const KEPT: RestorableSource = { key: "pages/a.md", path: "pages/a.md", text: "- old\n" };
const flush = async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); };

let host: TestHost;
let root: HTMLDivElement;
let dispose: () => void = () => {};
const button = (label: string) => [...root.querySelectorAll("button")].find((b) => b.textContent === label)!;
/** The host's list, as a mail of its own (no page has mail). */
const listed = (restorable: RestorableSource[], session?: number) =>
  host.deliver({ key: "", page: null, answer: null, restorable, session });

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
  it("a newly kept source opens the panel; Restore calls the host and the entry leaves once restored", async () => {
    const restore = vi.spyOn(backend(), "pageRestoreSource").mockResolvedValue("pending");
    const write = vi.spyOn(backend(), "writeText").mockResolvedValue();
    listed([KEPT]);
    expect(unsavedRecoveryOpen()).toBe(true);
    dispose = render(() => <UnsavedRecovery />, root);
    expect(root.querySelector("h3")?.textContent).toBe("pages/a.md — kept after a cancelled rename");
    expect(root.querySelector("pre")?.textContent).toBe("- old\n");
    expect(root.textContent).not.toContain("No pending page drafts");

    button("Copy draft").click();
    await flush();
    expect(write).toHaveBeenCalledWith("- old\n");

    button("Restore").click();
    await vi.waitFor(() => expect(root.textContent).toContain("Restoring…"));
    expect(restore).toHaveBeenCalledWith(host.session, "pages/a.md");
    expect(toasts()).toEqual([]);

    // Its own save published the text: the host's list no longer names it.
    listed([]);
    await flush();
    expect(root.textContent).not.toContain("kept after a cancelled rename");
    expect(restorableSources()).toEqual([]);
  });

  it("a refused restore is an error toast, and the original text stays to copy", async () => {
    vi.spyOn(backend(), "pageRestoreSource").mockResolvedValue("refused");
    listed([KEPT]);
    dispose = render(() => <UnsavedRecovery />, root);
    button("Restore").click();
    await vi.waitFor(() => expect(toasts()).toHaveLength(1));
    expect(toasts()[0]).toMatchObject({ kind: "error" });
    expect(toasts()[0].message).toContain("Couldn't restore pages/a.md");
    expect(root.querySelector("pre")?.textContent).toBe("- old\n");
    expect(restorableSources()).toEqual([KEPT]);
  });

  it("a failed command is shown too, never swallowed", async () => {
    vi.spyOn(backend(), "pageRestoreSource").mockRejectedValue(new Error("the page host is stopping"));
    listed([KEPT]);
    dispose = render(() => <UnsavedRecovery />, root);
    button("Restore").click();
    await vi.waitFor(() => expect(toasts()).toHaveLength(1));
    expect(toasts()[0]).toMatchObject({ kind: "error" });
    expect(toasts()[0].message).toContain("the page host is stopping");
  });

  it("the graph-open list does not open the panel, and the same list mailed again does not either", () => {
    setRestorableSources([KEPT], true);
    expect(unsavedRecoveryOpen()).toBe(false);
    listed([KEPT]);
    expect(unsavedRecoveryOpen()).toBe(false);
    expect(restorableSources()).toEqual([KEPT]);
  });

  it("a list mailed for another session is dropped, and an empty-key mail reaches no page", () => {
    listed([KEPT], host.session + 1);
    expect(restorableSources()).toEqual([]);
    expect(unsavedRecoveryOpen()).toBe(false);
  });
});
