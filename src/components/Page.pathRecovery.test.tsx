import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { applyGraphChange, applyGraphChangesBulk, resetStore } from "../document";
import { initParser } from "../render/parse";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import { setGraphMeta } from "../graphSession";
import { setToasts } from "../toasts";
import type { PageRead } from "../types";
import { PageView } from "./Page";

beforeAll(initParser);

it.each(["disk", "held-bytes", "held-unindexed"] as const)(
  "records %s when a concrete but different page answers the route", async source => {
    const diagnostic = vi.spyOn(backend(), "diagnosticFrontendEvent").mockResolvedValue();
    vi.spyOn(backend(), "getPageByPath").mockImplementation(async (_path, receiveSource) => {
      receiveSource?.(source);
      return { id: "pages/Other.md", name: "Other", title: "Other", kind: "page", blocks: [], pre_block: null };
    });
    mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Chosen", pageKind: "page", path: "pages/Chosen.md" });
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <PageView />, root);
    try {
      await vi.waitFor(() => expect(root.textContent).toContain("no longer available at that path"));
      expect(diagnostic).toHaveBeenCalledExactlyOnceWith("page_open_refused", { pageOpen: {
        stage: "route", readSource: source, pathMismatch: true, nameMismatch: true, kindMismatch: false,
      } });
    } finally { dispose(); }
  },
);

it.each(["disk", "held-bytes", "held-absent", "held-unindexed"] as const)(
  "exports the %s answer's source and mismatch flags on route refusal and recovery probe", async source => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const diagnostic = vi.spyOn(backend(), "diagnosticFrontendEvent").mockResolvedValue();
    const page: PageRead = { id: "pages/Other.md", name: "Other", title: "Other", kind: "journal",
      pre_block: null, blocks: [] };
    const read = vi.spyOn(backend(), "getPageByPath").mockImplementation(async (_path, receiveSource) => {
      receiveSource?.(source);
      return null;
    });
    mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Private title", pageKind: "page", path: "pages/Private title.md" });
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <PageView />, root);
    try {
      await vi.advanceTimersByTimeAsync(0);
      expect(root.textContent).toContain("no longer available at that path");
      expect(diagnostic).toHaveBeenCalledWith("page_open_refused", { pageOpen: {
        stage: "route", readSource: source, pathMismatch: true, nameMismatch: true, kindMismatch: true,
      } });
      read.mockImplementation(async (_path, receiveSource) => { receiveSource?.(source); return page; });
      await vi.advanceTimersByTimeAsync(3000);
      expect(diagnostic).toHaveBeenLastCalledWith("page_open_refused", { pageOpen: {
        stage: "probe", readSource: source, pathMismatch: true, nameMismatch: true, kindMismatch: true,
      } });
      expect(read).toHaveBeenCalledTimes(2);
      expect(diagnostic).toHaveBeenCalledTimes(2);
      await vi.advanceTimersByTimeAsync(6000 + 12_000);
      expect(read).toHaveBeenCalledTimes(4);
      expect(diagnostic).toHaveBeenCalledTimes(2);
      expect(JSON.stringify(diagnostic.mock.calls)).not.toMatch(/Private|Other|\.md/);
    } finally { dispose(); }
  },
);

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  resetStore();
  setGraphMeta(null);
  setToasts([]);
  document.body.innerHTML = "";
  resetTabsToJournals();
});

it("keeps a still-missing route's refusal visible while silent probes settle and back off to one minute", async () => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  const read = vi.spyOn(backend(), "getPageByPath").mockResolvedValue(null);
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Missing", pageKind: "page", path: "pages/Missing.md" });
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => <PageView />, root);
  const announcements: string[] = [];
  const observer = new MutationObserver(() => {
    for (const status of root.querySelectorAll('[role="status"]')) announcements.push(status.textContent ?? "");
  });
  try {
    await vi.advanceTimersByTimeAsync(0);
    expect(root.textContent).toContain("no longer available at that path");
    const refusal = root.textContent;
    observer.observe(root, { childList: true, subtree: true });
    let finishProbe!: (page: PageRead | null) => void;
    read.mockImplementation(() => new Promise(resolve => { finishProbe = resolve; }));
    for (const delay of [3000, 6000, 12000, 24000, 48000, 60000, 60000]) {
      const previousReads = read.mock.calls.length;
      await vi.advanceTimersByTimeAsync(delay - 1);
      expect(read).toHaveBeenCalledTimes(previousReads);
      await vi.advanceTimersByTimeAsync(1);
      expect(read).toHaveBeenCalledTimes(previousReads + 1);
      expect(read).toHaveBeenLastCalledWith("pages/Missing.md", expect.any(Function));
      expect(root.textContent).toBe(refusal);
      expect(root.querySelector('[role="status"]')).toBeNull();
      // A slow read must finish before the next delay starts.
      await vi.advanceTimersByTimeAsync(60_000);
      expect(read).toHaveBeenCalledTimes(previousReads + 1);
      finishProbe(null);
      await vi.advanceTimersByTimeAsync(0);
      expect(root.textContent).toBe(refusal);
    }
    expect(announcements).toEqual([]);
  } finally { observer.disconnect(); dispose(); }
});

