import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { ToolbarSettings } from "./ToolbarSettings";
import { backend } from "../backend";
import { saveToolbarChoice, toolbarChoice } from "../toolbarChoice";

let dispose = () => {};
afterEach(async () => { dispose(); saveToolbarChoice(null); await Promise.resolve(); vi.restoreAllMocks(); document.body.innerHTML = ""; });

it("moves between lists and reorders with accessible touch/keyboard controls, then resets", () => {
  vi.spyOn(backend(), "setAppString").mockResolvedValue();
  const host = document.body.appendChild(document.createElement("div"));
  dispose = render(ToolbarSettings, host);
  host.querySelector<HTMLButtonElement>('[aria-label="Move Calendar to the … menu"]')!.click();
  expect(toolbarChoice()?.bar).not.toContain("calendar");
  expect(host.querySelector('[data-toolbar-section="menu"] [data-toolbar-choice="calendar"]')).not.toBeNull();
  host.querySelector<HTMLButtonElement>('[aria-label="Move Settings up"]')!.click();
  expect(toolbarChoice()?.bar.slice(-2)).toEqual(["settings", "workspaces"]);
  [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "Reset toolbar")!.click();
  expect(toolbarChoice()).toBeNull();
  expect(host.querySelectorAll('[data-toolbar-section="bar"] li')).toHaveLength(7);
});
