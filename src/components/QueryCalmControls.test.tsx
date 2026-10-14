import { afterEach, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { QueryDisplay } from "./QueryDisplay";
import { ValueEditor } from "./querySheetParts";
import type { ViewSettings } from "../editor/queryIr";

afterEach(() => { document.body.replaceChildren(); vi.restoreAllMocks(); });

it("shows the saved sample limit and leaves it unchanged when the inline View field blurs", () => {
  const root = document.createElement("div"); document.body.append(root);
  const apply = vi.fn();
  const registry = { rows: () => [], pending: () => false, failure: () => null, unavailable: () => false, request: () => {}, retry: () => {} };
  const dispose = render(() => <QueryDisplay inline view={() => ({ sample: 7 })} apply={apply} registry={registry} rowKind={() => "block"} />, root);
  try {
    const input = root.querySelector<HTMLInputElement>('[aria-label="Sample size"]')!;
    expect(input.value).toBe("7");
    input.dispatchEvent(new FocusEvent("blur"));
    expect(apply).not.toHaveBeenCalled();
  } finally { dispose(); }
});

it("starts the column picker with visible defaults and selects all or none", () => {
  const root = document.createElement("div"); document.body.append(root);
  const [view, setView] = createSignal<ViewSettings>({});
  const registry = { rows: () => [], pending: () => false, failure: () => null, unavailable: () => false, request: () => {}, retry: () => {} };
  const dispose = render(() => <QueryDisplay inline view={view} apply={(next) => { setView(next); }}
    defaultColumns={() => ["state", "page", "owner"]} registry={registry} rowKind={() => "block"} />, root);
  try {
    expect([...root.querySelectorAll<HTMLInputElement>('[aria-label="Columns"] input')].filter((input) => input.checked)).toHaveLength(3);
    const owner = root.querySelector<HTMLInputElement>('[aria-label="owner"]')!;
    owner.click();
    expect(view().columns).toEqual(["state", "page"]);
    [...root.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent === "Select none")!.click();
    expect(view().columns).toEqual([]);
    [...root.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent === "Select all")!.click();
    expect(view().columns).toEqual(["state", "page", "owner"]);
  } finally { dispose(); }
});

it.each(["scheduled", "deadline"] as const)("offers relative engine comparisons for %s", (kind) => {
  const root = document.createElement("div"); document.body.append(root);
  const commit = vi.fn();
  const dispose = render(() => <ValueEditor kind={kind} onCommit={commit} />, root);
  try {
    const op = root.querySelector<HTMLSelectElement>('[aria-label="Date comparison"]');
    expect(op).not.toBeNull();
    expect([...op!.options].map((option) => option.value)).toEqual(["is_set", "is_not_set", "gt", "ge", "eq", "lt", "le"]);
    op!.value = "lt"; op!.dispatchEvent(new Event("change", { bubbles: true }));
    const input = root.querySelector<HTMLInputElement>('input')!;
    input.value = "+5d"; input.dispatchEvent(new Event("input", { bubbles: true }));
    [...root.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent === "Apply")!.click();
    expect(commit).toHaveBeenCalledWith({ kind: "leaf", leaf: { kind: "attr", attr: kind, op: "lt", value: { kind: "date", literal: "+5d" } } });
  } finally { dispose(); }
});
