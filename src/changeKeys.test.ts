import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { applyChangeSummary } from "./changeSummary";
import { changed, changedAny, flushChangeKeys, installChangeKeyPublicationHold, setRefreshEverything } from "./changeKeys";
import { bumpDataRev, bumpGraphEpoch } from "./graphSession";
import { invalidateBinding } from "./binding";
import { resetRefreshReads, trackRefreshRead } from "./refreshReads";
import type { ChangeSummary } from "./backendTypes";

let rev = 0;
function stage(fields: Partial<ChangeSummary> = {}) {
  applyChangeSummary({ rev: String(++rev), full: false, pages: [], blocks: [], linksTo: [], textTo: [], tagsTo: [], tagTableTo: [], refsTo: [], props: {}, inventoryChanged: false, ...fields });
}
beforeEach(() => {
  bumpGraphEpoch(); rev = 0; stage(); bumpDataRev(true);
});
afterEach(() => {
  setRefreshEverything(false); installChangeKeyPublicationHold(() => false);
  resetRefreshReads(); vi.useRealTimers();
});

it("stages and unions exact keys until the publication gate drains them", () => {
  const before = [changed("block:a"), changed("block:b"), changed("page:p"), changed("icon:p"), changed("other:x")];
  stage({ blocks: ["a"], pages: ["p"] });
  stage({ blocks: ["b"], props: { p: ["icon"] } });
  expect([changed("block:a"), changed("page:p"), changed("icon:p")]).toEqual([before[0], before[2], before[3]]);
  bumpDataRev(true);
  for (const [i, key] of ["block:a", "block:b", "page:p", "icon:p"].entries()) expect(changed(key)).not.toBe(before[i]);
  expect(changed("other:x")).toBe(before[4]);
});

it.each(["full", "gap", "epoch", "binding", "no summary", "missing expected summary", "debug"])("%s fires every key, including unknown kinds", reason => {
  const keys = ["block:other", "page:other", "icon:other", "text:other", "tags:other", "tagTable:other", "refsTo:other", "future:other"];
  const before = keys.map(changed);
  if (reason === "epoch") bumpGraphEpoch();
  else if (reason === "binding") invalidateBinding();
  else if (reason === "no summary") bumpDataRev();
  else if (reason === "missing expected summary") bumpDataRev(true);
  else {
    if (reason === "gap") rev += 2;
    if (reason === "debug") setRefreshEverything(true);
    stage({ full: reason === "full" }); bumpDataRev(true);
  }
  keys.forEach((key, i) => expect(changed(key)).not.toBe(before[i]));
});

it("S2 target keys stage precisely and legacy summaries refresh just the missing families", () => {
  const keys = ["text:p", "tags:p", "refsTo:id", "links:p", "text:other", "block:other"];
  const before = keys.map(changed);
  stage({ textTo: ["p"], tagsTo: ["p"], refsTo: ["id"], linksTo: ["p"] });
  expect(keys.map(changed)).toEqual(before);
  bumpDataRev(true);
  keys.forEach((key, i) => i < 4 ? expect(changed(key)).not.toBe(before[i]) : expect(changed(key)).toBe(before[i]));
  const precise = keys.map(changed);
  stage({ textTo: undefined, tagsTo: undefined, refsTo: undefined }); bumpDataRev(true);
  keys.forEach((key, i) => [0, 1, 2, 4].includes(i) ? expect(changed(key)).not.toBe(precise[i]) : expect(changed(key)).toBe(precise[i]));
});

it("shown members join without another initial read and update the combined token", () => {
  const initial = changedAny(["links:p"]);
  expect(changedAny(["links:p", "block:shown"])).toBe(initial);
  stage({ blocks: ["unrelated"] }); bumpDataRev(true);
  expect(changedAny(["links:p", "block:shown"])).toBe(initial);
  stage({ blocks: ["shown"] }); bumpDataRev(true);
  expect(changedAny(["links:p", "block:shown"])).not.toBe(initial);
});

it("registered query keys fire only for affected subscriptions", () => {
  const before = [changed("query:narrow"), changed("query:links"), changed("query:unrelated")];
  stage({ queryTo: ["narrow", "links"] } as Partial<ChangeSummary>);
  bumpDataRev(true);
  expect(changed("query:narrow")).not.toBe(before[0]);
  expect(changed("query:links")).not.toBe(before[1]);
  expect(changed("query:unrelated")).toBe(before[2]);
});

it("a summary-only delivery waits for the trailing delay and the previous refresh reads", async () => {
  vi.useFakeTimers();
  const before = changed("block:a");
  let finish!: () => void;
  trackRefreshRead(new Promise<void>(resolve => { finish = resolve; }));
  stage({ blocks: ["a"] });
  await vi.advanceTimersByTimeAsync(700);
  expect(changed("block:a")).toBe(before);
  finish(); await vi.advanceTimersByTimeAsync(0);
  expect(changed("block:a")).not.toBe(before);
});

it("summary-only refreshes have a bounded hold and are retired with their graph", async () => {
  vi.useFakeTimers();
  trackRefreshRead(new Promise<void>(() => {}));
  const before = changed("block:a");
  stage({ blocks: ["a"] });
  await vi.advanceTimersByTimeAsync(5699);
  expect(changed("block:a")).toBe(before);
  await vi.advanceTimersByTimeAsync(1);
  expect(changed("block:a")).not.toBe(before);
  stage({ blocks: ["a"] }); bumpGraphEpoch();
  const replacement = changed("block:a");
  await vi.advanceTimersByTimeAsync(6000);
  expect(changed("block:a")).toBe(replacement);
});

it("a local host publication retains its existing wait/hold ownership", async () => {
  vi.useFakeTimers();
  installChangeKeyPublicationHold(() => true);
  const before = changed("block:a");
  stage({ blocks: ["a"] });
  await vi.advanceTimersByTimeAsync(6000);
  expect(changed("block:a")).toBe(before);
  flushChangeKeys(true);
  expect(changed("block:a")).not.toBe(before);
});
