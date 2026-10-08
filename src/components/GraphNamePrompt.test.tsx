import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { GraphNamePrompt } from "./GraphNamePrompt";
import { askGraphName } from "../graphNamePrompt";
import { invalidateBinding } from "../binding";
import { readFileSync } from "node:fs";

let dispose: (() => void) | undefined;
afterEach(() => { dispose?.(); document.body.replaceChildren(); });
const tick = async () => { for (let i = 0; i < 8; i++) await Promise.resolve(); };

it("prefills the Rust suggestion and cancel creates nothing", async () => {
  dispose = render(() => <GraphNamePrompt />, document.body);
  const create = vi.fn(async () => "/parent/notes-2");
  const result = askGraphName("notes-2", create);
  await tick();
  expect(document.querySelector("input")?.value).toBe("notes-2");
  expect(create).not.toHaveBeenCalled();
  [...document.querySelectorAll("button")].find(b => b.textContent === "Cancel")!.click();
  await expect(result).resolves.toBeNull();
  expect(create).not.toHaveBeenCalled();
});

it("shows Rust's refusal inline and stays open for another name", async () => {
  dispose = render(() => <GraphNamePrompt />, document.body);
  const create = vi.fn().mockRejectedValueOnce(new Error("Graph folder is not empty. Choose another name."))
    .mockResolvedValueOnce("/parent/My notes");
  const result = askGraphName("notes", create);
  await tick();
  document.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  await tick();
  expect(document.querySelector('[role="alert"]')?.textContent).toContain("Graph folder is not empty. Choose another name.");
  expect(document.querySelector('[role="dialog"]')).not.toBeNull();
  const input = document.querySelector("input")!;
  input.value = "My notes";
  input.dispatchEvent(new Event("input", { bubbles: true }));
  document.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  await expect(result).resolves.toBe("/parent/My notes");
  expect(create).toHaveBeenLastCalledWith("My notes");
  expect(document.querySelector('[role="dialog"]')).toBeNull();
});

it("aborts a pending prompt when its graph binding changes", async () => {
  dispose = render(() => <GraphNamePrompt />, document.body);
  const create = vi.fn(async () => "/parent/notes");
  const result = askGraphName("notes", create);
  await tick();
  invalidateBinding();
  await expect(result).resolves.toBeNull();
  expect(document.querySelector('[role="dialog"]')).toBeNull();
  expect(create).not.toHaveBeenCalled();
});

it("places the name prompt above mandatory Welcome", async () => {
  const css = readFileSync("src/styles/app.css", "utf8") + readFileSync("src/styles/pdf-workspace.css", "utf8");
  const style = document.createElement("style");
  // Use the actual layer declarations; no pixel or incidental z-index oracle.
  style.textContent = [...css.matchAll(/\.(?:modal-overlay(?:\.graph-name-overlay)?|welcome-overlay)\s*\{[^}]+\}/g)]
    .map(match => match[0]).join("\n");
  document.head.append(style);
  const welcome = document.createElement("div"); welcome.className = "welcome-overlay";
  document.body.append(welcome);
  dispose = render(() => <GraphNamePrompt />, document.body);
  const result = askGraphName("notes", vi.fn(async () => "/parent/notes"));
  await tick();
  try {
    const overlay = document.querySelector<HTMLElement>(".graph-name-overlay")!;
    expect(Number(getComputedStyle(overlay).zIndex)).toBeGreaterThan(Number(getComputedStyle(welcome).zIndex));
  } finally {
    [...document.querySelectorAll("button")].find(b => b.textContent === "Cancel")!.click();
    await result; style.remove();
  }
});
