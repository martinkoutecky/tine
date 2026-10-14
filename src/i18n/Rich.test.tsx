import { afterEach, expect, it, vi } from "vitest";
import * as i18n from "./index";
import { render as mount } from "solid-js/web";
import { msg, setActiveLocale } from "./index";
import { Rich } from "./Rich";

let dispose = () => {};
afterEach(async () => {
  dispose();
  document.body.replaceChildren();
  await setActiveLocale("en");
  vi.restoreAllMocks();
});

it("keeps the whole conflict sentence and its emphasis while graph names stay literal", () => {
  const name = '</strong><a>Notes & plans</a>';
  dispose = mount(() => <Rich message={msg("conflict-bar-live-notice", { name })} />, document.body);
  expect(document.body.textContent).toBe(`“${name}” changed on disk while you were editing it. Your edits are kept; review them against the disk version on the page.`);
  expect(document.querySelector("strong")?.textContent).toBe(`“${name}” changed on disk while you were editing it.`);
  expect(document.querySelector("a")).toBeNull();
});

it("maps inline slots to owned elements", () => {
  dispose = mount(() => <Rich message={msg("quick-switcher-create-named-page", { name: "My notes" })}
    slots={{ strong: children => <strong data-page-name>{children}</strong> }} />, document.body);
  expect(document.body.textContent).toBe("Create page: My notes");
  expect(document.querySelector("[data-page-name]")?.textContent).toBe("My notes");
});

it("maps repeated and nested named slots while unknown markup stays literal", () => {
  vi.spyOn(i18n, "render").mockReturnValue("<key>Ctrl</key>+<key>Shift</key> <hint>read <em-hint>this</em-hint></hint> <unknown><strong>literal</strong></unknown>");
  dispose = mount(() => <Rich message={msg("settings-language-label")} slots={{
    key: children => <kbd>{children}</kbd>,
    hint: children => <span>{children}</span>,
    "em-hint": children => <em>{children}</em>,
  }} />, document.body);
  expect([...document.querySelectorAll("kbd")].map(node => node.textContent)).toEqual(["Ctrl", "Shift"]);
  expect(document.querySelector("span em")?.textContent).toBe("this");
  expect(document.body.textContent).toBe("Ctrl+Shift read this <unknown><strong>literal</strong></unknown>");
  expect(document.querySelector("strong")).toBeNull();
});

it("follows a language change while mounted", async () => {
  dispose = mount(() => <Rich message={msg("settings-language-label")} />, document.body);
  expect(document.body.textContent).toBe("Language");
  await setActiveLocale("zh-CN");
  expect(document.body.textContent).toBe("语言");
});
