import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { AstBody } from "./body";
import { backend } from "../backend";
import { audioPlayer, setAudioPlayer } from "../ui";

afterEach(() => { vi.restoreAllMocks(); setAudioPlayer(null); document.body.innerHTML = ""; });

it.each(["100%", "voice%20memo", "a".repeat(500)])("renders media name %s and opens its expanded player (OG-B-FRONT)", async name => {
  vi.spyOn(backend(), "streamAsset").mockResolvedValue("stream://fixture");
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <AstBody raw={`![voice](../assets/${name}.mp3)`} />, root);
  try {
    await vi.waitFor(() => expect(root.querySelector(".media-audio-widen")).not.toBeNull());
    root.querySelector<HTMLButtonElement>(".media-audio-widen")!.click();
    expect(audioPlayer()?.name).toBe(name === "voice%20memo" ? "voice memo.mp3" : `${name}.mp3`);
  } finally { dispose(); }
});
