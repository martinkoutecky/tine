import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { backend } from "../../backend";
import { startEditing } from "../../editorController";
import { flushPage, liveConflictDraft, loadFeed, resetStore, setRaw } from "../index";
import { forgetPage } from "../workingSet";
import { baseRevFor } from "../host/wiring";
import { answerOpensFromDocument } from "../host/documentHost.test.support";
import { bindTestHost } from "../host/wiring.test.support";

const KEY = "pages/Note.md";

beforeEach(() => resetStore());
afterEach(() => { resetStore(); vi.restoreAllMocks(); });

async function submitted() {
  loadFeed([{ id: KEY, name: "Note", title: "Note", kind: "page", pre_block: null, rev: "disk-0",
    blocks: [{ id: "body", raw: "original", collapsed: false, children: [] }] }]);
  const host = await bindTestHost();
  answerOpensFromDocument(host);
  const submit = vi.spyOn(backend(), "pageSubmit").mockResolvedValue(null);
  startEditing("body");
  setRaw("body", "typed text");
  const flushed = flushPage("Note");
  await vi.waitFor(() => expect(submit).toHaveBeenCalledTimes(1));
  return { host, flushed, id: submit.mock.calls[0][1] };
}

it.each([false, true])("review keeps the disk ancestor when a submit is taken with conflict=%s", async (conflict) => {
  vi.spyOn(backend(), "pageWait").mockResolvedValue(false);
  const { host, flushed, id } = await submitted();
  host.deliver({ key: KEY, answer: { id, version: 4, took: true, outcome: { kind: "applied" } },
    page: { version: 4, conflict, risk: conflict, disk: { kind: "file", rev: "cli-disk" },
      text: { kind: "unchanged", rev: "draft-never-published" } } });
  // Also cover an ordinary acknowledgment followed by an outside write before
  // publication: custody of input is not proof those bytes ever reached disk.
  if (!conflict) host.notice(KEY, { conflictReported: true }, { version: 4, conflict: true, risk: true,
    disk: { kind: "file", rev: "cli-disk" } });
  expect(await flushed).toBe(false);
  const draft = liveConflictDraft("Note");
  expect(draft?.page.blocks[0].raw).toBe("typed text");
  expect(draft?.baseRev).toBe("disk-0");
});

it("a proved publication advances the ancestor for the next conflict", async () => {
  const { host, flushed, id } = await submitted();
  host.deliver({ key: KEY, answer: { id, version: 4, took: true, outcome: { kind: "applied" } },
    page: { version: 4, conflict: false, risk: false, disk: { kind: "file", rev: "disk-0" },
      text: { kind: "unchanged", rev: "disk-1" } } });
  expect(await flushed).toBe(true);
  expect(baseRevFor("Note")).toBe("disk-1");
  host.notice(KEY, { conflictReported: true }, { version: 4, conflict: true, risk: true,
    disk: { kind: "file", rev: "cli-disk" } });
  expect(liveConflictDraft("Note")?.baseRev).toBe("disk-1");
});

it("a late publication cannot change the ancestor of a replacement page instance", async () => {
  let finish!: (published: boolean) => void;
  const wait = vi.spyOn(backend(), "pageWait").mockReturnValue(new Promise(resolve => { finish = resolve; }));
  const { host, flushed, id } = await submitted();
  host.deliver({ key: KEY, answer: { id, version: 4, took: true, outcome: { kind: "applied" } },
    page: { version: 4, conflict: false, risk: false, disk: { kind: "file", rev: "disk-0" },
      text: { kind: "unchanged", rev: "disk-1" } } });
  await vi.waitFor(() => expect(wait).toHaveBeenCalled());
  forgetPage("Note");
  loadFeed([{ id: KEY, name: "Note", title: "Note", kind: "page", pre_block: null, rev: "replacement",
    blocks: [{ id: "other", raw: "reloaded text", collapsed: false, children: [] }] }]);
  finish(true);
  await flushed;
  expect(baseRevFor("Note")).toBe("replacement");
});
