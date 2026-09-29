import { afterEach, describe, expect, it, vi } from "vitest";

// OG-I2 O7 / master cdd0eda4b: the Plugins tab has ONE busy slot. An operation
// that finishes (or a confirm that is cancelled) must clear only its own hold,
// never the flag of a later operation still in flight.

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => { resolve = r; });
  return { promise, resolve };
}

const plugin = {
  manifest: {
    id: "test.plugin", name: "Test Plugin", version: "1.0.0", description: "d", author: "a", license: "MIT",
    platforms: ["desktop"], capabilities: [], source: "https://example.test", settings: [],
  },
  storageId: "s", storageVersion: "1", sha256: "x", selected: true, enabled: false, running: false, settings: {},
};

async function mount(opts: { confirm: () => Promise<boolean>; enable: () => Promise<void>; uninstall: () => Promise<void> }) {
  vi.resetModules();
  vi.doMock("../plugins/manager", () => ({
    installedPlugins: () => [plugin],
    pluginManager: { enable: opts.enable, disable: vi.fn(async () => {}), uninstall: opts.uninstall },
  }));
  vi.doMock("../plugins/registry", () => ({
    COMMUNITY_REGISTRY_ENABLED: false,
    communityPlugins: () => [],
    installCommunityPlugin: vi.fn(),
    loadSafetyReport: vi.fn(),
    refreshCommunityRegistry: vi.fn(),
    registryPersistenceError: () => null,
    registryState: () => "ready",
  }));
  vi.doMock("../backend", () => ({ backend: () => ({ confirm: opts.confirm, openExternal: vi.fn() }), isTauri: () => false }));
  vi.doMock("../platform", () => ({ platformKind: async () => "desktop" }));
  const { render } = await import("solid-js/web");
  const { PluginsTab } = await import("./PluginsTab");
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <PluginsTab />, root);
  await tick();
  [...root.querySelectorAll<HTMLButtonElement>('[role="tab"]')].find((b) => b.textContent?.startsWith("Installed"))!.click();
  await tick();
  const uninstall = () => [...root.querySelectorAll<HTMLButtonElement>("button")].find((b) => /Uninstall/.test(b.textContent ?? ""))!;
  const toggle = () => root.querySelector<HTMLButtonElement>("button.settings-toggle")!;
  return { root, dispose, uninstall, toggle };
}

afterEach(() => {
  document.body.innerHTML = "";
  vi.resetModules();
});

describe("Plugins tab busy ownership", () => {
  it("an earlier operation finishing does not re-enable controls under a later one in flight", async () => {
    const confirm = deferred<boolean>();
    const enable = deferred<void>();
    const uninstall = deferred<void>();
    const ui = await mount({ confirm: () => confirm.promise, enable: () => enable.promise, uninstall: () => uninstall.promise });

    ui.uninstall().click(); // confirm dialog open
    await tick();
    ui.toggle().click(); // must be ignored: the confirm holds the slot (old code: starts an enable)
    await tick();
    confirm.resolve(true);
    await tick(); // uninstall now in flight
    enable.resolve(); // old code: the enable's `finally` clears the slot under the uninstall
    await tick();

    expect(ui.uninstall().disabled, "controls must stay disabled while the uninstall is in flight").toBe(true);
    expect(ui.toggle().disabled).toBe(true);
    uninstall.resolve();
    await tick();
    expect(ui.toggle().disabled, "released once the last hold ends").toBe(false);
    ui.dispose();
  });

  it("a cancelled confirm releases the slot", async () => {
    const confirm = deferred<boolean>();
    const ui = await mount({ confirm: () => confirm.promise, enable: async () => {}, uninstall: async () => {} });

    ui.uninstall().click();
    await tick();
    expect(ui.toggle().disabled, "controls are held while the confirm is open").toBe(true);
    confirm.resolve(false);
    await tick();

    expect(ui.toggle().disabled).toBe(false);
    expect(ui.uninstall().disabled).toBe(false);
    ui.dispose();
  });
});
