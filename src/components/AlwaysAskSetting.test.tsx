import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { Backend } from "../backend";
import { closeSettings, openSettings } from "../ui";
import { conflictPolicyAlwaysAsk, setConflictPolicyAlwaysAsk } from "../conflictPolicy";

const fake = vi.hoisted(() => ({
  getBackupKeep: vi.fn(async () => 12),
  listBackups: vi.fn(async () => []),
  setBackupKeep: vi.fn(),
  setAppBool: vi.fn(async () => undefined),
  getAppBool: vi.fn(async () => false),
}));

vi.mock("../backend", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../backend")>();
  return { ...actual, backend: () => fake as unknown as Backend, isTauri: () => false };
});

import { Settings } from "./Settings";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const LABEL = "Always ask before applying an external change";

afterEach(() => {
  closeSettings();
  setConflictPolicyAlwaysAsk(false);
  document.body.innerHTML = "";
  vi.clearAllMocks();
});

describe("Settings → Backups & recovery → Always ask (22a policy, mounted by 22b)", () => {
  it("shows the switch the Guide names and turns the policy on and off, remembering it", async () => {
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Settings />, root);
    openSettings("backups");
    await tick();
    const field = root.querySelector(`[data-setting-label="${LABEL}"]`);
    expect(field, "the Guide's Settings path must reach a real control").not.toBeNull();
    const toggle = field!.querySelector('[role="switch"]') as HTMLButtonElement;
    expect(toggle.getAttribute("aria-checked")).toBe("false");

    toggle.click();
    expect(conflictPolicyAlwaysAsk()).toBe(true);
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    expect(fake.setAppBool).toHaveBeenLastCalledWith("concord_always_ask", true);

    toggle.click();
    expect(conflictPolicyAlwaysAsk()).toBe(false);
    expect(fake.setAppBool).toHaveBeenLastCalledWith("concord_always_ask", false);
    dispose();
  });
});
