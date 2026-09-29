// GH #254 family (master 7bd793bd0, og J1): when another file holding today's
// name has uncommitted input, the journals feed used to publish that file as
// today, so what was typed there saved to the wrong file. The feed must stay
// unpublished, say why, and load the requested day once the holder is free.
import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { ensurePageLoaded, pageByName, pinPageWhileDrafting, resetStore } from "../document";
import { doc } from "../document/model";
import { appNow, journalTitle, localDayKey } from "../journal";
import { resetTabsToJournals } from "../router";
import { setGraphMeta } from "../graphSession";
import { setToasts, toasts } from "../toasts";
import type { JournalFeedPage, PageDto, PageRead } from "../types";
import { PageView } from "./Page";

beforeAll(async () => { await initParser(); });
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  resetStore();
  setGraphMeta(null);
  setToasts([]);
  document.body.innerHTML = "";
  resetTabsToJournals();
});

const file = (name: string, id: string, raw: string): PageDto & { id: string; rev: string } => ({
  id, name, title: name, kind: "journal", pre_block: null, rev: `rev-${id}`,
  blocks: [{ id: `${id}-b`, raw, collapsed: false, children: [] }],
});

it("keeps a second file holding today's name out of the feed, reports it, and loads today in place once it is free", async () => {
  vi.stubGlobal("IntersectionObserver", class { observe() {} unobserve() {} disconnect() {} });
  const today = journalTitle(appNow());
  ensurePageLoaded(file(today, "pages/stray.md", "stray text"));
  expect(pageByName(today)!.id).toBe("pages/stray.md");
  const unpin = pinPageWhileDrafting(() => today);
  const response: JournalFeedPage = { pages: [file(today, "journals/today.md", "canonical text") as PageRead], next_before_day: null, done: true, as_of_day: localDayKey() };
  vi.spyOn(backend(), "journalFeedPage").mockResolvedValue(response);
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => <PageView />, root);
  try {
    await vi.waitFor(() => expect(root.textContent).toContain("pages/stray.md"));
    expect(doc.feed).not.toContain(today);
    expect(root.textContent).not.toContain("stray text");
    expect(toasts().some((toast) => toast.kind === "error" && toast.message.includes("journals/today.md"))).toBe(true);
    expect(root.textContent).not.toContain("Couldn't open this page");
    unpin();
    // In place, as master: the open Journals view fills once the name is free.
    await vi.waitFor(() => expect(root.textContent).toContain("canonical text"));
    expect(doc.feed).toContain(today);
    expect(pageByName(today)!.id).toBe("journals/today.md");
    expect(root.textContent).not.toContain("pages/stray.md");
  } finally { unpin(); dispose(); }
});
