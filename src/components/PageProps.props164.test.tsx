// GH #164 (og 14 Q5, family 4): edit ARBITRARY page and block properties from
// the UI, not just the five page presets. Ported in meaning from master
// `PageProps.props164.test.tsx` (eae864acb). Kept in its own file because
// transientFallbacks.p1d2.test.tsx reaches for the FIRST `.pp-input`, so the
// preset fields must keep their position.

import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { type JSX } from "solid-js";
import { render } from "solid-js/web";
import { PageProps } from "./PageProps";
import { closePageProps, openBlockProps, openPageProps } from "../ui";
import { clearTransientLayersForTest } from "../transientLayers";
import { blockProperty, pageByName, readPageProperty, resetStore } from "../document";
import { doc } from "../document/model";
import { loadSingle } from "../document/workingSet";
import { initParser } from "../render/parse";
import { clearSeededFacets } from "../render/facets";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

function mount(node: () => JSX.Element) {
  const host = document.createElement("div");
  document.body.append(host);
  return { host, dispose: render(node, host) };
}

function loadPage(name: string, preBlock: string | null, raw = "body", readOnly = false) {
  loadSingle({
    name,
    kind: "page",
    title: name,
    pre_block: preBlock,
    blocks: [{ id: "body-1", raw, collapsed: false, children: [] }],
    read_only: readOnly,
  });
  // A test DTO carries no parsed `properties`; drop the DTO-seeded (empty)
  // facets so block reads derive from raw like the real backend's DTO does.
  clearSeededFacets();
}

const fieldLabels = (host: HTMLElement) =>
  Array.from(host.querySelectorAll<HTMLElement>(".pp-field .pp-label")).map((el) => el.textContent ?? "");

function typeInto(input: HTMLInputElement, value: string) {
  input.focus();
  input.value = value;
  input.dispatchEvent(new InputEvent("input", { bubbles: true, cancelable: true }));
}

beforeAll(async () => {
  await initParser();
});

afterEach(() => {
  closePageProps();
  clearTransientLayersForTest();
  resetStore();
  document.body.innerHTML = "";
});

