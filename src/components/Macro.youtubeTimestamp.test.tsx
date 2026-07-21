import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { VideoMacro, YoutubeTimestamp } from "./Macro";

interface MockPlayer {
  seekTo: ReturnType<typeof vi.fn>;
  getCurrentTime: ReturnType<typeof vi.fn>;
  destroy: ReturnType<typeof vi.fn>;
}

function installYoutube(player: MockPlayer) {
  const Player = vi.fn(function Player(_iframeId: string) {
    return player;
  });
  Object.assign(window as Window & { YT?: { Player: typeof Player } }, { YT: { Player } });
  return Player;
}

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

afterEach(() => {
  delete (window as Window & { YT?: unknown }).YT;
  document.body.innerHTML = "";
});

describe("YouTube timestamp macros", () => {
  it("renders a clickable timestamp that seeks the later YouTube player", async () => {
    const player: MockPlayer = {
      seekTo: vi.fn(),
      getCurrentTime: vi.fn(() => 0),
      destroy: vi.fn(),
    };
    const Player = installYoutube(player);
    const { root, dispose } = mount(() => (
      <>
        <VideoMacro body="video https://www.youtube.com/watch?v=dQw4w9WgXcQ" />
        <YoutubeTimestamp body="youtube-timestamp 125" />
      </>
    ));

    try {
      await expect.poll(() => Player.mock.calls.length).toBe(1);
      const timestamp = root.querySelector<HTMLElement>(".youtube-ts");
      expect(timestamp?.tagName).toBe("A");
      timestamp!.click();
      expect(player.seekTo).toHaveBeenCalledWith(125, true);
    } finally {
      dispose();
    }
  });

  it("registers a mounted embed and removes its player when it unmounts", async () => {
    const player: MockPlayer = {
      seekTo: vi.fn(),
      getCurrentTime: vi.fn(() => 0),
      destroy: vi.fn(),
    };
    const Player = installYoutube(player);
    const video = mount(() => <VideoMacro body="youtube dQw4w9WgXcQ" />);

    await expect.poll(() => Player.mock.calls.length).toBe(1);
    video.dispose();
    expect(player.destroy).toHaveBeenCalledTimes(1);

    // Recreate the retired iframe id ahead of the timestamp. A stale registry
    // entry would make this label seek the unmounted player.
    const retiredIframe = document.createElement("iframe");
    retiredIframe.id = Player.mock.calls[0]?.[0] ?? "";
    retiredIframe.src = "https://www.youtube.com/embed/dQw4w9WgXcQ?enablejsapi=1";
    document.body.appendChild(retiredIframe);

    const timestamp = mount(() => <YoutubeTimestamp body="youtube-timestamp 8" />);
    try {
      timestamp.root.querySelector<HTMLElement>(".youtube-ts")!.click();
      expect(player.seekTo).not.toHaveBeenCalled();
    } finally {
      timestamp.dispose();
    }
  });

  it("keeps the formatted label when no player is available", () => {
    const { root, dispose } = mount(() => <YoutubeTimestamp body="youtube-timestamp 3661" />);
    try {
      expect(root.querySelector(".youtube-ts")?.textContent).toContain("1:01:01");
    } finally {
      dispose();
    }
  });

  it("keeps the YouTube iframe URL and playback attributes", () => {
    const player: MockPlayer = {
      seekTo: vi.fn(),
      getCurrentTime: vi.fn(() => 0),
      destroy: vi.fn(),
    };
    installYoutube(player);
    const { root, dispose } = mount(() => <VideoMacro body="video https://www.youtube.com/watch?v=dQw4w9WgXcQ" />);
    try {
      const iframe = root.querySelector("iframe");
      expect(iframe?.getAttribute("src")).toBe("https://www.youtube.com/embed/dQw4w9WgXcQ?enablejsapi=1");
      expect(iframe?.getAttribute("allow")).toBe("accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share");
      expect(iframe?.getAttribute("referrerpolicy")).toBe("strict-origin-when-cross-origin");
      expect(iframe?.hasAttribute("allowfullscreen")).toBe(true);
    } finally {
      dispose();
    }
  });
});
