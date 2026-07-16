import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { audioPlayer, setAudioPlayer } from "../ui";
import { AudioOverlay } from "./AudioOverlay";
import { installKeybindings } from "../keybindings";
import { clearTransientLayersForTest, dismissTopTransient, registerTransientLayer } from "../transientLayers";

afterEach(() => {
  clearTransientLayersForTest();
  setAudioPlayer(null);
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("AudioOverlay resource lifecycle", () => {
  it("uses the streaming scrubber without fetching and decoding the whole track", async () => {
    vi.spyOn(backend(), "streamAsset").mockResolvedValue("asset://long.mp3");
    const fetchWholeTrack = vi.fn();
    vi.stubGlobal("fetch", fetchWholeTrack);
    vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => {});

    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <AudioOverlay />, host);
    setAudioPlayer({ url: "../assets/long.mp3", name: "Long recording" });
    await vi.waitFor(() => expect(host.querySelector("audio")).not.toBeNull());
    await Promise.resolve();

    expect(fetchWholeTrack).not.toHaveBeenCalled();
    dispose();
    host.remove();
  });

  it("cancels a queued Blob fallback and releases it when the panel closes", async () => {
    vi.spyOn(backend(), "streamAsset").mockResolvedValue("asset://broken.mp3");
    let resolveRead!: (bytes: Uint8Array) => void;
    const read = vi.spyOn(backend(), "readAsset").mockReturnValue(
      new Promise<Uint8Array>((resolve) => { resolveRead = resolve; })
    );
    const createObjectURL = vi.fn(() => "blob:late-audio");
    const revokeObjectURL = vi.fn();
    vi.stubGlobal("URL", { createObjectURL, revokeObjectURL });
    vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => {});

    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <AudioOverlay />, host);
    setAudioPlayer({ url: "../assets/broken.mp3", name: "Broken recording" });
    await vi.waitFor(() => expect(host.querySelector("audio")).not.toBeNull());
    host.querySelector("audio")!.dispatchEvent(new Event("error"));
    await vi.waitFor(() => expect(read).toHaveBeenCalledTimes(1));
    expect(read).toHaveBeenCalledWith("broken.mp3", 64 * 1024 * 1024);
    expect(createObjectURL).not.toHaveBeenCalled();

    (host.querySelector(".audio-close") as HTMLButtonElement).click();
    expect(audioPlayer()).toBeNull();
    expect(createObjectURL).not.toHaveBeenCalled();
    resolveRead(new Uint8Array([1, 2, 3]));
    await Promise.resolve();
    await Promise.resolve();
    expect(createObjectURL).not.toHaveBeenCalled();
    expect(revokeObjectURL).not.toHaveBeenCalled();
    dispose();
    host.remove();
  });

  it("owns one Escape/Back rung above a lower transient only while expanded", async () => {
    vi.spyOn(backend(), "streamAsset").mockResolvedValue("asset://owned.mp3");
    vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => {});
    const host = document.createElement("div");
    document.body.append(host);
    const dispose = render(() => <AudioOverlay />, host);
    const uninstall = installKeybindings();
    const lowerRoot = document.createElement("button");
    document.body.append(lowerRoot);
    let lowerDismissals = 0;
    const unregisterLower = registerTransientLayer({
      id: "audio-lower",
      root: () => lowerRoot,
      dismiss: () => { lowerDismissals += 1; return true; },
    });
    try {
      setAudioPlayer({ url: "../assets/owned.mp3", name: "Owned recording" });
      await vi.waitFor(() => expect(host.querySelector(".audio-overlay")).not.toBeNull());
      const event = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
      host.querySelector(".audio-overlay")!.dispatchEvent(event);
      await Promise.resolve();
      expect(event.defaultPrevented).toBe(true);
      expect(audioPlayer()).toBeNull();
      expect(lowerDismissals).toBe(0);

      setAudioPlayer({ url: "../assets/owned.mp3", name: "Owned recording" });
      await vi.waitFor(() => expect(host.querySelector(".audio-overlay")).not.toBeNull());
      expect(dismissTopTransient("back")).toBe(true);
      await Promise.resolve();
      expect(audioPlayer()).toBeNull();
      expect(lowerDismissals).toBe(0);

      expect(dismissTopTransient("escape")).toBe(true);
      expect(lowerDismissals).toBe(1);
    } finally {
      unregisterLower();
      uninstall();
      dispose();
      host.remove();
    }
  });
});
