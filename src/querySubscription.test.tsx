import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { createEffect, createSignal } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "./backend";
import { createQuerySubscription, noteQueryInput, QUERY_QUIET_MS, type QueryRegistration, type QuerySubscription } from "./querySubscription";
import { bumpGraphEpoch, bumpDataRev } from "./graphSession";
import { applyChangeSummary } from "./changeSummary";
import { installResultsPendingGate, resultsSettled } from "./queryResultsEpoch";
import { createMembershipResource } from "./resultMembership";
import { setRefreshEverything } from "./changeKeys";
import type { ChangeSummary } from "./backendTypes";

let rev = 0;
let dispose: (() => void) | undefined;
const [pending, setPending] = createSignal(false);
installResultsPendingGate(pending, "own-work");
const host = backend();
const original = host.registerQuerySubscription;
const register = async (_id: string, request: QuerySubscription | null): Promise<QueryRegistration> => ({
  mode: request?.kind === "registry" ? "quiet" as const : "narrow" as const, rev: String(rev),
});
const registration = vi.fn(register);
function publish(fields: Partial<ChangeSummary> = {}) {
  applyChangeSummary({ rev: String(++rev), pages: [], blocks: [], linksTo: [], props: {}, inventoryChanged: false,
    full: false, queryTo: [], ...fields });
  bumpDataRev(true);
}
beforeEach(() => {
  vi.useFakeTimers(); noteQueryInput(); vi.setSystemTime(Date.now() + QUERY_QUIET_MS);
  setPending(false); bumpGraphEpoch(); rev = 0; publish();
  host.registerQuerySubscription = registration; registration.mockReset().mockImplementation(register);
});
afterEach(() => { dispose?.(); dispose = undefined; host.registerQuerySubscription = original;
  setPending(false); setRefreshEverything(false); vi.useRealTimers(); document.body.innerHTML = ""; });

function mount(request: QuerySubscription = { kind: "full" }) {
  const read = vi.fn();
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => {
    const subscription = createQuerySubscription(() => request);
    createEffect(() => { const revision = subscription.revision(); void subscription.ready().then(current => { if (current) read(revision); }); });
    return <div />;
  }, root);
  return read;
}

it("a registered reader uses only its query key, with full and gap fallback", async () => {
  const read = mount(); await vi.advanceTimersByTimeAsync(0); read.mockClear();
  const id = registration.mock.calls[0][0];
  publish({ queryTo: ["some-other-query"] }); await vi.advanceTimersByTimeAsync(0);
  expect(read).not.toHaveBeenCalled();
  publish({ queryTo: [id] }); await vi.advanceTimersByTimeAsync(0); expect(read).toHaveBeenCalledTimes(1);
  publish({ full: true }); await vi.advanceTimersByTimeAsync(0); expect(read).toHaveBeenCalledTimes(2);
  rev += 2; publish(); await vi.advanceTimersByTimeAsync(0); expect(read).toHaveBeenCalledTimes(3);
});

it("coalesces a typing burst, waits past the quiet boundary for the latest publication, and runs once", async () => {
  const read = mount({ kind: "registry" }); await vi.advanceTimersByTimeAsync(0); read.mockClear();
  const id = registration.mock.calls[0][0];
  for (let i = 0; i < 5; i++) {
    noteQueryInput(); publish({ queryTo: [id] });
    await vi.advanceTimersByTimeAsync(1000);
    expect(read).not.toHaveBeenCalled();
  }
  setPending(true);
  await vi.advanceTimersByTimeAsync(QUERY_QUIET_MS);
  expect(read).not.toHaveBeenCalled();
  // The final publication arrives late: no query against its predecessor.
  publish({ queryTo: [id] }); setPending(false); await vi.advanceTimersByTimeAsync(0);
  expect(read).toHaveBeenCalledTimes(1);
  await vi.advanceTimersByTimeAsync(5000); expect(read).toHaveBeenCalledTimes(1);
});

it("a keystroke after a relevant publication resets the pause even without another save yet", async () => {
  const read = mount({ kind: "registry" }); await vi.advanceTimersByTimeAsync(0); read.mockClear();
  const id = registration.mock.calls[0][0];
  noteQueryInput(); publish({ queryTo: [id] }); await vi.advanceTimersByTimeAsync(1200);
  noteQueryInput(); await vi.advanceTimersByTimeAsync(1000); expect(read).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(500); expect(read).toHaveBeenCalledTimes(1);
});

it("re-arms when a quiet timer wakes just before the wall-clock boundary", async () => {
  const read = mount({ kind: "registry" }); await vi.advanceTimersByTimeAsync(0); read.mockClear();
  const id = registration.mock.calls[0][0];
  noteQueryInput(); const inputAt = Date.now(); publish({ queryTo: [id] });
  const clock = vi.spyOn(Date, "now").mockReturnValue(inputAt + QUERY_QUIET_MS - 1);
  try {
    await vi.advanceTimersByTimeAsync(QUERY_QUIET_MS);
    expect(read).not.toHaveBeenCalled();
  } finally { clock.mockRestore(); }
  await vi.advanceTimersByTimeAsync(1);
  expect(read).toHaveBeenCalledTimes(1);
});