it("opens a restored file without a watcher event and cancels retries after opening", async () => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  const diagnostic = vi.spyOn(backend(), "diagnosticFrontendEvent").mockResolvedValue();
  const page: PageRead = { name: "Restored", title: "Restored", kind: "page", id: "pages/Restored.md", rev: "unchanged",
    pre_block: null, blocks: [{ id: "restored", raw: "Identical bytes restored by sync", children: [], collapsed: false }] };
  const read = vi.spyOn(backend(), "getPageByPath").mockResolvedValue(null);
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: page.name, pageKind: "page", path: page.id });
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => <PageView />, root);
  try {
    await vi.waitFor(() => expect(root.textContent).toContain("no longer available at that path"));
    read.mockResolvedValue(page);
    await vi.advanceTimersByTimeAsync(3500);
    expect(root.textContent).toContain("Identical bytes restored by sync");
    read.mockResolvedValue(null);
    await vi.advanceTimersByTimeAsync(30_000);
    expect(root.textContent).toContain("Identical bytes restored by sync");
    expect(root.textContent).not.toContain("no longer available at that path");
    expect(diagnostic).toHaveBeenCalledTimes(1);
  } finally { dispose(); }
});

it("resets the probe delay when a publication revalidates a still-missing route", async () => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  const read = vi.spyOn(backend(), "getPageByPath").mockResolvedValue(null);
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Missing", pageKind: "page", path: "pages/Missing.md" });
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => <PageView />, root);
  try {
    await vi.advanceTimersByTimeAsync(0);
    // Reach a 24-second backoff, then publish while that timer is armed.
    await vi.advanceTimersByTimeAsync(21_000);
    expect(read).toHaveBeenCalledTimes(4);
    await applyGraphChange({ name: "Missing", kind: "page", path: "pages/Missing.md", created: true, removed: false });
    await vi.advanceTimersByTimeAsync(0);
    expect(root.textContent).toContain("no longer available at that path");
    const afterPublication = read.mock.calls.length;
    expect(afterPublication).toBeGreaterThan(4);
    await vi.advanceTimersByTimeAsync(2999);
    expect(read).toHaveBeenCalledTimes(afterPublication);
    await vi.advanceTimersByTimeAsync(1);
    expect(read).toHaveBeenCalledTimes(afterPublication + 1);
    await vi.advanceTimersByTimeAsync(5999);
    expect(read).toHaveBeenCalledTimes(afterPublication + 1);
    await vi.advanceTimersByTimeAsync(1);
    expect(read).toHaveBeenCalledTimes(afterPublication + 2);
  } finally { dispose(); }
});

