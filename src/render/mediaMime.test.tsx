import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import * as wasm from "./wasm/lsdoc_wasm";
import { AstBody } from "./body";
import { AudioOverlay } from "../components/AudioOverlay";
import { setAudioPlayer } from "../ui";

afterEach(() => { setAudioPlayer(null); document.body.innerHTML = ""; vi.restoreAllMocks(); vi.unstubAllGlobals(); });

it.each(["inline-audio", "inline-video", "overlay"])("%s fallback uses the shared wasm MIME answer", async surface => {
  const blobs: Blob[] = [];
  vi.stubGlobal("URL", { createObjectURL: (blob: Blob) => { blobs.push(blob); return "blob:fallback"; }, revokeObjectURL: vi.fn() });
  const mime = vi.spyOn(wasm, "mime_from_path").mockReturnValue("fixture/shared-answer");
  vi.spyOn(backend(), "streamAsset").mockResolvedValue("asset://stream");
  vi.spyOn(backend(), "readAsset").mockResolvedValue(new Uint8Array([1]));
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
  vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => {});
  const root = document.createElement("div"); document.body.append(root);
  const ext = surface === "inline-video" ? "mkv" : "mp3";
  const url = `../assets/nested/track.${ext}`;
  if (surface === "overlay") setAudioPlayer({ url, name: "Track" });
  const dispose = render(() => surface === "overlay" ? <AudioOverlay /> : <AstBody raw={`![Track](${url})`} />, root);
  try {
    await vi.waitFor(() => expect(root.querySelector("audio, video")?.getAttribute("src")).toBe("asset://stream"));
    root.querySelector("audio, video")!.dispatchEvent(new Event("error"));
    await vi.waitFor(() => expect(blobs).toHaveLength(1));
    expect(blobs[0].type).toBe("fixture/shared-answer");
    expect(mime).toHaveBeenCalledWith(`nested/track.${ext}`);
  } finally { dispose(); }
});
