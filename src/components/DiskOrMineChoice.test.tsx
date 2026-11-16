import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { DiskOrMineChoice } from "./DiskOrMineChoice";
import { ExternalChangeBar } from "./ExternalChangeBar";
import { holdExternalChange, heldExternalChangeFor, installHeldExternalChangeApplier, clearHeldExternalChanges } from "../conflictPolicy";
import type { GraphChange } from "../backendTypes";

let dispose: (() => void) | undefined;
afterEach(() => { dispose?.(); dispose = undefined; clearHeldExternalChanges(); document.body.innerHTML = ""; });

function mount(view: () => any) {
  const host = document.createElement("div");
  document.body.append(host);
  dispose = render(view, host);
  return host;
}

it("D-20261018-UI2a: [Keep mine][Use disk version], neither styled primary or danger", () => {
  const host = mount(() => <DiskOrMineChoice buttonClass="conflict-btn" onKeepMine={() => {}} onUseDisk={() => {}} />);
  const buttons = [...host.querySelectorAll("button")];
  expect(buttons.map((b) => b.textContent)).toEqual(["Keep mine", "Use disk version"]);
  for (const button of buttons) expect(button.className).toBe("conflict-btn");
});

it("the external-change bar offers the same two labels in the same order and each does its own thing", () => {
  const applied = vi.fn();
  installHeldExternalChangeApplier(applied);
  holdExternalChange("P", { kind: "page" } as unknown as GraphChange);
  const host = mount(() => <ExternalChangeBar name="P" />);
  expect([...host.querySelectorAll("button")].map((b) => b.textContent)).toEqual(["Keep mine", "Use disk version"]);
  expect(host.querySelector(".settings-btn-primary")).toBeNull();
  host.querySelectorAll("button")[0].click();
  expect(heldExternalChangeFor("P")).toBe(false);
  expect(applied).not.toHaveBeenCalled();
  holdExternalChange("P", { kind: "page" } as unknown as GraphChange);
  host.querySelectorAll("button")[1].click();
  expect(applied).toHaveBeenCalledOnce();
});