it.each([
  { id: "pages/Twin.org", name: "Twin", kind: "page" as const },
  { id: "pages/Twin.md", name: "Renamed", kind: "page" as const },
])("keeps a probe silent when the returned identity differs: %j", async identity => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  const diagnostic = vi.spyOn(backend(), "diagnosticFrontendEvent").mockResolvedValue();
  const read = vi.spyOn(backend(), "getPageByPath").mockResolvedValue(null);
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Twin", pageKind: "page", path: "pages/Twin.md" });
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => <PageView />, root);
  try {
    await vi.advanceTimersByTimeAsync(0);
    const refusal = root.textContent;
    read.mockResolvedValue({ ...identity, title: identity.name, rev: "other", pre_block: null,
      blocks: [{ id: "other", raw: "A different identity", children: [], collapsed: false }] });
    await vi.advanceTimersByTimeAsync(3000);
    expect(read).toHaveBeenCalledTimes(2);
    expect(root.textContent).toBe(refusal);
    expect(diagnostic).toHaveBeenLastCalledWith("page_open_refused", { pageOpen: {
      stage: "probe", readSource: null,
      pathMismatch: identity.id !== "pages/Twin.md", nameMismatch: identity.name !== "Twin", kindMismatch: identity.kind !== "page",
    } });
    expect(mainPaneRouter.route()).toMatchObject({ name: "Twin", pageKind: "page", path: "pages/Twin.md" });
    await vi.advanceTimersByTimeAsync(6000);
    expect(read).toHaveBeenCalledTimes(3);
    expect(root.textContent).toBe(refusal);
  } finally { dispose(); }
});

it("opens a restored file under the store's kind when the route remembered another (GH #863)", async () => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  vi.spyOn(backend(), "diagnosticFrontendEvent").mockResolvedValue();
  const read = vi.spyOn(backend(), "getPageByPath").mockResolvedValue(null);
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Twin", pageKind: "page", path: "pages/Twin.md" });
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => <PageView />, root);
  try {
    await vi.advanceTimersByTimeAsync(0);
    read.mockResolvedValue({ id: "pages/Twin.md", name: "Twin", title: "Twin", kind: "journal", rev: "restored", pre_block: null,
      blocks: [{ id: "restored", raw: "The restored day", children: [], collapsed: false }] });
    await vi.advanceTimersByTimeAsync(3000);
    await vi.waitFor(() => expect(root.textContent).toContain("The restored day"));
    expect(mainPaneRouter.route()).toMatchObject({ name: "Twin", pageKind: "journal", path: "pages/Twin.md" });
  } finally { dispose(); }
});

it("leaves a later navigation alone when the refused path becomes available", async () => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  const page: PageRead = { name: "Elsewhere", title: "Elsewhere", kind: "page", id: "pages/Elsewhere.md", rev: "r",
    pre_block: null, blocks: [{ id: "elsewhere", raw: "The page chosen afterwards", children: [], collapsed: false }] };
  vi.spyOn(backend(), "getPageByPath").mockResolvedValue(null);
  vi.spyOn(backend(), "getPage").mockResolvedValue(page);
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Missing", pageKind: "page", path: "pages/Missing.md" });
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => <PageView />, root);
  try {
    await vi.waitFor(() => expect(root.textContent).toContain("no longer available at that path"));
    mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Elsewhere", pageKind: "page" });
    await vi.waitFor(() => expect(root.textContent).toContain("The page chosen afterwards"));
    await vi.advanceTimersByTimeAsync(30_000);
    expect(root.textContent).toContain("The page chosen afterwards");
    expect(mainPaneRouter.route()).toMatchObject({ name: "Elsewhere" });
  } finally { dispose(); }
});

it("recovers when publication overtakes the missing-file read without leaving an older retry armed", async () => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  const page: PageRead = { name: "Arriving", title: "Arriving", kind: "page", id: "pages/Arriving.md", rev: "r",
    pre_block: null, blocks: [{ id: "arriving", raw: "The delivered physical page", children: [], collapsed: false }] };
  let finishRead!: (page: PageRead | null) => void;
  const read = vi.spyOn(backend(), "getPageByPath").mockResolvedValue(page)
    .mockImplementationOnce(() => new Promise(resolve => { finishRead = resolve; }));
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: page.name, pageKind: "page", path: page.id });
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => <PageView />, root);
  try {
    await vi.waitFor(() => expect(finishRead).toBeTypeOf("function"));
    await applyGraphChange({ name: page.name, kind: "page", path: page.id!, created: true, removed: false });
    finishRead(null);
    await vi.waitFor(() => expect(root.textContent).toContain("The delivered physical page"));
    read.mockResolvedValue(null);
    await vi.advanceTimersByTimeAsync(30_000);
    expect(root.textContent).toContain("The delivered physical page");
  } finally { dispose(); }
});

