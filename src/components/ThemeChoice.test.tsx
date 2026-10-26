import { afterEach, expect, it } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { ThemeChoice } from "./ThemeChoice";
import { Rich } from "../i18n/Rich";
import { msg, setActiveLocale } from "../i18n";

let dispose: (() => void) | undefined;
afterEach(async () => {
  dispose?.();
  document.body.innerHTML = "";
  await setActiveLocale("en");
});

it("renders the three appearance choices with the default English labels", () => {
  const host = document.createElement("div");
  document.body.append(host);
  dispose = render(() => <ThemeChoice />, host);
  expect(host.querySelector('[role="radiogroup"]')?.getAttribute("aria-label")).toBe("Appearance");
  expect([...host.querySelectorAll('[role="radio"]')].map((el) => el.textContent?.slice(1))).toEqual(["Light", "System", "Dark"]);
});

it("keeps catalog emphasis while preserving external values as text and updating reactively", () => {
  const host = document.createElement("div");
  document.body.append(host);
  const [name, setName] = createSignal("<code>notes & names</code>");
  dispose = render(() => <Rich message={msg("page-open-failed", { error: name() })} slots={{ "code": (children) => <code>{children}</code> }} />, host);
  expect(host.querySelectorAll("code")).toHaveLength(1);
  expect(host.querySelector("code")?.textContent).toBe("<code>notes & names</code>");
  expect(host.textContent).toBe("Couldn't open this page: <code>notes & names</code>");
  expect(host.querySelector("img")).toBeNull();
  setName("Another page");
  expect(host.querySelector("code")?.textContent).toBe("Another page");
});

it("renders every pane split instruction as a complete sentence with its nested emphasis", () => {
  const host = document.createElement("div");
  const [kind, setKind] = createSignal("edge");
  dispose = render(() => <Rich message={msg("app-pane-split-instruction", { kind: kind() })} slots={{ "kbd": (children) => <kbd>{children}</kbd>, "span": (children) => <span>{children}</span>, "em-hint": (children) => <em>{children}</em> }} />, host);
  expect(host.textContent).toBe("Enter split the whole window · type a page name (or Ctrl+K) to open it in the new split");
  expect(host.querySelector("em")?.textContent).toBe("whole window");
  setKind("pane-edge");
  expect(host.textContent).toBe("Enter split this pane · type a page name (or Ctrl+K) to open it in the new split");
  setKind("seam");
  expect(host.textContent).toBe("Enter split here (mirrors the pane) · type a page name (or Ctrl+K) to open it in the new split");
});
