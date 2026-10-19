import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { PageView, reloadJournalsFeedFromStart } from "./Page";
import { resetTabsToJournals, restoreSession } from "../router";
import { graphEpoch, setGraphMeta } from "../graphSession";
import { toasts, setToasts } from "../toasts";
import { localDayKey } from "../journal";
import type { GraphMeta } from "../types";
import { ensureJournalTemplateForDay, type JournalTemplateEnsureResult } from "../graph";

vi.mock("../graph", async (original) => ({
  ...await original<typeof import("../graph")>(),
  ensureJournalTemplateForDay: vi.fn(async () => "ready"),
}));

beforeAll(async () => { await initParser(); });
afterEach(async () => {
  // Each case owns a fresh graph. Settle shared deferred refresh state before
  // restoring the mock backend; otherwise the next mount inherits its retry.
  setGraphMeta(null);
  vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({
    pages: [], next_before_day: null, done: true, as_of_day: localDayKey(),
  });
  await reloadJournalsFeedFromStart({ graphEpoch: graphEpoch(), isLive: () => true });
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.restoreAllMocks();
  resetStore();
  setGraphMeta(null);
  document.body.innerHTML = "";
  resetTabsToJournals();
  setToasts([]);
});
function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

describe("journal feed read failures (GH #385, master d6024bac3aad)", () => {
  it("transfers pending template work to the replacement startup surface (GH #671)", async () => {
    setGraphMeta({ root: "/private/graph", default_journal_template: "Private template" } as GraphMeta);
    let canWrite: (() => boolean) | undefined;
    let resolveEnsure!: (result: JournalTemplateEnsureResult) => void;
    const pending = new Promise<JournalTemplateEnsureResult>((resolve) => { resolveEnsure = resolve; });
    // The real graph ensure coalesces by graph/template/day and retains its
    // first write guard while the backend page/template read is in flight.
    vi.mocked(ensureJournalTemplateForDay).mockImplementation((_date, guard) => {
      canWrite ??= guard;
      return pending;
    });
    const read = vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({
      pages: [], next_before_day: null, done: true, as_of_day: localDayKey(),
    });
    const first = mount(() => <PageView />);
    await vi.waitFor(() => expect(canWrite).toBeDefined());
    first.dispose();
    const replacement = mount(() => <PageView />);
    try {
      await Promise.resolve();
      resolveEnsure(canWrite!() ? "ready" : "deferred");
      await vi.waitFor(() => expect(replacement.root.querySelector(".journal-title")).not.toBeNull());
      expect(replacement.root.textContent).not.toContain("Journal feed read failed");
      expect(read).toHaveBeenCalledTimes(1);
    } finally {
      replacement.dispose();
      vi.mocked(ensureJournalTemplateForDay).mockResolvedValue("ready");
    }
  });

  it("keeps transferred startup work owned through the feed read after template ensure (GH #671)", async () => {
    setGraphMeta({ root: "/private/graph", default_journal_template: "Private template" } as GraphMeta);
    vi.mocked(ensureJournalTemplateForDay).mockResolvedValue("ready");
    let resolveFeed!: (feed: import("../types").JournalFeedPage) => void;
    const read = vi.spyOn(backend(), "journalFeedPage").mockImplementation(() => new Promise((resolve) => { resolveFeed = resolve; }));
    const first = mount(() => <PageView />);
    await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(1));
    first.dispose();
    const replacement = mount(() => <PageView />);
    try {
      await Promise.resolve();
      resolveFeed({ pages: [], next_before_day: null, done: true, as_of_day: localDayKey() });
      await vi.waitFor(() => expect(replacement.root.querySelector(".journal-title")).not.toBeNull());
      expect(replacement.root.textContent).not.toContain("Journal feed read failed");
      expect(read).toHaveBeenCalledTimes(1);
    } finally { replacement.dispose(); }
  });

  it.each(["deferred", "stale"] as const)("records template %s without the template name or graph path (GH #671)", async (result) => {
    setGraphMeta({ root: "/private/graph", default_journal_template: "Private template" } as GraphMeta);
    vi.mocked(ensureJournalTemplateForDay).mockResolvedValueOnce(result);
    const diagnostic = vi.spyOn(backend(), "diagnosticFrontendEvent").mockResolvedValue();
    const read = vi.spyOn(backend(), "journalFeedPage");
    const mounted = mount(() => <PageView />);
    try {
      await vi.waitFor(() => expect(mounted.root.textContent).toContain("Journal feed read failed"));
      expect(read).not.toHaveBeenCalled();
      expect(diagnostic).toHaveBeenCalledWith("journal_feed", { journalFeedReason: `template-${result}` });
      expect(JSON.stringify(diagnostic.mock.calls)).not.toContain("Private template");
      expect(JSON.stringify(diagnostic.mock.calls)).not.toContain("/private/graph");
    } finally { mounted.dispose(); }
  });

  it("records a repeated native/frontend day mismatch (GH #671)", async () => {
    vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({
      pages: [], next_before_day: null, done: true, as_of_day: localDayKey() - 1,
    });
    const diagnostic = vi.spyOn(backend(), "diagnosticFrontendEvent").mockResolvedValue();
    const mounted = mount(() => <PageView />);
    try {
      await vi.waitFor(() => expect(mounted.root.textContent).toContain("Journal feed read failed"));
      expect(diagnostic).toHaveBeenCalledWith("journal_feed", { journalFeedReason: "day-mismatch" });
    } finally { mounted.dispose(); }
  });

  it("distinguishes a day change during stale template work (GH #671)", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    setGraphMeta({ root: "/private/graph", default_journal_template: "Private template" } as GraphMeta);
    vi.mocked(ensureJournalTemplateForDay).mockImplementationOnce(async () => {
      vi.setSystemTime(Date.now() + 86_400_000);
      return "stale";
    });
    const diagnostic = vi.spyOn(backend(), "diagnosticFrontendEvent").mockResolvedValue();
    const mounted = mount(() => <PageView />);
    try {
      await vi.waitFor(() => expect(mounted.root.textContent).toContain("Journal feed read failed"));
      expect(diagnostic).toHaveBeenCalledWith("journal_feed", { journalFeedReason: "template-day-changed" });
    } finally { mounted.dispose(); }
  });

  it("a failed session read leaves the initial journal feed available (GH #671)", async () => {
    vi.spyOn(backend(), "loadSession").mockRejectedValue(new Error("no app-data dir"));
    vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({
      pages: [], next_before_day: null, done: true, as_of_day: localDayKey(),
    });
    const mounted = mount(() => <PageView />);
    try {
      await restoreSession();
      await vi.waitFor(() => expect(mounted.root.querySelector(".journal-title")).not.toBeNull());
      expect(mounted.root.textContent).not.toContain("Couldn't open this page");
    } finally { mounted.dispose(); }
  });

  it("surfaces an initial feed read failure instead of claiming the graph has no journals", async () => {
    const diagnostic = vi.spyOn(backend(), "diagnosticFrontendEvent").mockResolvedValue();
    vi.spyOn(backend(), "journalFeedPage").mockRejectedValue(new Error("iCloud journal read failed"));
    const mounted = mount(() => <PageView />);
    try {
      await vi.waitFor(() => expect(mounted.root.textContent).toContain("iCloud journal read failed"));
      expect(mounted.root.textContent).toContain("Couldn't open this page");
      expect(mounted.root.textContent).not.toContain("No journal entries found");
      expect(diagnostic).toHaveBeenCalledWith("journal_feed", { journalFeedReason: "feed-error", journalFeedCause: "other-rejection" });
      expect(JSON.stringify(diagnostic.mock.calls)).not.toContain("iCloud journal read failed");
    } finally {
      mounted.dispose();
    }
  });

  // og C5 P2 (I-22): a journal the backend skipped is named, never silently absent.
  it("reports journals the backend skipped as unreadable", async () => {
    vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({
      pages: [], next_before_day: null, done: true, as_of_day: localDayKey(),
      unreadable: ["journals/2026_01_02.md: stream did not contain valid UTF-8"],
    });
    const mounted = mount(() => <PageView />);
    try {
      await vi.waitFor(() => expect(toasts()).toEqual([
        expect.objectContaining({ kind: "error", message: expect.stringContaining("journals/2026_01_02.md") }),
      ]));
    } finally {
      mounted.dispose();
    }
  });
});
