// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { enterFocusMode, exitFocusMode, focusMode } from "./ui";

const windowMock = vi.hoisted(() => ({ isFullscreen: vi.fn(), setFullscreen: vi.fn() }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => windowMock }));

afterEach(async () => {
  if (focusMode()) await exitFocusMode();
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  vi.clearAllMocks();
});

it("does not enter native fullscreen after focus mode was exited", async () => {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  let finish!: (fullscreen: boolean) => void;
  windowMock.isFullscreen.mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
  windowMock.setFullscreen.mockResolvedValue(undefined);
  const entering = enterFocusMode();
  await vi.waitFor(() => expect(windowMock.isFullscreen).toHaveBeenCalledOnce());
  const exiting = exitFocusMode();
  finish(false);
  await Promise.all([entering, exiting]);
  expect(focusMode()).toBe(false);
  expect(windowMock.setFullscreen).not.toHaveBeenCalledWith(true);
});
