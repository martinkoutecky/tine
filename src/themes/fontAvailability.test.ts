import { describe, expect, it } from "vitest";
import { fontPresence, type TextMeasurer } from "./fontAvailability";

// A fake renderer: a family "has" a face when it is in `installed`; a face has its
// own advance width, the generics another. The probe only sees the widths.
function measurer(installed: Record<string, number>, calls: string[] = []): TextMeasurer {
  return (font) => {
    calls.push(font);
    const named = /^72px "([^"]+)", (monospace|serif|sans-serif)$/.exec(font);
    if (named && installed[named[1]] !== undefined) return installed[named[1]];
    const generic = /(monospace|serif|sans-serif)$/.exec(font)![1];
    return { monospace: 600, serif: 520, "sans-serif": 500 }[generic]!;
  };
}

describe("font availability probe (GH #760)", () => {
  it("reports a font whose width differs from the generics as installed", () => {
    expect(fontPresence("IBM Plex Sans", measurer({ "IBM Plex Sans": 540 }))).toBe("installed");
  });
  it("reports a name that renders exactly like every generic as missing", () => {
    expect(fontPresence("No Such Font", measurer({}))).toBe("missing");
  });
  it("finds a font that happens to match one generic by trying the others", () => {
    expect(fontPresence("Looks Like Mono", measurer({ "Looks Like Mono": 600 }))).toBe("installed");
  });
  it("answers unknown, never missing, when the environment cannot measure", () => {
    expect(fontPresence("Anything", () => null)).toBe("unknown");
    expect(fontPresence("Anything", () => 0)).toBe("unknown");
  });
  it("measures at most three generics per family", () => {
    const calls: string[] = [];
    fontPresence("Nope", measurer({}, calls));
    expect(calls.length).toBeLessThanOrEqual(6);
  });
  it("is unknown under jsdom, which has no canvas, so no hint is invented", () => {
    expect(fontPresence("IBM Plex Sans")).toBe("unknown");
  });
});
