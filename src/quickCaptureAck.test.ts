import { describe, expect, it } from "vitest";
import { quickCaptureAckMatches, shouldRetryQuickCapture } from "./quickCaptureAck";

describe("quick-capture ack helpers", () => {
  it("matches only the pending request id", () => {
    expect(quickCaptureAckMatches("a", { id: "a", ok: true })).toBe(true);
    expect(quickCaptureAckMatches("a", { id: "b", ok: true })).toBe(false);
    expect(quickCaptureAckMatches(null, { id: "a", ok: true })).toBe(false);
  });

  it("allows the initial emit plus two retries", () => {
    expect(shouldRetryQuickCapture(1)).toBe(true);
    expect(shouldRetryQuickCapture(2)).toBe(true);
    expect(shouldRetryQuickCapture(3)).toBe(false);
  });
});
