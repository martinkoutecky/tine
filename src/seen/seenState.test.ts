import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../backend";
import type { SeenPageChanges } from "../backendTypes";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { setToasts, toasts } from "../toasts";
import { dropDisplayedFor } from "./displayed";
import { ensureSeenAvailability, forgetSeenStateNow, leaveAllSeenPages, leaveSeenPage, markGraphSeenNow, markPageSeenNow, noteSeenAnswer, registerDisplayed, resetSeenForTests, seenAvailable } from "./seenState";
import { graphOwner } from "../owned";

beforeAll(() => initParser());
const answer = (available: boolean): SeenPageChanges => ({
  available, reason: available ? null : "not a repository", head_commit: null, baseline_commit: null, current_text_sha256: "",
  identity_known: true, unattributed: false, blocks: [], removed: [],
});
const shown = (name: string, path = `pages/${name}.md`) => registerDisplayed(name, { key: name.toLowerCase(), path, owner: graphOwner() });

beforeEach(() => resetSeenForTests());
afterEach(() => { vi.restoreAllMocks(); resetStore(); setToasts([]); });

describe("availability is learned once, from getPageChanges (F5)", () => {
  it("probes once per binding and stays true once any read answered", async () => {
    const read = vi.spyOn(backend(), "getPageChanges").mockResolvedValue(answer(true));
    ensureSeenAvailability();
    ensureSeenAvailability();
    await vi.waitFor(() => expect(seenAvailable()).toBe(true));
    expect(read).toHaveBeenCalledTimes(1);
    noteSeenAnswer(false); // a page whose baseline left the branch is not a graph without git
    expect(seenAvailable()).toBe(true);
  });

  it("mark, mark graph and forget are silent no-ops while unavailable: no call, no toast", async () => {
    const mark = vi.spyOn(backend(), "markPageSeen").mockResolvedValue(undefined);
    const markGraph = vi.spyOn(backend(), "markGraphSeen").mockResolvedValue(undefined);
    const forget = vi.spyOn(backend(), "forgetSeenState").mockResolvedValue(undefined);
    noteSeenAnswer(false);
    shown("P");
    expect(await markGraphSeenNow()).toBe(false);
    expect(await forgetSeenStateNow()).toBe(false);
    await leaveSeenPage("P");
    await leaveAllSeenPages();
    expect(mark).not.toHaveBeenCalled();
    expect(markGraph).not.toHaveBeenCalled();
    expect(forget).not.toHaveBeenCalled();
    expect(toasts()).toEqual([]);
  });
});

describe("explicit actions", () => {
  beforeEach(() => noteSeenAnswer(true));

  it("Mark page seen marks the loaded page under its identity key and path, then announces it", async () => {
    loadSingle({ name: "My Page", kind: "page", title: "My Page", pre_block: null, blocks: [], format: "md", id: "pages/My Page.md" } as never);
    const mark = vi.spyOn(backend(), "markPageSeen").mockResolvedValue(undefined);
    expect(await markPageSeenNow("My Page")).toBe(true);
    expect(mark).toHaveBeenCalledWith("my page", "pages/My Page.md");
    expect(toasts()).toHaveLength(1);
  });

  it("Mark whole graph seen and Forget seen state call their commands once", async () => {
    const markGraph = vi.spyOn(backend(), "markGraphSeen").mockResolvedValue(undefined);
    const forget = vi.spyOn(backend(), "forgetSeenState").mockResolvedValue(undefined);
    expect(await markGraphSeenNow()).toBe(true);
    expect(await forgetSeenStateNow()).toBe(true);
    expect(markGraph).toHaveBeenCalledTimes(1);
    expect(forget).toHaveBeenCalledTimes(1);
  });

  it("a failed mark is one error toast", async () => {
    vi.spyOn(backend(), "markGraphSeen").mockRejectedValue(new Error("disk full"));
    expect(await markGraphSeenNow()).toBe(false);
    expect(toasts()).toHaveLength(1);
  });
});

describe("leaving a page", () => {
  beforeEach(() => noteSeenAnswer(true));

  it("marks it once, under the key and path it was read with", async () => {
    const mark = vi.spyOn(backend(), "markPageSeen").mockResolvedValue(undefined);
    shown("Alpha");
    await leaveSeenPage("alpha"); // the route may spell the name in another case
    await leaveSeenPage("Alpha");
    expect(mark).toHaveBeenCalledTimes(1);
    expect(mark).toHaveBeenCalledWith("alpha", "pages/Alpha.md");
  });

  it("F6: a page Tine renamed, merged or deleted is not marked on leave", async () => {
    const mark = vi.spyOn(backend(), "markPageSeen").mockResolvedValue(undefined);
    shown("Renamed");
    shown("Merged");
    shown("ByPath", "pages/by-path.md");
    dropDisplayedFor({ name: "RENAMED" });
    dropDisplayedFor({ name: "Merged" });
    dropDisplayedFor({ path: "pages/by-path.md" });
    await leaveSeenPage("Renamed");
    await leaveSeenPage("Merged");
    await leaveSeenPage("ByPath");
    await leaveAllSeenPages();
    expect(mark).not.toHaveBeenCalled();
  });

  it("quitting marks every page still shown, and a failure never throws", async () => {
    const mark = vi.spyOn(backend(), "markPageSeen").mockRejectedValueOnce(new Error("boom")).mockResolvedValue(undefined);
    shown("One");
    shown("Two");
    await expect(leaveAllSeenPages()).resolves.toBeUndefined();
    expect(mark).toHaveBeenCalledTimes(2);
    expect(toasts()).toEqual([]);
  });
});
