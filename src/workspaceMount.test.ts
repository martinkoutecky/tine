import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { expectOrderedIn } from "./testOrder.test.support";

describe("window-scoped workspace chrome", () => {
  it("mounts one switcher in App chrome and never inside the pane-scoped TabBar", () => {
    const app = readFileSync("src/App.tsx", "utf8");
    const tabBar = readFileSync("src/components/TabBar.tsx", "utf8");
    const mounts = app.match(/<WorkspaceSwitcher\s*\/>/g) ?? [];

    expect(mounts).toHaveLength(1);
    expectOrderedIn(app, "<WorkspaceSwitcher />", "<TabBar router=");
    expect(tabBar).not.toContain("WorkspaceSwitcher");
  });
});
