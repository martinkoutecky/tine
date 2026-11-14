import { afterEach, expect, it } from "vitest";
import { render } from "solid-js/web";
import { App } from "./App";
import { closeSettings, settingsOpen, settingsTabRequest } from "./ui";

let dispose = () => {};
afterEach(() => { dispose(); closeSettings(); document.body.innerHTML = ""; });

it("GH #926: the toolbar menu opens toolbar customization", () => {
  const host = document.body.appendChild(document.createElement("div"));
  dispose = render(() => <App />, host);
  host.querySelector<HTMLButtonElement>("[data-topbar-overflow-trigger]")!.click();
  const customize = [...host.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')]
    .find(button => button.textContent === "Customize toolbar…");
  expect(customize, "the toolbar must offer the approved customization route").toBeDefined();
  customize!.click();
  expect(settingsOpen()).toBe(true);
  expect(settingsTabRequest()).toBe("toolbar");
});
