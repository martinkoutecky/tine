import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { startEditing } from "../editorController";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { bumpGraphEpoch } from "../graphSession";
import { initParser } from "../render/parse";
import { editorIn } from "../tests/editorDriver";
import { Block } from "./Block";
import type { PageEntry } from "../types";

beforeAll(() => initParser());
afterEach(() => { vi.restoreAllMocks(); resetStore(); document.body.innerHTML = ""; });
const flush = async () => { for (let i = 0; i < 40; i++) await Promise.resolve(); };
const row: PageEntry = { name: "Topic", kind: "page", path: "pages/Topic.md", date_key: null };

// A page or tag read the display epoch outran (graph open, config or title-format
// change while the read is outstanding) must not leave the open list empty
// until the next keystroke.
it.each(["[[", "#"])("%s list asks again when the epoch moves under its read", async trigger => {
  const replies: Array<(rows: PageEntry[]) => void> = [];
  const read = vi.spyOn(backend(), "quickSwitch").mockImplementation((_q, limit) => limit === 0
    ? Promise.resolve([]) : new Promise(resolve => { replies.push(resolve); }));
  const positive = () => read.mock.calls.filter(([, limit]) => limit > 0);
  loadSingle({ name: "Completion epoch", title: "Completion epoch", kind: "page", pre_block: null,
    blocks: [{ id: "completion-epoch", raw: "", collapsed: false, children: [] }] });
  startEditing("completion-epoch", 0);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <For each={pageByName("Completion epoch")?.roots ?? []}>{id => <Block id={id} />}</For>, root);
  const editor = editorIn(root);
  try {
    for (const char of `${trigger}T`) { editor.type(char); await flush(); }
    expect(positive()).toHaveLength(1);
    // The window repaints while that read is outstanding; the reply then lands
    // for an owner the epoch has retired. The list asks again and shows the
    // second answer.
    bumpGraphEpoch(); await flush();
    replies[0]([row]); await flush();
    expect(positive()).toHaveLength(2);
    replies[1]([row]); await flush();
    expect(document.querySelector(".autocomplete")?.textContent ?? "").toContain("Topic");
  } finally { dispose(); }
});

it("an epoch move with no list open asks nothing", async () => {
  const read = vi.spyOn(backend(), "quickSwitch").mockResolvedValue([row]);
  loadSingle({ name: "Completion idle", title: "Completion idle", kind: "page", pre_block: null,
    blocks: [{ id: "completion-idle", raw: "", collapsed: false, children: [] }] });
  startEditing("completion-idle", 0);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <For each={pageByName("Completion idle")?.roots ?? []}>{id => <Block id={id} />}</For>, root);
  try {
    bumpGraphEpoch(); await flush();
    expect(read).not.toHaveBeenCalled();
  } finally { dispose(); }
});
