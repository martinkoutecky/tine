// UI-QUERY-STUCK-LOADING: query results stayed at "Loading query results…" after
// the user edited a block they match and left the editor. Leaving the editor
// closes the page on the host; while that close is in flight the page is busy,
// so the results gate (`resultsSettled`) is closed. A query run that landed in
// that window was not adopted, and because the gate's closing never reached
// reactive readers, its reopening re-ran nothing: the run that would replace
// the dropped answer never started. Every transition of the gate must reach a
// reactive reader (the own-work gate's contract: hostRevision moves whenever a
// page's busy state changes).

import { afterEach, expect, it, vi } from "vitest";
import { createComputed, createMemo, createRoot, getOwner } from "solid-js";
import { backend } from "../../backend";
import { endEdit, startEditing } from "../../editorController";
import { resultsSettled } from "../../queryResultsEpoch";
import type { PageDto } from "../../types";
import { resetStore, setRaw } from "../index";
import { loadRoutedPage } from "../workingSet";
import { bindTestHost } from "./wiring.test.support";
import { pageFrozen } from "./wiring";
import { HostClient } from "./client";

afterEach(() => { endEdit("blur"); resetStore(); vi.restoreAllMocks(); });

function dto(name: string): PageDto & { id: string } {
  return { name, kind: "page", title: name, id: `pages/${name}.md`, rev: "r1", pre_block: null,
    blocks: [{ id: `${name}-block`, raw: name, collapsed: false, children: [] }] };
}

it("a page becoming busy does not notify unchanged input-freeze readers", async () => {
  resetStore();
  loadRoutedPage(dto("P"));
  await bindTestHost();
  let reads = 0;
  const dispose = createRoot(dispose => {
    createComputed(() => { expect(pageFrozen("P")).toBe(false); reads++; });
    return dispose;
  });
  try {
    expect(reads).toBe(1);
    setRaw("P-block", "Typed text", { timetracking: false });
    expect(reads).toBe(1);
  } finally { dispose(); }
});

it("input checks see the current host freeze flag before another status notification", async () => {
  resetStore();
  loadRoutedPage(dto("P"));
  await bindTestHost();
  const frozen = vi.spyOn(HostClient.prototype, "isFrozen").mockReturnValue(false);
  expect(pageFrozen("P")).toBe(false);
  frozen.mockReturnValue(true);
  expect(pageFrozen("P")).toBe(true);
  frozen.mockReturnValue(false);
  expect(pageFrozen("P")).toBe(false);
});

it("input-freeze publishers do not retain the view owner that first queried them", async () => {
  resetStore();
  await bindTestHost();
  const detached: boolean[] = [];
  vi.spyOn(HostClient.prototype, "isFrozen").mockImplementation(() => {
    const owner = getOwner();
    if (owner && "fn" in owner) detached.push(owner.owner?.owner === null);
    return false;
  });
  createRoot(dispose => {
    loadRoutedPage(dto("P"));
    expect(pageFrozen("P")).toBe(false);
    dispose();
  });
  expect(detached.length).toBeGreaterThan(0);
  expect(detached.every(Boolean)).toBe(true);
});

it("a host close in flight after the editor leaves closes the results gate for reactive readers, and reopens it", async () => {
  resetStore();
  loadRoutedPage(dto("P"));
  await bindTestHost();
  const open = vi.spyOn(backend(), "pageOpen");
  let admitClose: (() => void) | null = null;
  const realClose = backend().pageClose.bind(backend());
  const close = vi.spyOn(backend(), "pageClose").mockImplementation((...args) => new Promise((resolve) => {
    admitClose = () => { void realClose(...args).then(resolve); };
  }));
  await createRoot(async (dispose) => {
    // What the query blocks' request memo reads (resultMembership.ts `settled`).
    const observed = createMemo(() => resultsSettled());
    try {
      startEditing("P-block");
      await vi.waitFor(() => expect(open).toHaveBeenCalledOnce(), { timeout: 2000 });
      await vi.waitFor(() => expect(observed()).toBe(true), { timeout: 2000 });
      endEdit("blur");
      await vi.waitFor(() => expect(close).toHaveBeenCalledOnce(), { timeout: 2000 });
      expect(resultsSettled()).toBe(false);
      expect(observed()).toBe(false);
      admitClose!();
      await vi.waitFor(() => expect(resultsSettled()).toBe(true), { timeout: 2000 });
      expect(observed()).toBe(true);
    } finally {
      dispose();
    }
  });
});
