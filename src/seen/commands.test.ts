import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { paletteCommands } from "../keybindings";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { setGraphMeta } from "../graphSession";
import * as router from "../router";
import { setToasts } from "../toasts";
import { noteSeenAnswer, resetSeenForTests } from "./seenState";

beforeAll(() => initParser());
beforeEach(() => resetSeenForTests());
afterEach(() => { vi.restoreAllMocks(); resetStore(); resetSeenForTests(); setGraphMeta(null); setToasts([]); router.resetTabsToJournals(); });

const ids = () => paletteCommands().map((c) => c.id).filter((id) => id.startsWith("seen/"));

it("the palette offers the seen commands only once a git read answered (F5)", () => {
  vi.spyOn(backend(), "getPageChanges").mockResolvedValue(undefined as never);
  expect(ids()).toEqual([]);
  noteSeenAnswer(false);
  expect(ids()).toEqual([]);
});

it("Mark whole graph seen needs no page; Mark page seen needs a routed page with a file", async () => {
  noteSeenAnswer(true);
  router.resetTabsToJournals();
  expect(ids()).toEqual(["seen/mark-graph"]);
  loadSingle({ name: "Notes", kind: "page", title: "Notes", pre_block: null, blocks: [], format: "md", id: "pages/Notes.md" } as never);
  router.openPage("Notes", "page");
  expect(ids()).toEqual(["seen/mark-page", "seen/mark-graph"]);
  const mark = vi.spyOn(backend(), "markPageSeen").mockResolvedValue(undefined);
  const markGraph = vi.spyOn(backend(), "markGraphSeen").mockResolvedValue(undefined);
  const command = (id: string) => paletteCommands().find((c) => c.id === id)!;
  expect(command("seen/mark-page").label).toBe("Mark page seen");
  expect(command("seen/mark-graph").label).toBe("Mark whole graph seen");
  command("seen/mark-page").run();
  command("seen/mark-graph").run();
  await vi.waitFor(() => expect(mark).toHaveBeenCalledWith("notes", "pages/Notes.md"));
  await vi.waitFor(() => expect(markGraph).toHaveBeenCalledTimes(1));
});
