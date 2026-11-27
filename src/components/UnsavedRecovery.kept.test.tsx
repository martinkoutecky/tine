// GH #540: the recovery panel's kept originals. A cancelled rename kept a
// page's original text after its file was removed; the panel lists it with
// Copy original text. Copy hands the exact text to the clipboard writer; a
// binary original says "not text" and offers no Copy; an answer for a panel,
// graph or host session that is gone is dropped, and a retired owner puts
// nothing on the clipboard.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { resetStore, type KeptOriginal, type KeptText } from "../document";
import { bindTestHost, type TestHost } from "../document/host/wiring.test.support";
import { invalidateBinding } from "../binding";
import { UnsavedRecovery } from "./UnsavedRecovery";
import { closeUnsavedRecovery, openUnsavedRecovery, unsavedRecoveryOpen } from "../unsavedRecovery";
import { paletteCommands } from "../keybindings";
import { noticeKeptOriginals } from "../graph";
import { setToasts, toasts } from "../toasts";

const ROW: KeptOriginal = { key: "pages/a.md", origin: 7, path: "pages/a.md", name: "a", kind: "page", len: 7, text: true };
const flush = async () => { for (let i = 0; i < 8; i++) await Promise.resolve(); };

/** A promise and its resolver: an answer the test releases. */
function later<T>(): { promise: Promise<T>; resolve: (value: T) => void } {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

let host: TestHost;
let rows: KeptOriginal[] | null;
let root: HTMLDivElement;
let dispose: (() => void) | undefined;
const COPY = "Copy original text";
const buttons = (label: string) => [...root.querySelectorAll("button")].filter((b) => b.textContent === label);
const status = () => root.querySelector(".unsaved-recovery-entry p[role=status]")?.textContent ?? "";

beforeEach(async () => {
  resetStore();
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
  document.body.innerHTML = "";
});

async function openPanel(rowsShown = 1) {
  openUnsavedRecovery();
  dispose = render(() => <UnsavedRecovery />, root);
  await vi.waitFor(() => expect(root.querySelectorAll(".recovery-kept-row")).toHaveLength(rowsShown));
}

describe("kept originals in the recovery panel (GH #540)", () => {
  it("lists the original for its session and Copy hands the clipboard writer its exact CRLF text", async () => {
    vi.spyOn(backend(), "pageKeptText").mockResolvedValue({ kind: "text", text: "- old\r\n" });
    const write = vi.spyOn(backend(), "writeText").mockResolvedValue();
    await openPanel();
    expect(backend().pageKeptOriginals).toHaveBeenCalledWith(host.session);
    expect(root.textContent).toContain("Kept after a cancelled rename");
    expect(root.textContent).not.toContain("Restore");
    buttons(COPY)[0].click();
    await vi.waitFor(() => expect(status()).toBe("Copied. Paste it into a separate file to keep a copy."));
    expect(backend().pageKeptText).toHaveBeenCalledWith(host.session, "pages/a.md", 7);
    expect(write).toHaveBeenCalledTimes(1);
    expect(write.mock.calls[0][0]).toBe("- old\r\n");
  });

  it("a binary row and a hidden text row from one List: not text without Copy, and Copy", async () => {
    rows = [
      { key: "journals/Loose.md", origin: 3, path: "journals/Loose.md", name: null, kind: null, len: 3, text: false },
      { key: "pages/x.md", origin: 4, path: "pages/x.md", name: null, kind: null, len: 3, text: true },
    ];
    await openPanel(2);
    const [binary, text] = [...root.querySelectorAll(".recovery-kept-row")];
    expect(binary.textContent).toContain("Not text");
    expect(binary.querySelector("button")).toBeNull();
    expect(text.querySelector("button")?.textContent).toBe(COPY);
    expect(backend().pageKeptOriginals).toHaveBeenCalledTimes(1);
  });

  it("a Copy answer that is not text puts nothing on the clipboard", async () => {
    vi.spyOn(backend(), "pageKeptText").mockResolvedValue({ kind: "notText", len: 3 });
    const write = vi.spyOn(backend(), "writeText").mockResolvedValue();
    await openPanel();
    buttons(COPY)[0].click();
    await vi.waitFor(() => expect(status()).toContain("Not text"));
    expect(write).not.toHaveBeenCalled();
  });

  it("no longer kept only for Missing; Unavailable says so and keeps the last list", async () => {
    const answers: KeptText[] = [{ kind: "unavailable" }, { kind: "missing" }];
    vi.spyOn(backend(), "pageKeptText").mockImplementation(async () => answers.shift()!);
    const write = vi.spyOn(backend(), "writeText").mockResolvedValue();
    await openPanel();
    buttons(COPY)[0].click();
    await vi.waitFor(() => expect(status()).toBe("Recovery is unavailable right now."));
    expect(buttons(COPY)).toHaveLength(1);
    rows = [];
    buttons(COPY)[0].click();
    await vi.waitFor(() => expect(status()).toBe("This original is no longer kept."));
    await vi.waitFor(() => expect(buttons(COPY)).toHaveLength(0));
    expect(write).not.toHaveBeenCalled();
  });

  it("a List that is not answered keeps the last list and says recovery is unavailable", async () => {
    await openPanel();
    rows = null;
    await bindTestHost();
    await vi.waitFor(() => expect(status()).toBe("Recovery is unavailable right now."));
    expect(buttons(COPY)).toHaveLength(1);
  });

  it("lists on open and on a rebind, never in the panel's poll", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      await openPanel();
      expect(backend().pageKeptOriginals).toHaveBeenCalledTimes(1);
      await vi.advanceTimersByTimeAsync(5_000);
      expect(backend().pageKeptOriginals).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it("a rebind relists for the new session and drops the old list's late answer", async () => {
    const first = later<KeptOriginal[] | null>();
    const lists = vi.spyOn(backend(), "pageKeptOriginals")
      .mockReturnValueOnce(first.promise)
      .mockImplementation(async () => [{ ...ROW, origin: 9 }]);
    openUnsavedRecovery();
    dispose = render(() => <UnsavedRecovery />, root);
    await vi.waitFor(() => expect(lists).toHaveBeenCalledTimes(1));
    const rebound = await bindTestHost();
    await vi.waitFor(() => expect(lists).toHaveBeenLastCalledWith(rebound.session));
    await vi.waitFor(() => expect(buttons(COPY)).toHaveLength(1));
    first.resolve([ROW, { ...ROW, key: "pages/old.md" }]);
    await flush();
    expect(root.querySelectorAll(".recovery-kept-row")).toHaveLength(1);
    const copy = vi.spyOn(backend(), "pageKeptText").mockResolvedValue({ kind: "missing" });
    buttons(COPY)[0].click();
    await vi.waitFor(() => expect(copy).toHaveBeenCalledWith(rebound.session, "pages/a.md", 9));
  });

  it("a Copy read answered across a rebind is dropped: nothing reaches the clipboard", async () => {
    const answer = later<KeptText>();
    vi.spyOn(backend(), "pageKeptText").mockReturnValue(answer.promise);
    const write = vi.spyOn(backend(), "writeText").mockResolvedValue();
    await openPanel();
    buttons(COPY)[0].click();
    await bindTestHost();
    answer.resolve({ kind: "text", text: "- old\n" });
    await flush();
    expect(write).not.toHaveBeenCalled();
  });

  it("a graph switch after the read answers and before the clipboard transport dispatches writes nothing and shows nothing", async () => {
    vi.spyOn(backend(), "pageKeptText").mockResolvedValue({ kind: "text", text: "- old\n" });
    // The transport yields (the native clipboard plugin's import) before it
    // dispatches, re-checking the caller's owner first, as `TauriBackend` does.
    const gate = later<void>();
    const dispatched: string[] = [];
    vi.spyOn(backend(), "writeText").mockImplementation(async (text, still = () => true) => {
      await gate.promise;
      if (still()) dispatched.push(text);
    });
    await openPanel();
    buttons(COPY)[0].click();
    await vi.waitFor(() => expect(backend().writeText).toHaveBeenCalledTimes(1));
    invalidateBinding();
    expect(unsavedRecoveryOpen()).toBe(false);
    gate.resolve();
    await flush();
    expect(dispatched).toEqual([]);
    expect(document.body.textContent).not.toContain("Copied");
  });

  it("the palette offers the panel in a bound graph whatever the launch count", async () => {
    const command = paletteCommands(null).find((c) => c.id === "recovery/review-kept-originals");
    expect(command).toBeDefined();
    command!.run();
    expect(unsavedRecoveryOpen()).toBe(true);
  });

  it("the launch notice's Review opens its own graph's panel and retires with that graph", async () => {
    setToasts([]);
    noticeKeptOriginals(1);
    const [notice] = toasts();
    expect(notice.action).toBeDefined();
    invalidateBinding();
    expect(toasts()).toEqual([]);
    notice.action!.run();
    expect(unsavedRecoveryOpen()).toBe(false);
    noticeKeptOriginals(2);
    toasts()[0].action!.run();
    expect(unsavedRecoveryOpen()).toBe(true);
    setToasts([]);
  });
});
