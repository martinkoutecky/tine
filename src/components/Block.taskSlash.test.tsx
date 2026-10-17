import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { For, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { startEditing } from "../editorController";
import { initParser } from "../render/parse";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import type { BlockDto, PageDto } from "../types";
import { Block } from "./Block";
import { setActiveLocale } from "../i18n";
import { COMMANDS, filterCommands } from "../editor/autocomplete";

beforeAll(async () => {
  await initParser();
});

afterEach(async () => {
  await setActiveLocale("en");
  resetStore();
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

function taskPage(raw: string, format: "md" | "org" = "md"): PageDto {
  const block: BlockDto = { id: "task-slash-host", raw, collapsed: false, children: [] };
  return {
    name: "Task slash",
    kind: "page",
    title: "Task slash",
    pre_block: null,
    format,
    blocks: [block],
  };
}

function inputAt(textarea: HTMLTextAreaElement, value: string, caret: number) {
  textarea.focus();
  textarea.value = value;
  textarea.setSelectionRange(caret, caret);
  textarea.dispatchEvent(new InputEvent("input", {
    bubbles: true,
    inputType: "insertText",
    data: value[caret - 1] ?? null,
  }));
}

function choose(label: string) {
  const item = [...document.body.querySelectorAll<HTMLElement>(".autocomplete .ac-item")]
    .find((candidate) => candidate.querySelector(".ac-label")?.textContent === label);
  expect(item).toBeDefined();
  item!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
}

async function openAndChoose(raw: string, label: string, format: "md" | "org" = "md") {
  loadSingle(taskPage(raw, format));
  const caret = raw.indexOf(`/${label.toLowerCase()}`) + label.length + 1;
  startEditing("task-slash-host", caret);
  const { root, dispose } = mount(() => (
    <For each={pageByName("Task slash")?.roots ?? []}>{(id) => <Block id={id} />}</For>
  ));
  const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
  inputAt(textarea, raw, caret);
  await vi.waitFor(() => expect([...document.body.querySelectorAll(".autocomplete .ac-label")]
    .some((element) => element.textContent === label)).toBe(true));
  choose(label);
  return { textarea, dispose };
}

describe("task-state slash commands (GH #225)", () => {
  it("shows English slash keywords with Chinese glosses and keeps marker insertion literal (GH #107)", async () => {
    const english = COMMANDS.map((command) => command.matchLabel ?? command.label);
    const ranking = filterCommands("TODO").map((command) => command.matchTieOrder);
    await setActiveLocale("zh-CN");
    loadSingle(taskPage("/"));
    startEditing("task-slash-host", 1);
    const { root, dispose } = mount(() => <Block id="task-slash-host" />);
    try {
      const textarea = root.querySelector("textarea.block-editor") as HTMLTextAreaElement;
      inputAt(textarea, "/", 1);
      await vi.waitFor(() => expect(document.querySelectorAll(".autocomplete .ac-label").length).toBe(COMMANDS.length));
      const labels = [...document.querySelectorAll(".autocomplete .ac-label")].map((el) => el.textContent!);
      for (const keyword of english) {
        const label = labels.find((value) => value.startsWith(`${keyword} (`));
        expect(label, keyword).toMatch(/[\u3400-\u9fff]/);
      }
      expect(filterCommands("TODO").map((command) => command.matchTieOrder)).toEqual(ranking);
      expect(filterCommands("待办")[0].matchLabel).toBe("TODO");
      choose(labels.find((label) => label.startsWith("TODO ("))!);
      await vi.waitFor(() => expect(doc.byId["task-slash-host"].raw).toBe("TODO"));
    } finally { dispose(); }
  });
  it("replaces an existing marker from anywhere in a multiline block and moves the caret to the end", async () => {
    const { textarea, dispose } = await openAndChoose(
      "TODO [#A] Write report /done\nowner:: Martin",
      "DONE",
    );
    try {
      await vi.waitFor(() => expect(doc.byId["task-slash-host"].raw.split("\n")).toEqual([
        "DONE [#A] Write report ",
        "owner:: Martin",
      ]));
      await vi.waitFor(() => expect([
        textarea.selectionStart,
        textarea.selectionEnd,
      ]).toEqual([textarea.value.length, textarea.value.length]));
    } finally {
      dispose();
    }
  });

  it("adds a marker to a plain Org block without changing its content", async () => {
    const { dispose } = await openAndChoose("Buy milk /waiting", "WAITING", "org");
    try {
      await vi.waitFor(() => expect(doc.byId["task-slash-host"].raw).toBe("WAITING Buy milk "));
    } finally {
      dispose();
    }
  });
});
