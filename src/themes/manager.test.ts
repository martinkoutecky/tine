import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { setToasts } from "../toasts";
import { initThemePackages, installThemePackage, installedThemes, uninstallThemePackage } from "./manager";

const KEY = "theme.packages.v1";

function theme(id: string, extra: Record<string, unknown> = {}) {
  return {
    schemaVersion: 1,
    id,
    name: id,
    version: "1.0.0",
    apiVersion: "0.1",
    description: "A test theme.",
    author: "Tine",
    license: "MIT",
    source: "https://example.invalid/theme",
    modes: { dark: { "--ls-primary-background-color": "#010203" } },
    screenshots: [],
    ...extra,
  };
}

let stored: Record<string, string>;
let getFails: boolean;

beforeEach(() => {
  stored = {};
  getFails = false;
  vi.spyOn(backend(), "getAppString").mockImplementation(async (key, fallback) => {
    if (getFails) throw new Error("transient read failure");
    return stored[key] ?? fallback;
  });
  vi.spyOn(backend(), "setAppString").mockImplementation(async (key, value) => { stored[key] = value; });
});
afterEach(() => { vi.restoreAllMocks(); setToasts([]); });

const ids = () => (JSON.parse(stored[KEY] ?? "[]") as Array<{ id: string }>).map((entry) => entry.id);

it("a failed startup read never lets the next install replace the stored list", async () => {
  stored[KEY] = JSON.stringify([theme("page.tine.theme.a"), theme("page.tine.theme.b")]);
  getFails = true;
  await initThemePackages();
  expect(installedThemes()).toEqual([]);
  await expect(installThemePackage(theme("page.tine.theme.c"))).rejects.toThrow(/not overwriting/);
  expect(ids()).toEqual(["page.tine.theme.a", "page.tine.theme.b"]);
  // Once the read works again the install retries the read and keeps the others.
  getFails = false;
  await installThemePackage(theme("page.tine.theme.c"));
  expect(ids()).toEqual(["page.tine.theme.a", "page.tine.theme.b", "page.tine.theme.c"]);
});

it("an unparseable stored list is refused, not overwritten", async () => {
  stored[KEY] = "{not json";
  await initThemePackages();
  await expect(installThemePackage(theme("page.tine.theme.c"))).rejects.toThrow(/not overwriting/);
  await expect(uninstallThemePackage("page.tine.theme.c@1.0.0")).rejects.toThrow(/not overwriting/);
  expect(stored[KEY]).toBe("{not json");
});

it("an entry this build cannot parse (a newer Tine) is carried through install and uninstall", async () => {
  const future = { ...theme("page.tine.theme.future"), schemaVersion: 99, newField: { deep: true } };
  stored[KEY] = JSON.stringify([theme("page.tine.theme.a"), future]);
  await initThemePackages();
  expect(installedThemes().map((t) => t.manifest.id)).toEqual(["page.tine.theme.a"]);
  await installThemePackage(theme("page.tine.theme.c"));
  await uninstallThemePackage("page.tine.theme.a@1.0.0");
  const after = JSON.parse(stored[KEY]) as unknown[];
  expect(after).toContainEqual(future);
  expect(ids().sort()).toEqual(["page.tine.theme.c", "page.tine.theme.future"]);
});

it("concurrent installs and uninstalls do not lose each other's change", async () => {
  await initThemePackages();
  const slow = backend().setAppString as unknown as ReturnType<typeof vi.fn>;
  slow.mockImplementation(async (key: string, value: string) => {
    await new Promise((resolve) => setTimeout(resolve, 5));
    stored[key] = value;
  });
  await installThemePackage(theme("page.tine.theme.a"));
  await Promise.all([
    installThemePackage(theme("page.tine.theme.b")),
    installThemePackage(theme("page.tine.theme.c")),
    uninstallThemePackage("page.tine.theme.a@1.0.0"),
  ]);
  expect(ids().sort()).toEqual(["page.tine.theme.b", "page.tine.theme.c"]);
  expect(installedThemes().map((t) => t.manifest.id).sort()).toEqual(["page.tine.theme.b", "page.tine.theme.c"]);
});
