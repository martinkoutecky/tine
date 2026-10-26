import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { VideoMacro, YoutubeTimestamp, youtubeTimestampMacroFor } from "./Macro";

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
  // Intentional OG divergence: same-video and playback history cross pane boundaries.
  it("prefers the played same-id sidebar copy over a nearer different video", async () => {
    const players = new Map<string, MockPlayer>();
    const events = new Map<string, (event: { data: number }) => void>();
    const Player = vi.fn(function Player(id: string, options: { events: { onStateChange: (event: { data: number }) => void } }) {
      const player = { seekTo: vi.fn(), getCurrentTime: vi.fn(() => 42.9), destroy: vi.fn() };
      players.set(id, player);
      events.set(id, options.events.onStateChange);
      return player;
    });
    Object.assign(window, { YT: { Player } });
    const { root, dispose } = mount(() => <>
      <div data-pane-id="main"><VideoMacro body="youtube dQw4w9WgXcQ" />
        <div data-pane-id="other"><VideoMacro body="youtube JI-AyLv68Xs" /></div>
        <YoutubeTimestamp body="youtube-timestamp 125" />
      </div>
      <div class="rs-item"><VideoMacro body="youtube dQw4w9WgXcQ" /></div>
    </>);
    try {
      await expect.poll(() => players.size).toBe(3);
      const frames = root.querySelectorAll("iframe");
      const stamp = root.querySelector<HTMLElement>(".youtube-ts")!;
      events.get(frames[2].id)?.({ data: 1 });
      events.get(frames[1].id)?.({ data: 1 });
      stamp.click();
      expect(players.get(frames[2].id)!.seekTo).toHaveBeenCalledWith(125, true);
      expect(players.get(frames[1].id)!.seekTo).not.toHaveBeenCalled();
      expect(youtubeTimestampMacroFor(stamp)).toBe("{{youtube-timestamp 42}}");
    } finally { dispose(); }
  });

  it("uses the last played sidebar player without a video association and forgets unmounted history", async () => {
    const players: MockPlayer[] = [];
    const events: ((event: { data: number }) => void)[] = [];
    Object.assign(window, { YT: { Player: function (_id: string, options: { events: { onStateChange: (event: { data: number }) => void } }) {
      const player = { seekTo: vi.fn(), getCurrentTime: vi.fn(() => 17), destroy: vi.fn() };
      players.push(player); events.push(options.events.onStateChange); return player;
    } } });
    const stamp = mount(() => <div data-pane-id="main"><YoutubeTimestamp body="youtube-timestamp 8" /></div>);
    const video = mount(() => <div class="rs-item"><VideoMacro body="youtube dQw4w9WgXcQ" /><VideoMacro body="youtube JI-AyLv68Xs" /></div>);
    try {
      await expect.poll(() => players.length).toBe(2);
      events[1]?.({ data: 1 }); events[0]?.({ data: 1 }); events[1]?.({ data: 2 });
      stamp.root.querySelector<HTMLElement>(".youtube-ts")!.click();
      expect(players[0].seekTo).toHaveBeenCalledWith(8, true);
      expect(players[1].seekTo).not.toHaveBeenCalled();
      video.dispose();
      expect(youtubeTimestampMacroFor(stamp.root.querySelector(".youtube-ts")!)).toBeNull();
    } finally { stamp.dispose(); }
  });

  it("retains OG's nearest preceding player with no history or own-pane association", async () => {
    const players: MockPlayer[] = [];
    Object.assign(window, { YT: { Player: function () {
      const player = { seekTo: vi.fn(), getCurrentTime: vi.fn(() => 0), destroy: vi.fn() };
      players.push(player); return player;
    } } });
    const { root, dispose } = mount(() => <>
      <div class="rs-item"><VideoMacro body="youtube dQw4w9WgXcQ" /><VideoMacro body="youtube JI-AyLv68Xs" /></div>
      <div data-pane-id="main"><YoutubeTimestamp body="youtube-timestamp 9" /></div>
    </>);
    try {
      await expect.poll(() => players.length).toBe(2);
      root.querySelector<HTMLElement>(".youtube-ts")!.click();
      expect(players[1].seekTo).toHaveBeenCalledWith(9, true);
      expect(players[0].seekTo).not.toHaveBeenCalled();
    } finally { dispose(); }
  });
  it.each(["video https://youtu.be/JI-AyLv68Xs", "youtube JI-AyLv68Xs"])("renders the GH #600 video through %s", (body) => {
    const { root, dispose } = mount(() => <VideoMacro body={body} />);
    try {
      const iframe = root.querySelector("iframe")!;
      expect(iframe.src).toBe("https://www.youtube.com/embed/JI-AyLv68Xs?enablejsapi=1");
      expect(iframe.getAttribute("referrerpolicy")).toBe("strict-origin-when-cross-origin");
    } finally { dispose(); }
  });

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
