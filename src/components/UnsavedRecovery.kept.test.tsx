// GH #540: the recovery panel's kept originals. A cancelled rename kept a
// page's original text after its file was removed; the panel lists it with
// Restore and Copy original. Each outcome is reported once, by one producer
// (`reportRestore`); an answer for a graph or host session that is gone is
// dropped; a save the page's own editor reports adds no toast.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { resetStore, type KeptOriginal, type RestoreOutcome } from "../document";
import { bindTestHost, type TestHost } from "../document/host/wiring.test.support";
import { invalidateBinding } from "../binding";
import { UnsavedRecovery } from "./UnsavedRecovery";
import { closeUnsavedRecovery, keptOriginalCount, openUnsavedRecovery, unsavedRecoveryOpen } from "../unsavedRecovery";
import { setToasts, toasts } from "../toasts";
import { render as renderMessage } from "../i18n";

const ROW: KeptOriginal = { key: "pages/a.md", origin: 7, path: "pages/a.md", name: "a", kind: "page", len: 6 };
const flush = async () => { for (let i = 0; i < 8; i++) await Promise.resolve(); };
const texts = () => toasts().map((t) => typeof t.message === "string" ? t.message : renderMessage(t.message));

/** A promise and its resolver: an answer the test releases. */
function later<T>(): { promise: Promise<T>; resolve: (value: T) => void } {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

let host: TestHost;
let rows: KeptOriginal[];
let root: HTMLDivElement;
let dispose: (() => void) | undefined;
const button = (label: string) => [...root.querySelectorAll("button")].find((b) => b.textContent === label);

beforeEach(async () => {
  resetStore();
  setToasts([]);
  host = await bindTestHost();
  rows = [ROW];
  vi.spyOn(backend(), "pageKeptOriginals").mockImplementation(async () => rows);
  root = document.createElement("div");
  document.body.appendChild(root);
});
afterEach(() => {
  dispose?.();
  dispose = undefined;
  closeUnsavedRecovery();
  vi.restoreAllMocks();
  resetStore();
  setToasts([]);
  document.body.innerHTML = "";
});

async function openPanel() {
  openUnsavedRecovery();
  dispose = render(() => <UnsavedRecovery />, root);
  await vi.waitFor(() => expect(button("Restore")).toBeDefined());
}

describe("kept originals in the recovery panel (GH #540)", () => {
  it("lists the original for its session, restores it with one toast, and offers Open", async () => {
    const restore = vi.spyOn(backend(), "pageRestoreKept").mockImplementation(async () => {
      rows = [];
      return { kind: "restored" };
    });
    await openPanel();
    expect(backend().pageKeptOriginals).toHaveBeenCalledWith(host.session);
    expect(root.textContent).toContain("Kept after a cancelled rename");
    expect(keptOriginalCount()).toBe(1);
    button("Restore")!.click();
    await vi.waitFor(() => expect(texts()).toEqual(["Restored pages/a.md."]));
    expect(restore).toHaveBeenCalledWith(host.session, "pages/a.md", 7);
    await vi.waitFor(() => expect(button("Restore")).toBeUndefined());
    expect(keptOriginalCount()).toBe(0);
    expect(button("Open")).toBeDefined();
    expect(texts()).toHaveLength(1);
  });

  it("a refusal says why once and keeps the row", async () => {
    vi.spyOn(backend(), "pageRestoreKept").mockResolvedValue({ kind: "refused", reason: "held", name: "a" });
    await openPanel();
    button("Restore")!.click();
    await vi.waitFor(() => expect(texts()).toEqual(["Close a, then Restore."]));
    await flush();
    expect(button("Restore")).toBeDefined();
    expect(texts()).toHaveLength(1);
  });

  it("other content first: says so, offers Open, and keeps the original listed", async () => {
    vi.spyOn(backend(), "pageRestoreKept").mockResolvedValue({ kind: "superseded" });
    await openPanel();
    button("Restore")!.click();
    await vi.waitFor(() => expect(texts()).toEqual(["pages/a.md now holds other content. The original stays kept while Tine has it."]));
    await vi.waitFor(() => expect(button("Open")).toBeDefined());
    expect(button("Restore")).toBeDefined();
  });

  it("an unwritten save is reported once: by the panel, or by the editor when a window holds the page", async () => {
    const outcomes: RestoreOutcome[] = [{ kind: "unwritten", held: false }, { kind: "unwritten", held: true }];
    vi.spyOn(backend(), "pageRestoreKept").mockImplementation(async () => outcomes.shift()!);
    await openPanel();
    button("Restore")!.click();
    await vi.waitFor(() => expect(texts()).toEqual(["pages/a.md is not saved yet; the original stays kept."]));
    await vi.waitFor(() => expect(button("Restore")?.disabled).toBe(false));
    button("Restore")!.click();
    await flush();
    await vi.waitFor(() => expect(backend().pageRestoreKept).toHaveBeenCalledTimes(2));
    await flush();
    expect(texts()).toHaveLength(1);
  });

  it("an answer arriving after a graph switch reports nothing", async () => {
    const answer = later<RestoreOutcome>();
    vi.spyOn(backend(), "pageRestoreKept").mockReturnValue(answer.promise);
    await openPanel();
    button("Restore")!.click();
    await vi.waitFor(() => expect(button("Restoring…")).toBeDefined());
    invalidateBinding();
    expect(unsavedRecoveryOpen()).toBe(false);
    answer.resolve({ kind: "restored" });
    await flush();
    expect(texts()).toEqual([]);
  });

  it("a rebind relists for the new session and drops the old list's late answer", async () => {
    const first = later<KeptOriginal[]>();
    const lists = vi.spyOn(backend(), "pageKeptOriginals")
      .mockReturnValueOnce(first.promise)
      .mockImplementation(async () => [{ ...ROW, origin: 9 }]);
    openUnsavedRecovery();
    dispose = render(() => <UnsavedRecovery />, root);
    await vi.waitFor(() => expect(lists).toHaveBeenCalledTimes(1));
    const rebound = await bindTestHost();
    await vi.waitFor(() => expect(lists).toHaveBeenLastCalledWith(rebound.session));
    await vi.waitFor(() => expect(button("Restore")).toBeDefined());
    first.resolve([ROW]);
    await flush();
    const restore = vi.spyOn(backend(), "pageRestoreKept").mockResolvedValue({ kind: "refused", reason: "notKept" });
    button("Restore")!.click();
    await vi.waitFor(() => expect(restore).toHaveBeenCalledWith(rebound.session, "pages/a.md", 9));
    expect(root.querySelectorAll(".recovery-kept-row")).toHaveLength(1);
  });

  it("Copy original copies the kept text", async () => {
    vi.spyOn(backend(), "pageKeptText").mockResolvedValue("- old\r\n");
    const write = vi.spyOn(backend(), "writeText").mockResolvedValue();
    await openPanel();
    button("Copy original")!.click();
    await vi.waitFor(() => expect(write).toHaveBeenCalledWith("- old\r\n"));
    await vi.waitFor(() => expect(root.textContent).toContain("Copied. Paste it into a separate file to keep a copy."));
    expect(backend().pageKeptText).toHaveBeenCalledWith(host.session, "pages/a.md", 7);
  });
});
