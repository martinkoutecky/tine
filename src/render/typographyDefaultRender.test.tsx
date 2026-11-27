import { afterEach, beforeAll, expect, it } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "./parse";
import { InlineText } from "./inline";
import { setTypographyModeSig, typographyMode } from "../ui";

// a user who never touched Settings sees glyphs; "off" shows the arrows as typed (GH #583).
beforeAll(async () => { await initParser(); });
const initial = typographyMode();
afterEach(() => { setTypographyModeSig(initial); document.body.innerHTML = ""; });

function visibleText(text: string): string {
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <InlineText text={text} />, host);
  const out = host.textContent ?? "";
  dispose();
  return out;
}

it("renders -> <- => and dashes as glyphs by default", () => {
  expect(initial).toBe("render");
  expect(visibleText("a -> b <- c => d -- e --- f")).toBe("a → b ← c ⇒ d – e — f");
});

it("an explicit opt-out shows them as typed", () => {
  setTypographyModeSig("off");
  expect(visibleText("a -> b <- c => d -- e --- f")).toBe("a -> b <- c => d -- e --- f");
});
