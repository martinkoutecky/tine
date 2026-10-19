import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { installPlatformAttribute } from "./nativeChrome";
import { installTouchMedia, setTouchInput } from "./tests/touchMediaTestkit";

// Review r1 of D-20261018-W1: `<html data-touch-gestures>` (the CSS hook for
// touch-action on rows and swipe feedback) was stamped once at startup, so a
// touch input that appeared or went away afterwards left the CSS and the swipe
// wiring disagreeing. The stamp follows the capability.
const stamp = () => document.documentElement.getAttribute("data-touch-gestures");

beforeAll(() => installTouchMedia());
afterEach(() => {
  setTouchInput(false);
  document.documentElement.removeAttribute("data-touch-gestures");
  delete globalThis.__TINE_E2E_TOUCH_GESTURES__;
});

describe("the touch-gesture stamp on <html>", () => {
  it("appears when a touch input appears after startup and goes when it goes", () => {
    const stop = installPlatformAttribute();
    try {
      expect(stamp()).toBeNull();
      setTouchInput(false, true);
      expect(stamp()).toBe("touch");
      setTouchInput(false);
      expect(stamp()).toBeNull();
      setTouchInput(true);
      expect(stamp()).toBe("touch");
    } finally { stop(); }
  });

  it("is there at startup on a touch device and does not stack up across flips", () => {
    setTouchInput(true);
    const stop = installPlatformAttribute();
    try {
      expect(stamp()).toBe("touch");
      for (let i = 0; i < 3; i++) {
        setTouchInput(false);
        expect(stamp()).toBeNull();
        setTouchInput(true);
        expect(stamp()).toBe("touch");
      }
    } finally { stop(); }
  });

  it("stops following once disposed", () => {
    const stop = installPlatformAttribute();
    stop();
    setTouchInput(true);
    expect(stamp()).toBeNull();
  });

  it("keeps the E2E-forced stamp on a desktop process with no touch input", () => {
    globalThis.__TINE_E2E_TOUCH_GESTURES__ = "android";
    const stop = installPlatformAttribute();
    try {
      expect(stamp()).toBe("android");
      setTouchInput(true);
      setTouchInput(false);
      expect(stamp()).toBe("android");
    } finally { stop(); }
  });
});
