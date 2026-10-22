// UI-GH746-REFERENCE-REFRESH-COST (GX5-1): a save-driven dataRev bump names
// the pages it published, so a page's own linked/unlinked references (which
// the host computes without that page's blocks) can skip a save of only that
// page. Every other bump, and a trailing bump covering several publications,
// still counts as a change beyond the page.

import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { createEffect, createRoot, on } from "solid-js";
import { dataRev, dataRevBeyond } from "../../graphSession";
import { trackRefreshRead } from "../../refreshReads";
import type { PageDto } from "../../types";
import { resetStore, setRaw } from "../index";
import { loadRoutedPage } from "../workingSet";
import { isDirty, isSaving, pendingDataRevision } from "./wiring";
import { bindTestHost } from "./wiring.test.support";

function dto(name: string): PageDto & { id: string } {
  return { name, kind: "page", title: name, id: `pages/${name}.md`, rev: "r1", pre_block: null,
    blocks: [{ id: `${name}-block`, raw: name, collapsed: false, children: [] }] };
}

interface Bump { rev: number; beyondP: number; beyondQ: number; settledP: boolean }

let dispose: (() => void) | null = null;
beforeEach(() => { resetStore(); });
afterEach(() => { dispose?.(); dispose = null; vi.useRealTimers(); resetStore(); vi.restoreAllMocks(); });

async function start(slow = false) {
  loadRoutedPage(dto("P"));
  loadRoutedPage(dto("Q"));
  await bindTestHost();
  const bumps: Bump[] = [];
  const settles: (() => void)[] = [];
  createRoot((d) => {
    dispose = d;
    createEffect(on(dataRev, (rev) => {
      bumps.push({ rev, beyondP: dataRevBeyond("P"), beyondQ: dataRevBeyond("Q"),
        settledP: !isDirty("P") && !isSaving("P") && !pendingDataRevision() });
      if (slow) void trackRefreshRead(new Promise<void>((resolve) => { settles.push(resolve); }));
    }, { defer: true }));
  });
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  return { bumps, settles };
}

async function type(id: string, text: string) {
  setRaw(id, text, { timetracking: false });
  await vi.advanceTimersByTimeAsync(1200);
}

it("a save of one page is a change beyond every other page, not beyond itself", async () => {
  const { bumps } = await start();
  await type("P-block", "a");
  await type("P-block", "ab");
  expect(bumps).toHaveLength(2);
  for (const bump of bumps) {
    expect(bump.beyondQ).toBe(bump.rev);
    expect(bump.beyondP).toBeLessThan(bumps[0].rev);
    // The page is settled at its bump, so a pending header change clears there.
    expect(bump.settledP).toBe(true);
  }
  await type("Q-block", "q");
  expect(bumps[2].beyondP).toBe(bumps[2].rev);
});

it("the trailing bump after a slow round names every page published meanwhile", async () => {
  const { bumps, settles } = await start(true);
  await type("P-block", "a");
  await type("P-block", "ab");
  await type("Q-block", "q");
  expect(bumps).toHaveLength(1);
  settles[0]();
  await vi.advanceTimersByTimeAsync(0);
  expect(bumps).toHaveLength(2);
  expect(bumps[1].beyondP, "Q was published during the round").toBe(bumps[1].rev);
  expect(bumps[1].beyondQ).toBe(bumps[1].rev);
});
