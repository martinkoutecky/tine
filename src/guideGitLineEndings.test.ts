import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("onboards a converted checkout by saving edits before disabling conversion and replacing files", () => {
  const guide = readFileSync("crates/tine-core/src/templates/files-external-edits-backups.md", "utf8");
  const section = guide.split("## Git line endings on Windows")[1].split("## Git: what changed")[0];
  const steps = ["git status --short", "git add --all", 'git commit -m "Save notes before changing line endings"', "git status --porcelain", "git config --local core.autocrlf false", "git reset --hard HEAD"];
  let at = -1;
  for (const step of steps) {
    const next = section.indexOf(step);
    expect(next).toBeGreaterThan(at);
    at = next;
  }
  expect(section).toContain("continue only if it prints nothing");
  expect(section).toContain("close Tine and other editors");
  expect(section).toContain("never global or system settings");
  expect(section).toContain("e7ad234c-701e-4473-a6e3-111264be9655");
});
