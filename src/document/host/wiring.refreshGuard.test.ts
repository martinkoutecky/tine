// UI-GH746-REFRESH-ROUNDS-DO-NOT-PILE-UP (related to GH #746): while a key is
// held the page is published about once a second, and 700 ms after each
// publication `dataRev` moved, so every whole-graph view refetched. The host
// never cancels a superseded read, so once a refresh round took longer than the
// publication interval the rounds overlapped and piled up, and typing stalled
// more and more. Now at most one save-driven round is in flight: publications
// during it coalesce into exactly one trailing bump, after the last of them.

import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { createEffect, createRoot, on } from "solid-js";
import { backend } from "../../backend";
import { dataRev } from "../../graphSession";
import { refreshReadsInFlight, trackRefreshRead } from "../../refreshReads";
import type { PageDto } from "../../types";
import { resetStore, setRaw } from "../index";
import { loadRoutedPage } from "../workingSet";
import { pendingDataRevision, REFRESH_HOLD_MAX_MS } from "./wiring";
import { bindTestHost, submittedPages } from "./wiring.test.support";

function dto(name: string): PageDto & { id: string } {
  return { name, kind: "page", title: name, id: `pages/${name}.md`, rev: "r1", pre_block: null,
    blocks: [{ id: `${name}-block`, raw: name, collapsed: false, children: [] }] };
}

interface Round { rev: number; inFlightAtStart: number; settle: (ok: boolean) => void; lastSubmitted: string | undefined }

let dispose: (() => void) | null = null;
beforeEach(() => { resetStore(); });
afterEach(() => { dispose?.(); dispose = null; vi.useRealTimers(); resetStore(); vi.restoreAllMocks(); });

/** A `dataRev` reader whose round is two whole-graph reads that answer only
 * when the test settles them (slower than the publication interval). */
function slowReader(submit: { mock: { calls: unknown[][] } }): Round[] {
  const rounds: Round[] = [];
  createRoot((d) => {
    dispose = d;
    createEffect(on(dataRev, (rev) => {
      const inFlightAtStart = refreshReadsInFlight();
      const settles: ((ok: boolean) => void)[] = [];
      for (let i = 0; i < 2; i += 1) {
        void trackRefreshRead(new Promise<void>((resolve, reject) => {
          settles.push((ok) => ok ? resolve() : reject(new Error("read failed")));
        })).catch(() => {});
      }
      rounds.push({ rev, inFlightAtStart, settle: (ok) => settles.forEach((s) => s(ok)),
        lastSubmitted: submittedPages(submit, "P").at(-1)?.blocks[0]?.raw });
    }, { defer: true }));
  });
  return rounds;
}

async function start() {
  loadRoutedPage(dto("P"));
  await bindTestHost();
  const submit = vi.spyOn(backend(), "pageSubmit");
  const rounds = slowReader(submit);
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  return { submit, rounds };
}

/** One keystroke burst: the page is sent (400 ms debounce), published, and the
 * 700 ms coalescing timer runs out. */
async function type(text: string) {
  setRaw("P-block", text, { timetracking: false });
  await vi.advanceTimersByTimeAsync(1200);
}

it("publications during a slow refresh round coalesce into one trailing bump with the final text", async () => {
  const { rounds } = await start();
  await type("a");
  expect(rounds.map((r) => r.lastSubmitted)).toEqual(["a"]);
  // Two more publications while the first round's reads are still running.
  await type("ab");
  await type("abc");
  expect(rounds).toHaveLength(1);
  expect(refreshReadsInFlight()).toBe(2);
  expect(pendingDataRevision()).toBe(true);
  rounds[0].settle(true);
  await vi.advanceTimersByTimeAsync(0);
  // Exactly one trailing round, started with nothing in flight, after the last publication.
  expect(rounds).toHaveLength(2);
  expect(rounds[1].inFlightAtStart).toBe(0);
  expect(rounds[1].lastSubmitted).toBe("abc");
  expect(pendingDataRevision()).toBe(false);
  rounds[1].settle(true);
  await vi.advanceTimersByTimeAsync(REFRESH_HOLD_MAX_MS * 2);
  expect(rounds).toHaveLength(2);
});

it("a cheap round does not delay the next bump", async () => {
  const { rounds } = await start();
  await type("a");
  rounds[0].settle(true);
  await type("ab");
  expect(rounds.map((r) => r.lastSubmitted)).toEqual(["a", "ab"]);
});

it("a failed refresh read ends its round like an answered one", async () => {
  const { rounds } = await start();
  await type("a");
  await type("ab");
  expect(rounds).toHaveLength(1);
  rounds[0].settle(false);
  await vi.advanceTimersByTimeAsync(0);
  expect(rounds.map((r) => r.lastSubmitted)).toEqual(["a", "ab"]);
});

it("a read that never settles holds the trailing bump for at most REFRESH_HOLD_MAX_MS", async () => {
  const { rounds } = await start();
  await type("a");
  setRaw("P-block", "ab", { timetracking: false });
  await vi.advanceTimersByTimeAsync(1200);
  expect(rounds).toHaveLength(1);
  expect(pendingDataRevision()).toBe(true);
  // The trailing bump queued about 1100 ms after this edit (submit + 700 ms).
  await vi.advanceTimersByTimeAsync(REFRESH_HOLD_MAX_MS - 200);
  expect(rounds).toHaveLength(1);
  await vi.advanceTimersByTimeAsync(400);
  expect(rounds.map((r) => r.lastSubmitted)).toEqual(["a", "ab"]);
  expect(pendingDataRevision()).toBe(false);
});

it("a graph switch drops the queued bump and the old graph's reads", async () => {
  const { rounds } = await start();
  await type("a");
  await type("ab");
  expect(pendingDataRevision()).toBe(true);
  vi.useRealTimers();
  resetStore();
  expect(pendingDataRevision()).toBe(false);
  expect(refreshReadsInFlight()).toBe(0);
  const before = rounds.length;
  rounds[0].settle(true);
  await new Promise((resolve) => setTimeout(resolve, 0));
  // The old graph's round ending bumps nothing, and its late settlement does
  // not make the count negative (a new graph's reads would then be ignored).
  expect(rounds.length).toBe(before);
  expect(refreshReadsInFlight()).toBe(0);
});
