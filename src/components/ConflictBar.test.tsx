import { afterEach, beforeEach, expect, it } from "vitest";
import { render } from "solid-js/web";
import { ConflictBar } from "./ConflictBar";
import { resetStore } from "../document";
import { markConflict } from "../document/save/engine";

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
beforeEach(() => { resetStore(); host = document.createElement("div"); document.body.append(host); });
afterEach(() => { dispose?.(); host.remove(); });

it("N4: a released page explains both outcomes and offers Use disk first", () => {
  markConflict("Source", { kind: "released", partner: "Destination" });
  dispose = render(() => <ConflictBar />, host);
  const message = host.querySelector(".conflict-msg")?.textContent ?? "";
  expect(message).toContain("disk version of “Destination”");
  expect(message).toContain("restore what this page gave");
  expect(message).toContain("moved content is on no page");
  expect([...host.querySelectorAll("button")].map((button) => button.textContent?.trim())).toEqual([
    "Use disk version", "Keep mine (overwrite)",
  ]);
});
