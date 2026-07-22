import { afterEach, describe, expect, it } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { installKeybindings } from "../keybindings";
import { clearTransientLayersForTest, registerTransientLayer, topTransientLayer } from "../transientLayers";
import { WelcomeLayer } from "./Welcome";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

function escape(init: { composing?: boolean; keyCode?: number } = {}) {
  const event = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
  if (init.composing) Object.defineProperty(event, "isComposing", { value: true });
  if (init.keyCode != null) Object.defineProperty(event, "keyCode", { value: init.keyCode });
  return event;
}

function harness(initialMandatory: boolean, initialOptional: boolean) {
  let mandatory = () => initialMandatory;
  let optional = () => initialOptional;
  let setMandatory = (_value: boolean) => {};
  let setOptional = (_value: boolean) => {};
  const host = document.createElement("div");
  document.body.append(host);
  const Harness = () => {
    const [mandatorySignal, writeMandatory] = createSignal(initialMandatory);
    const [optionalSignal, writeOptional] = createSignal(initialOptional);
    mandatory = mandatorySignal;
    optional = optionalSignal;
    setMandatory = (value) => { writeMandatory(value); };
    setOptional = (value) => { writeOptional(value); };
    return <WelcomeLayer mandatory={mandatory()} optionalOpen={optional()} onClose={() => setOptional(false)} />;
  };
  const dispose = render(() => <Harness />, host);
  return { host, mandatory, setMandatory, optional, setOptional, dispose };
}

afterEach(() => {
  clearTransientLayersForTest();
  document.body.replaceChildren();
});

describe("GH #161 mandatory and optional Welcome ownership", () => {
  it("lets mandatory Welcome dominate an optional signal and leaves Escape for the lower/root rung", async () => {
    const view = harness(true, true);
    const uninstall = installKeybindings();
    const lowerRoot = document.createElement("button");
    document.body.append(lowerRoot);
    let lowerDismissals = 0;
    let unregisterLower = () => {};
    unregisterLower = registerTransientLayer({
      id: "welcome-root-fallback",
      root: () => lowerRoot,
      dismiss: () => { lowerDismissals += 1; unregisterLower(); return true; },
    });
    try {
      await tick();
      expect(view.host.querySelector(".welcome-overlay")).not.toBeNull();
      expect(topTransientLayer()?.id).toBe("welcome-root-fallback");
      view.host.querySelector(".welcome-card")!.dispatchEvent(escape());
      await tick();
      expect(lowerDismissals).toBe(1);
      expect(view.optional()).toBe(true);
      expect(view.host.querySelector(".welcome-overlay")).not.toBeNull();
      expect(topTransientLayer()).toBeUndefined();
    } finally {
      unregisterLower();
      uninstall();
      view.dispose();
    }
  });

  it("dismisses optional Welcome exactly once and cleans its owner", async () => {
    const view = harness(false, true);
    const uninstall = installKeybindings();
    try {
      await tick();
      expect(topTransientLayer()?.id).toBe("welcome");
      const event = escape();
      view.host.querySelector(".welcome-card")!.dispatchEvent(event);
      await tick();
      expect(event.defaultPrevented).toBe(true);
      expect(view.optional()).toBe(false);
      expect(view.host.querySelector(".welcome-overlay")).toBeNull();
      expect(topTransientLayer()).toBeUndefined();
    } finally {
      uninstall();
      view.dispose();
    }
  });

  it("changes ownership reactively in both mandatory/optional transition directions", async () => {
    const view = harness(true, true);
    try {
      await tick();
      expect(topTransientLayer()).toBeUndefined();
      view.setMandatory(false);
      await tick();
      expect(topTransientLayer()?.id).toBe("welcome");
      view.setMandatory(true);
      await tick();
      expect(view.host.querySelector(".welcome-overlay")).not.toBeNull();
      expect(topTransientLayer()).toBeUndefined();
      view.setOptional(false);
      view.setMandatory(false);
      await tick();
      expect(view.host.querySelector(".welcome-overlay")).toBeNull();
      expect(topTransientLayer()).toBeUndefined();
    } finally {
      view.dispose();
    }
  });

  it.each([escape({ composing: true }), escape({ keyCode: 229 })])("does not consume an IME-owned Escape", async (event) => {
    const view = harness(false, true);
    const uninstall = installKeybindings();
    try {
      await tick();
      view.host.querySelector(".welcome-card")!.dispatchEvent(event);
      await tick();
      expect(event.defaultPrevented).toBe(false);
      expect(view.optional()).toBe(true);
      expect(topTransientLayer()?.id).toBe("welcome");
    } finally {
      uninstall();
      view.dispose();
    }
  });
});