it.each(["journal", "page"] as const)("opens a refused physical %s route when a late watcher publication brings its file back (GH #863)", async kind => {
  const name = kind === "journal" ? "2026-11-07" : "Delivered page";
  const page: PageRead = { name, title: name, kind,
    id: kind === "journal" ? "journals/2026_11_07.md" : "pages/Delivered page.md", rev: "delivered", pre_block: null,
    blocks: [{ id: "delivered-block", raw: "Journal delivered after resume", children: [], collapsed: false }] };
  const read = vi.spyOn(backend(), "getPageByPath").mockResolvedValue(null);
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: page.name, pageKind: kind, path: page.id });
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => <PageView />, root);
  try {
    await vi.waitFor(() => expect(root.textContent).toContain("no longer available at that path"));
    read.mockResolvedValue(page);
    const change = { name: page.name, kind, path: page.id!, created: true, removed: false };
    if (kind === "journal") await applyGraphChange(change);
    else await applyGraphChangesBulk({ changes: [change] });
    await vi.waitFor(() => expect(root.textContent).toContain("Journal delivered after resume"));
    expect(root.textContent).not.toContain("no longer available at that path");
    expect(mainPaneRouter.route()).toMatchObject({ path: page.id });
  } finally { dispose(); }
});

it("keeps a missing physical path pinned when only a same-name sibling exists", async () => {
  const sibling: PageRead = { name: "Twin", title: "Twin", kind: "page", id: "pages/Twin.org", rev: "sibling",
    pre_block: null, blocks: [{ id: "sibling", raw: "A different physical file", children: [], collapsed: false }] };
  vi.spyOn(backend(), "getPageByPath").mockResolvedValue(null);
  vi.spyOn(backend(), "getPage").mockResolvedValue(sibling);
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Twin", pageKind: "page", path: "pages/Twin.md" });
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => <PageView />, root);
  try {
    await vi.waitFor(() => expect(root.textContent).toContain("no longer available at that path"));
    await applyGraphChange({ name: "Twin", kind: "page", path: "pages/Twin.org", created: true, removed: false });
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(root.textContent).toContain("no longer available at that path");
    expect(root.textContent).not.toContain("A different physical file");
    expect(mainPaneRouter.route()).toMatchObject({ path: "pages/Twin.md" });
  } finally { dispose(); }
});

it("shows an unreadable pinned file's read error, never as missing, and opens it on the later publication (GH #863)", async () => {
  const page: PageRead = { name: "Locked", title: "Locked", kind: "page", id: "pages/Locked.md", rev: "readable",
    pre_block: null, blocks: [{ id: "locked", raw: "Readable once the lock is released", children: [], collapsed: false }] };
  const read = vi.spyOn(backend(), "getPageByPath")
    .mockRejectedValue(new Error("The process cannot access the file because it is being used by another process."));
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: page.name, pageKind: "page", path: page.id });
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => <PageView />, root);
  try {
    await vi.waitFor(() => expect(root.textContent).toContain("being used by another process"));
    expect(root.textContent).not.toContain("no longer available at that path");
    read.mockResolvedValue(page);
    await applyGraphChange({ name: page.name, kind: "page", path: page.id!, created: false, removed: false });
    await vi.waitFor(() => expect(root.textContent).toContain("Readable once the lock is released"));
    expect(root.textContent).not.toContain("being used by another process");
  } finally { dispose(); }
});

it("probes an unreadable pinned file and opens it once it reads, with no publication (GH #863)", async () => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  const page: PageRead = { name: "Locked", title: "Locked", kind: "page", id: "pages/Locked.md", rev: "readable",
    pre_block: null, blocks: [{ id: "locked", raw: "Readable once the lock is released", children: [], collapsed: false }] };
  const read = vi.spyOn(backend(), "getPageByPath").mockRejectedValue(new Error("Input/output error (os error 5)"));
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: page.name, pageKind: "page", path: page.id });
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => <PageView />, root);
  try {
    await vi.advanceTimersByTimeAsync(0);
    expect(root.textContent).toContain("os error 5");
    expect(root.textContent).not.toContain("no longer available at that path");
    // A probe that still fails keeps the error and backs off.
    await vi.advanceTimersByTimeAsync(3000);
    expect(read).toHaveBeenCalledTimes(2);
    expect(root.textContent).toContain("os error 5");
    read.mockResolvedValue(page);
    await vi.advanceTimersByTimeAsync(6000);
    expect(root.textContent).toContain("Readable once the lock is released");
  } finally { dispose(); }
});
