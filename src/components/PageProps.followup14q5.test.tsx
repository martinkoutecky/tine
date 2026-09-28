// og 14 Q5 follow-up: the properties panel's write binding.
// - Reader B blocker B2: after a graph switch / store reset the panel refuses
//   visibly and closes; it never drops a typed edit silently. An unknown block
//   id never shows a writable panel.
// - G3 required neighbor: a write is bound to the subject instance captured at
//   open, so a same-graph external reload (page-name or block-id reuse) refuses
//   instead of overwriting the intervening change.
// - G3 blocker: the add-row explains why a key is refused.
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { type JSX } from "solid-js";
import { render } from "solid-js/web";
import { PageProps } from "./PageProps";
import { closePageProps, openBlockProps, openPageProps, pagePropsPanel } from "../ui";
import { clearTransientLayersForTest } from "../transientLayers";
import { blockProperty, pageByName, readPageProperty, resetStore } from "../document";
import { loadSingle, reloadPage } from "../document/workingSet";
import { initParser } from "../render/parse";
import { clearSeededFacets } from "../render/facets";
import { invalidateBinding } from "../binding";
import { setToasts, toasts } from "../toasts";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

function mount(node: () => JSX.Element) {
  const host = document.createElement("div");
  document.body.append(host);
  return { host, dispose: render(node, host) };
}

const dto = (preBlock: string | null, raw: string) => ({
  name: "Notes", kind: "page" as const, title: "Notes", pre_block: preBlock,
  blocks: [{ id: "body-1", raw, collapsed: false, children: [] }],
});

function load(preBlock: string | null, raw = "body") {
  loadSingle(dto(preBlock, raw));
  clearSeededFacets();
}

function field(host: HTMLElement, label: string): HTMLInputElement {
  return Array.from(host.querySelectorAll<HTMLElement>(".pp-field"))
    .find((f) => f.querySelector(".pp-label")?.textContent === label)!
    .querySelector<HTMLInputElement>("input.pp-input")!;
}

function typeAndBlur(input: HTMLInputElement, value: string) {
  input.focus();
  input.value = value;
  input.dispatchEvent(new InputEvent("input", { bubbles: true, cancelable: true }));
  input.dispatchEvent(new FocusEvent("blur", { bubbles: true }));
}

const errorToasts = () => toasts().filter((t) => t.kind === "error").map((t) => t.message);

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  closePageProps();
  clearTransientLayersForTest();
  resetStore();
  setToasts([]);
  document.body.innerHTML = "";
});

describe("properties panel binding (og 14 Q5 follow-up)", () => {
  it("after a store reset, a typed edit is refused visibly and the panel closes", async () => {
    load("status:: draft");
    openPageProps("Notes", 20, 20);
    const { host, dispose } = mount(() => <PageProps />);
    try {
      await tick();
      invalidateBinding();
      typeAndBlur(field(host, "status"), "final");
      await tick();
      expect(readPageProperty("Notes", "status")).toBe("draft");
      expect(errorToasts().length).toBe(1);
      expect(pagePropsPanel()).toBeNull();
    } finally {
      dispose();
    }
  });

  it("an unknown block id opens a panel with nothing writable", async () => {
    load(null, "task");
    openBlockProps("no-such-block", 20, 20);
    const { host, dispose } = mount(() => <PageProps />);
    try {
      await tick();
      expect(host.querySelector(".page-props-panel")).not.toBeNull();
      expect(host.querySelectorAll("input.pp-input").length).toBe(0);
      expect(host.querySelector("button.pp-add-commit")).toBeNull();
    } finally {
      dispose();
    }
  });

  it("page: an external reload while open refuses the stale write instead of overwriting it", async () => {
    load("status:: draft");
    openPageProps("Notes", 20, 20);
    const { host, dispose } = mount(() => <PageProps />);
    try {
      await tick();
      reloadPage(dto("status:: external", "body"));
      clearSeededFacets();
      typeAndBlur(field(host, "status"), "mine");
      await tick();
      expect(pageByName("Notes")?.preBlock).toBe("status:: external");
      expect(errorToasts().length).toBe(1);
      expect(pagePropsPanel()).toBeNull();
    } finally {
      dispose();
    }
  });

  it("block: a reload that reuses the block id refuses the stale write", async () => {
    load(null, "task\nowner:: martin");
    openBlockProps("body-1", 20, 20);
    const { host, dispose } = mount(() => <PageProps />);
    try {
      await tick();
      // A real reload DTO carries its block facets (seeded, as the backend ships them).
      const next = dto(null, "other block\nowner:: ann");
      reloadPage({ ...next, blocks: [{ ...next.blocks[0], properties: [["owner", "ann"]] }] });
      typeAndBlur(field(host, "owner"), "martina");
      await tick();
      expect(blockProperty("body-1", "owner")).toBe("ann");
      expect(errorToasts().length).toBe(1);
      expect(pagePropsPanel()).toBeNull();
    } finally {
      dispose();
    }
  });

  it("the add-row explains a refused key and does not write it", async () => {
    load("klíč:: hodnota");
    openPageProps("Notes", 20, 20);
    const { host, dispose } = mount(() => <PageProps />);
    try {
      await tick();
      const key = host.querySelector<HTMLInputElement>("input.pp-add-key")!;
      key.focus();
      key.value = "nový";
      key.dispatchEvent(new InputEvent("input", { bubbles: true, cancelable: true }));
      await tick();
      expect(host.querySelector<HTMLButtonElement>("button.pp-add-commit")!.disabled).toBe(true);
      expect(host.querySelector(".pp-add .pp-error")?.textContent ?? "").toMatch(/ASCII/);
      // An existing key outside the grammar is listed, shown read-only, and removable.
      expect(field(host, "klíč").disabled).toBe(true);
      host.querySelector<HTMLButtonElement>('button.pp-remove[title="Remove klíč"]')!.click();
      await tick();
      expect(readPageProperty("Notes", "klíč")).toBeNull();
    } finally {
      dispose();
    }
  });
});