describe("properties panel — arbitrary keys (GH #164)", () => {
  it("lists a property the page actually has, even when it is not a preset", async () => {
    loadPage("Notes", "status:: draft");
    openPageProps("Notes", 20, 20);
    const { host, dispose } = mount(() => <PageProps />);
    try {
      await tick();
      expect(fieldLabels(host)).toContain("status");
      // Machine-managed keys keep their own surfaces.
      expect(fieldLabels(host)).not.toContain("id");
    } finally {
      dispose();
    }
  });

  it("adds a new property from the add-row; the canonical reader reads it back", async () => {
    loadPage("Notes", "status:: draft");
    openPageProps("Notes", 20, 20);
    const { host, dispose } = mount(() => <PageProps />);
    try {
      await tick();
      const keyInput = host.querySelector<HTMLInputElement>("input.pp-add-key");
      const valueInput = host.querySelector<HTMLInputElement>("input.pp-add-value");
      expect(keyInput).not.toBeNull();
      typeInto(keyInput!, "reviewer");
      typeInto(valueInput!, "hodnota");
      host.querySelector<HTMLButtonElement>("button.pp-add-commit")!.click();
      await tick();
      expect(readPageProperty("Notes", "reviewer")).toBe("hodnota");
      expect(readPageProperty("Notes", "status")).toBe("draft");
      // It is now a listed row, and removable from the panel.
      expect(fieldLabels(host)).toContain("reviewer");
      host.querySelector<HTMLButtonElement>('button.pp-remove[title="Remove reviewer"]')!.click();
      await tick();
      expect(readPageProperty("Notes", "reviewer")).toBeNull();
      expect(readPageProperty("Notes", "status")).toBe("draft");
    } finally {
      dispose();
    }
  });

  it("refuses an add-row key the property reader could not find again", async () => {
    loadPage("Notes", null);
    openPageProps("Notes", 20, 20);
    const { host, dispose } = mount(() => <PageProps />);
    try {
      await tick();
      const commit = () => host.querySelector<HTMLButtonElement>("button.pp-add-commit")!;
      for (const bad of ["a b", "a::b", "#tag", "id", "tine.view", ""]) {
        typeInto(host.querySelector<HTMLInputElement>("input.pp-add-key")!, bad);
        await tick();
        expect(commit().disabled, bad).toBe(true);
      }
    } finally {
      dispose();
    }
  });

  it("lists and edits a block's own properties in block scope", async () => {
    loadPage("Notes", null, "task\nowner:: martin\nid:: 6500a1b2-0000-4000-8000-000000000001");
    expect(blockProperty("body-1", "owner")).toBe("martin");
    openBlockProps("body-1", 20, 20);
    const { host, dispose } = mount(() => <PageProps />);
    try {
      await tick();
      expect(fieldLabels(host)).toContain("owner");
      // Page presets are page-level keys and would write to the wrong subject.
      expect(fieldLabels(host)).not.toContain("Aliases");
      expect(fieldLabels(host)).not.toContain("id");
      const input = Array.from(host.querySelectorAll<HTMLInputElement>("input.pp-input"))
        .find((el) => !el.classList.contains("pp-add-key") && !el.classList.contains("pp-add-value"))!;
      typeInto(input, "martina");
      input.dispatchEvent(new FocusEvent("blur", { bubbles: true }));
      await tick();
      expect(blockProperty("body-1", "owner")).toBe("martina");
      // Only the edited line changed; the machine-managed id stays put.
      expect(doc.byId["body-1"].raw).toBe("task\nowner:: martina\nid:: 6500a1b2-0000-4000-8000-000000000001");
    } finally {
      dispose();
    }
  });

  it("opening one scope closes the other: one panel at a time", async () => {
    loadPage("Notes", "status:: draft", "task\nowner:: martin");
    const { host, dispose } = mount(() => <PageProps />);
    try {
      openBlockProps("body-1", 20, 20);
      await tick();
      openPageProps("Notes", 20, 20);
      await tick();
      expect(host.querySelectorAll(".page-props-panel").length).toBe(1);
      expect(fieldLabels(host)).toContain("Aliases");
      openBlockProps("body-1", 20, 20);
      await tick();
      expect(host.querySelectorAll(".page-props-panel").length).toBe(1);
      expect(fieldLabels(host)).not.toContain("Aliases");
      host.querySelector<HTMLButtonElement>("button.pp-done")!.click();
    } finally {
      dispose();
    }
  });

  it("lists and edits an Org page's `#+key:` directives in Org form", async () => {
    loadSingle({
      name: "OrgNotes", kind: "page", title: "OrgNotes", format: "org",
      pre_block: "#+title: OrgNotes\n#+status: draft",
      blocks: [{ id: "o-1", raw: "body", collapsed: false, children: [] }],
    });
    openPageProps("OrgNotes", 20, 20);
    const { host, dispose } = mount(() => <PageProps />);
    try {
      await tick();
      expect(fieldLabels(host)).toContain("status");
      const status = Array.from(host.querySelectorAll<HTMLElement>(".pp-field"))
        .find((f) => f.querySelector(".pp-label")?.textContent === "status")!
        .querySelector<HTMLInputElement>("input.pp-input")!;
      expect(status.value).toBe("draft");
      typeInto(status, "final");
      status.dispatchEvent(new FocusEvent("blur", { bubbles: true }));
      await tick();
      expect(pageByName("OrgNotes")?.preBlock).toBe("#+title: OrgNotes\n#+status: final");
    } finally {
      dispose();
    }
  });

  it("lists a Markdown page header that lives in a properties-only first root", async () => {
    loadPage("Journalish", null, "owner:: martin\nstatus:: draft");
    openPageProps("Journalish", 20, 20);
    const { host, dispose } = mount(() => <PageProps />);
    try {
      await tick();
      expect(fieldLabels(host)).toEqual(expect.arrayContaining(["owner", "status"]));
    } finally {
      dispose();
    }
  });

  it("offers no property editing on a read-only page", async () => {
    loadPage("Readonly", "status:: draft", "body", true);
    openPageProps("Readonly", 20, 20);
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
});
