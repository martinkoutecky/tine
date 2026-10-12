import { afterEach, beforeAll, expect, it } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "./parse";
import { InlineText } from "./inline";
import { setTypographyModeSig } from "../ui";

// GH #583: a user who never touched Settings sees the arrows they typed.
beforeAll(async () => { await initParser(); });
afterEach(() => { setTypographyModeSig("off"); document.body.innerHTML = ""; });

function visibleText(text: string): string {
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <InlineText text={text} />, host);
  const out = host.textContent ?? "";
  dispose();
  return out;
}

it("renders -> <- => and dashes as typed by default", () => {
  expect(visibleText("a -> b <- c => d -- e --- f")).toBe("a -> b <- c => d -- e --- f");
});

it("an explicit opt-in still shows glyphs", () => {
  setTypographyModeSig("render");
  const out = visibleText("a -> b");
  expect(out).toContain("→");
  expect(out).not.toContain("->");
});
