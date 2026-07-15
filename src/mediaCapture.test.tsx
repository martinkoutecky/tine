import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  DESKTOP_RECORDING_MAX_BYTES,
  cancelDesktopVoiceRecording,
  desktopVoiceRecordingActive,
  startDesktopVoiceRecording,
} from "./mediaCapture";

class FakeRecorder {
  static instances: FakeRecorder[] = [];
  state: RecordingState = "inactive";
  mimeType = "audio/webm";
  ondataavailable: ((event: BlobEvent) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  onstop: ((event: Event) => void) | null = null;

  constructor(_stream: MediaStream) {
    FakeRecorder.instances.push(this);
  }

  start(): void {
    this.state = "recording";
  }

  stop(): void {
    if (this.state === "inactive") throw new Error("already stopped");
    this.state = "inactive";
    queueMicrotask(() => this.onstop?.(new Event("stop")));
  }

  emitClaimedBytes(size: number): void {
    const data = new Blob([new Uint8Array([1])], { type: this.mimeType });
    Object.defineProperty(data, "size", { value: size });
    this.ondataavailable?.({ data } as BlobEvent);
  }
}

describe("desktop voice-recording ownership and bounds", () => {
  let trackStopped = false;
  let getUserMedia: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    trackStopped = false;
    FakeRecorder.instances = [];
    getUserMedia = vi.fn(async () => ({
      getTracks: () => [{ stop: () => { trackStopped = true; } }],
    } as unknown as MediaStream));
    Object.defineProperty(navigator, "mediaDevices", {
      configurable: true,
      value: { getUserMedia },
    });
    vi.stubGlobal("MediaRecorder", FakeRecorder);
  });

  afterEach(() => {
    cancelDesktopVoiceRecording();
    vi.unstubAllGlobals();
  });

  it("reserves one physical recorder and releases it when its editor unmounts", async () => {
    const callbacks = { complete: vi.fn(), error: vi.fn() };
    const owner = Symbol("owner");
    expect(await startDesktopVoiceRecording(owner, callbacks)).toBe("started");
    expect(await startDesktopVoiceRecording(Symbol("other"), callbacks)).toBe("busy");
    expect(getUserMedia).toHaveBeenCalledTimes(1);
    expect(FakeRecorder.instances).toHaveLength(1);

    expect(cancelDesktopVoiceRecording(owner)).toBe(true);
    expect(trackStopped).toBe(true);
    expect(desktopVoiceRecordingActive()).toBe(false);
  });

  it("stops before retaining a chunk that would exceed the encoded-byte cap", async () => {
    const complete = vi.fn();
    const error = vi.fn();
    expect(await startDesktopVoiceRecording(Symbol("bounded"), { complete, error })).toBe("started");
    const recorder = FakeRecorder.instances[0];
    recorder.emitClaimedBytes(DESKTOP_RECORDING_MAX_BYTES - 1);
    recorder.emitClaimedBytes(2);

    await vi.waitFor(() => expect(complete).toHaveBeenCalledTimes(1));
    expect(complete.mock.calls[0][0].byteLength).toBeLessThanOrEqual(DESKTOP_RECORDING_MAX_BYTES);
    expect(complete.mock.calls[0][2]).toBe(true);
    expect(error).not.toHaveBeenCalled();
    expect(trackStopped).toBe(true);
    expect(desktopVoiceRecordingActive()).toBe(false);
  });
});
