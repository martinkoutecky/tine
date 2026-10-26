import fs from "node:fs";
import { afterEach, describe, expect, it } from "vitest";
import { commandDefaults } from "../keybindings";
import { setActiveLocale } from "./index";

afterEach(() => setActiveLocale("en"));

// tine#107, D4 and review #10: a built-in command's label is the catalog message
// named after its id (`/` → `-`), so the mapping is total and collision-free; the
// palette and Settings show it in the UI language and search its English too.
describe("command labels", () => {
  it("are the message named after each command's id", () => {
    const source = fs.readFileSync("src/keybindings.ts", "utf8");
    const table = source.slice(source.indexOf("const COMMANDS: CommandDef[] = ["));
    const pairs = [...table.matchAll(/\bid:\s*"([^"]+)",[^{}]*?label:\s*msg\("([^"]+)"/g)];
    expect(pairs.length).toBeGreaterThan(80);
    for (const [, id, message] of pairs) expect(message, id).toBe(`command-${id.replaceAll("/", "-")}`);
    const builtIn = commandDefaults().filter((command) => !command.id.startsWith("plugin:"));
    const named = builtIn.map((command) => command.id.replaceAll("/", "-"));
    expect(new Set(named).size).toBe(named.length);
  });

  it("are shown in the active language and keep their English for search", async () => {
    await setActiveLocale("zh-CN");
    const rows = commandDefaults();
    const search = rows.find((command) => command.id === "go/search")!;
    expect(search.english).toBe("Search / quick switch");
    expect(search.label).toBe("搜索 / 快速切换");
    expect(rows.find((command) => command.id === "pane/focus-3")!.label).toBe("聚焦窗格 3");
  });
});