it("DOM composition input resets the typing clock before any draft is committed", async () => {
  const read = mount({ kind: "registry" }); await vi.advanceTimersByTimeAsync(0); read.mockClear();
  const id = registration.mock.calls[0][0];
  noteQueryInput(); publish({ queryTo: [id] }); await vi.advanceTimersByTimeAsync(1200);
  document.dispatchEvent(new CompositionEvent("compositionupdate", { data: "한", bubbles: true }));
  await vi.advanceTimersByTimeAsync(1000); expect(read).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(500); expect(read).toHaveBeenCalledTimes(1);
});

it("a disposed reader never starts its initial read when registration finishes late", async () => {
  let finish!: (value: { mode: "narrow"; rev: string }) => void;
  registration.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  const read = mount(); await vi.advanceTimersByTimeAsync(0);
  const id = registration.mock.calls[0][0];
  dispose!(); dispose = undefined; finish({ mode: "narrow", rev: "1" });
  await vi.advanceTimersByTimeAsync(0);
  expect(read).not.toHaveBeenCalled();
  expect(registration).toHaveBeenCalledWith(id, null, expect.any(Number));
});

it("registration failures reach the reader and a retry renews the subscription", async () => {
  registration.mockRejectedValueOnce(new Error("registration unavailable"));
  let subscription!: ReturnType<typeof createQuerySubscription>;
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => {
    subscription = createQuerySubscription(() => ({ kind: "full" }));
    return <div />;
  }, root);
  await expect(subscription.ready()).rejects.toThrow("registration unavailable");
  await expect(subscription.ready()).resolves.toBe(true);
  expect(registration).toHaveBeenCalledTimes(2);
});

it("the S3 debug toggle bypasses query quiet scheduling", async () => {
  const read = mount({ kind: "registry" }); await vi.advanceTimersByTimeAsync(0); read.mockClear();
  noteQueryInput(); setRefreshEverything(true); publish(); await vi.advanceTimersByTimeAsync(0);
  expect(read).toHaveBeenCalledTimes(1);
});

it("disposal releases the registration and cancels a pending quiet rerun", async () => {
  const read = mount({ kind: "registry" }); await vi.advanceTimersByTimeAsync(0); read.mockClear();
  const id = registration.mock.calls[0][0];
  noteQueryInput(); publish({ queryTo: [id] }); dispose!(); dispose = undefined;
  await vi.advanceTimersByTimeAsync(5000);
  expect(read).not.toHaveBeenCalled();
  expect(registration).toHaveBeenCalledWith(id, null, expect.any(Number));
});

it("full registrations coalesce graph-wide fallback reads throughout continuous typing", async () => {
  registration.mockImplementation(async () => ({ mode: "full", rev: String(rev) }));
  const read = mount(); await vi.advanceTimersByTimeAsync(QUERY_QUIET_MS); read.mockClear();
  const id = registration.mock.calls[0][0];
  for (let i = 0; i < 3; i++) {
    noteQueryInput(); publish({ queryTo: [id] });
    await vi.advanceTimersByTimeAsync(1000);
    expect(read).not.toHaveBeenCalled();
  }
  await vi.advanceTimersByTimeAsync(500);
  expect(read).toHaveBeenCalledTimes(1);
});

it.each(["input", "publication"])("renewal re-arms after a later %s instead of starting a read", async cause => {
  const read = vi.fn(() => expect(resultsSettled()).toBe(true));
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => {
    const subscription = createQuerySubscription(() => ({ kind: "registry" }));
    createMembershipResource(() => "query", () => "query", async () => {
      if (await subscription.ready()) read();
      return undefined;
    }, () => false, () => true, true, subscription.revision);
    return <div />;
  }, root);
  await vi.advanceTimersByTimeAsync(QUERY_QUIET_MS); read.mockClear();
  const id = registration.mock.calls[0][0];
  let finish!: (value: { mode: "quiet"; rev: string }) => void;
  registration.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  noteQueryInput(); publish({ queryTo: [id] });
  await vi.advanceTimersByTimeAsync(QUERY_QUIET_MS);
  expect(finish).toBeTypeOf("function");
  if (cause === "input") noteQueryInput();
  else setPending(true);
  finish({ mode: "quiet", rev: String(rev) });
  await vi.advanceTimersByTimeAsync(0);
  expect(read).not.toHaveBeenCalled();
  if (cause === "input") {
    await vi.advanceTimersByTimeAsync(QUERY_QUIET_MS - 1);
    expect(read).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
  } else {
    publish({ queryTo: [id] }); setPending(false);
    await vi.advanceTimersByTimeAsync(0);
  }
  expect(read).toHaveBeenCalledTimes(1);
});
