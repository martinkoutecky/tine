// GH #540 (master dc3f2104b): a close with unsaved pages names them, and "No"
// opens a recovery panel that retries the ordinary save, opens the page, or
// copies the draft. A failed save's toast leads to the same panel.
//
// The page host owns saving (STEP3 §4): it took P's input, its save keeps
// failing, and it says so with a save-error notice (three failed attempts) and
// waits for publication in vain until a retry publishes.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "./backend";
import { initParser } from "./render/parse";
import { loadFeed, pageByName, resetStore, setRaw, unsavedDrafts } from "./document";
import { bindTestHost, submittedPages, type TestHost } from "./document/host/wiring.test.support";
import { startEditing } from "./editorController";
import { safeClose } from "./App";
import { UnsavedRecovery } from "./components/UnsavedRecovery";
import { closeUnsavedRecovery, unsavedRecoveryOpen } from "./unsavedRecovery";
import { setToasts, toasts } from "./toasts";
import type { BlockDto } from "./types";

const PATH = "pages/P.md";
const block = (id: string, raw: string, children: BlockDto[] = []): BlockDto => ({ id, raw, collapsed: false, children });
const flush = async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); };
const reviewToasts = () => toasts().filter((t) => t.action?.label === "Review unsaved");

let failing = true;
let host: TestHost;
beforeAll(() => initParser());
beforeEach(async () => {
  resetStore();
  setToasts([]);
  failing = true;
  loadFeed([{ id: PATH, name: "P", title: "P", kind: "page", pre_block: null, rev: "r1",
    blocks: [block("p1", "first", [block("p2", "child")])] } as never]);
  host = await bindTestHost();
  // While failing, the host never publishes; a retry that works publishes the
  // taken version and clears the page's save-error notice.
  vi.spyOn(backend(), "pageOwed").mockImplementation(async () => failing ? [{ key: PATH, version: 2 }] : []);
  vi.spyOn(backend(), "pageWait").mockImplementation(async () => !failing);
  vi.spyOn(backend(), "pageSaveNow").mockImplementation(async (_session, keys) => {
    if (!failing) for (const key of keys) host.notice(key, {}, { version: 2 });
  });
  const submit = vi.spyOn(backend(), "pageSubmit");
  setRaw("p1", "typed draft");
  startEditing("p1");
  await vi.waitFor(() => expect(submit).toHaveBeenCalledOnce(), { timeout: 1000 });
  expect(submittedPages(submit, "P")[0].blocks[0].raw).toBe("typed draft");
  host.notice(PATH, { failures: 3, saveError: true }, { version: 2, risk: true });
  await vi.waitFor(() => expect(reviewToasts()).toHaveLength(1));
});
afterEach(() => {
  closeUnsavedRecovery();
  safeClose.reset();
  vi.restoreAllMocks();
  resetStore();
  setToasts([]);
  document.body.innerHTML = "";
});

describe("unsaved-changes recovery (GH #540)", () => {
  it("the close prompt names the page, and No opens the recovery panel", async () => {
    const confirm = vi.spyOn(backend(), "confirm").mockResolvedValue(false);
    expect(await safeClose.prepare()).toBe("rejected");
    expect(confirm).toHaveBeenCalledOnce();
    expect(confirm.mock.calls[0][0]).toContain("• P — Not saved");
    expect(unsavedRecoveryOpen()).toBe(true);
    expect(unsavedDrafts().map((d) => [d.name, d.state])).toEqual([["P", "Not saved"]]);
  });

  it("shows the draft as source text, copies it, and retries the ordinary save", async () => {
    const write = vi.spyOn(backend(), "writeText").mockResolvedValue();
    const [toast] = reviewToasts();
    expect(toast.sticky).toBe(true);
    expect(toast.message).toBe("Couldn't save “P” yet; Tine keeps trying and keeps a crash-recovery copy.");
    toast.action!.run(); // the failed-save toast leads to the panel
    expect(unsavedRecoveryOpen()).toBe(true);
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <UnsavedRecovery />, root);
    try {
      expect(root.querySelector("h3")?.textContent).toBe("P — Not saved");
      expect(root.querySelector("pre")?.textContent).toBe("- typed draft\n\t- child");
      const button = (label: string) => [...root.querySelectorAll("button")].find((b) => b.textContent === label)!;
      button("Copy draft").click();
      await flush();
      expect(write).toHaveBeenCalledWith("- typed draft\n\t- child");
      // Copying never acknowledges a save.
      expect(unsavedDrafts().map((d) => [d.name, d.state])).toEqual([["P", "Not saved"]]);

      button("Retry saving").click();
      await vi.waitFor(() => expect(root.textContent).toContain("Some changes still need attention"));
      expect(unsavedDrafts().map((d) => d.name)).toEqual(["P"]);

      failing = false;
      button("Retry saving").click();
      await vi.waitFor(() => expect(root.textContent).toContain("All pending changes saved"));
      expect(unsavedDrafts()).toEqual([]);
      expect(reviewToasts()).toEqual([]);
    } finally {
      dispose();
    }
  });

  it("a crash-recovery copy the host cannot write is one sticky error that leads to the panel", async () => {
    host.notice(PATH, { failures: 0, saveError: false, draftError: true }, { version: 2, risk: true });
    host.notice(PATH, { failures: 0, saveError: false, draftError: true }, { version: 2, risk: true });
    await vi.waitFor(() => expect(reviewToasts().map((t) => t.message))
      .toEqual(["Couldn't write the crash-recovery copy of “P”."]));
    const [toast] = reviewToasts();
    expect(toast).toMatchObject({ kind: "error", sticky: true });
    toast.action!.run();
    expect(unsavedRecoveryOpen()).toBe(true);
  });

  // Q2: a rename or delete its caller reported as unconfirmed later ended
  // with its draft durably absent; the page's notice says it did not happen.
  it("an unconfirmed operation that did not happen is one sticky error", async () => {
    host.notice(PATH, { failures: 0, saveError: false, dropped: true }, { version: 2 });
    await vi.waitFor(() => expect(toasts().filter((t) => t.message.includes("did not happen"))).toHaveLength(1));
    expect(toasts().find((t) => t.message.includes("did not happen"))).toMatchObject({ kind: "error", sticky: true });
  });

  it("belongs to the graph it was opened for", async () => {
    vi.spyOn(backend(), "confirm").mockResolvedValue(false);
    await safeClose.prepare();
    expect(unsavedRecoveryOpen()).toBe(true);
    resetStore(); // graph switch
    expect(unsavedRecoveryOpen()).toBe(false);
    expect(pageByName("P")).toBeUndefined();
  });
});
