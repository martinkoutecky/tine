import { describe, it, expect, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";

// GH #36: the frameless main window's title-bar close button is covered by the
// full-cover Welcome overlay, so the welcome screen must draw its own window
// controls whenever the OS isn't drawing a frame. Mock the Tauri-only bits so
// Welcome renders in jsdom; WindowControls is stubbed (the real one calls
// getCurrentWindow()). osDrawsWindowControls is togglable via a hoisted mock.
const { drawsMock } = vi.hoisted(() => ({ drawsMock: vi.fn(() => false) }));
vi.mock("../backend", () => ({ isTauri: () => true }));
vi.mock("./WindowChrome", () => ({ WindowControls: () => null }));
vi.mock("../nativeChrome", () => ({ osDrawsWindowControls: drawsMock }));
vi.mock("../graph", () => ({ switchGraph: async () => {}, createNewGraph: async () => {} }));

import { Welcome } from "./Welcome";

function html(node: () => JSX.Element): string {
  const div = document.createElement("div");
  const dispose = render(() => node(), div);
  const out = div.innerHTML;
  dispose();
  return out;
}

describe("Welcome window chrome (GH #36)", () => {
  it("draws window controls on a frameless window (isTauri && !osDrawsWindowControls)", () => {
    drawsMock.mockReturnValue(false);
    expect(html(() => <Welcome />)).toContain("welcome-winchrome");
  });

  it("omits them when the OS draws the frame (macOS / native frame / mobile)", () => {
    drawsMock.mockReturnValue(true);
    expect(html(() => <Welcome />)).not.toContain("welcome-winchrome");
  });
});
