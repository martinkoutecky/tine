// The media and small display macros ({{video}}/{{youtube}}/{{vimeo}}/
// {{bilibili}}, {{tweet}}, {{youtube-timestamp}}, {{cloze}}, {{zotero-…}}),
// split from Macro.tsx along the query/media seam.
import { createSignal, createUniqueId, onCleanup, onMount, Show, type JSX } from "solid-js";
import { msg, t, render as renderMessage } from "../i18n";
import { youtubePlayerForTarget, registerYoutubePlayer, recordYoutubePlayback, unregisterYoutubePlayer, type YoutubePlayer } from "../youtubePlayer";
import { ExternalLink } from "./ExternalLink";
import { mainWindow } from "../windowRealm";

interface YoutubeApi {
  Player: new (iframeId: string, options: { events?: { onReady?: () => void; onStateChange?: (event: { data: number }) => void } }) => YoutubePlayer;
}

type YoutubeWindow = Window & {
  YT?: YoutubeApi;
  onYouTubeIframeAPIReady?: () => void;
};

let youtubeApiLoading: Promise<YoutubeApi | null> | null = null;
const YOUTUBE_API_SCRIPT_ID = "tine-youtube-iframe-api";

// The API is intentionally fetched only from a mounted YouTube embed. OG does
// the same mount-time load/register sequence (og-1.0.0 6e7afa8eb,
// extensions/video/youtube.cljs:20-27, :45-53).
function loadYoutubeApi(): Promise<YoutubeApi | null> {
  if (typeof window === "undefined" || typeof document === "undefined") return Promise.resolve(null);
  const ytWindow = window as YoutubeWindow;
  if (ytWindow.YT?.Player) return Promise.resolve(ytWindow.YT);
  if (youtubeApiLoading) return youtubeApiLoading;

  youtubeApiLoading = new Promise((resolve) => {
    let settled = false;
    const settle = (api: YoutubeApi | undefined) => {
      if (settled) return;
      settled = true;
      resolve(api?.Player ? api : null);
    };
    const priorReady = ytWindow.onYouTubeIframeAPIReady;
    ytWindow.onYouTubeIframeAPIReady = () => {
      try {
        priorReady?.();
      } finally {
        settle(ytWindow.YT);
      }
    };
    // The IFrame API is one process-wide script in the main realm's <head>.
    let script = mainWindow.document.getElementById(YOUTUBE_API_SCRIPT_ID) as HTMLScriptElement | null;
    if (!script) {
      script = document.createElement("script");
      script.id = YOUTUBE_API_SCRIPT_ID;
      script.async = true;
      script.src = "https://www.youtube.com/iframe_api";
      document.head.appendChild(script);
    }
    script.addEventListener("error", () => settle(undefined), { once: true });
  });
  return youtubeApiLoading;
}


// The OG generator floors getCurrentTime before it formats the macro; with no
// registered/ready player it produces NOTHING — the command is a no-op, no
// macro is inserted (og-1.0.0 6e7afa8eb, extensions/video/youtube.cljs:113-122).
export function youtubeTimestampMacroFor(target: Node): string | null {
  const seconds = youtubePlayerForTarget(target)?.getCurrentTime();
  if (typeof seconds !== "number" || !Number.isFinite(seconds)) return null;
  return `{{youtube-timestamp ${Math.max(0, Math.floor(seconds))}}}`;
}

function httpUrl(value: string): string | undefined {
  if (!/^https?:\/\//i.test(value)) return undefined;
  try {
    const url = new URL(value);
    return (url.protocol === "http:" || url.protocol === "https:") && url.hostname ? value : undefined;
  } catch {
    return undefined;
  }
}

// A {{video}} / {{youtube}} / {{vimeo}} / {{bilibili}} macro: embeds YouTube,
// Vimeo or Bilibili as an iframe and direct media files as a <video>. Each of the
// provider-named macros also accepts a bare id (e.g. `{{vimeo 12345}}`), matching
// OG; the generic `{{video URL}}` sniffs the provider from the URL. Falls back to a
// link. (`youtube-timestamp` is a SEPARATE macro — handled before this one.)
export function VideoMacro(props: { body: string }): JSX.Element {
  const iframeId = `youtube-player-${createUniqueId()}`;
  const parsed = () => {
    const m = /^(\w+)\s*([\s\S]*)$/.exec(props.body.trim());
    const name = (m?.[1] ?? "video").toLowerCase();
    const arg = (m?.[2] ?? "").trim().replace(/^\[\[|\]\]$/g, "");
    return { name, arg };
  };
  const url = () => parsed().arg;
  const safeUrl = () => httpUrl(url());
  const embed = () => {
    const { name, arg } = parsed();
    // `?enablejsapi=1` matches OG (youtube.cljs:58) and enables timestamps.
    const yt = safeUrl() && /(?:youtube\.com\/(?:watch\?v=|embed\/)|youtu\.be\/)([\w-]{11})/.exec(arg);
    if (yt) return `https://www.youtube.com/embed/${yt[1]}?enablejsapi=1`;
    if (name === "youtube" && /^[\w-]{11}$/.test(arg)) return `https://www.youtube.com/embed/${arg}?enablejsapi=1`;
    const vimeo = safeUrl() && /vimeo\.com\/(\d+)/.exec(arg);
    if (vimeo) return `https://player.vimeo.com/video/${vimeo[1]}`;
    if (name === "vimeo" && /^\d+$/.test(arg)) return `https://player.vimeo.com/video/${arg}`;
    const bili = safeUrl() && /bilibili\.com\/video\/(BV[0-9A-Za-z]+)/i.exec(arg);
    const bvid = bili ? bili[1] : name === "bilibili" && /^BV[0-9A-Za-z]+$/.test(arg) ? arg : null;
    if (bvid) return `https://player.bilibili.com/player.html?bvid=${bvid}&high_quality=1`;
    return null;
  };
  // OG parity (og-1.0.0 6e7afa8eb): the embed iframe's `allow`/`referrerpolicy`.
  // YouTube (youtube.cljs:54-70) preserves an HTTP(S) parent's origin referrer.
  // A custom-protocol desktop parent still needs native WebView identification
  // (GH #600); this policy cannot manufacture a missing HTTP Referer header.
  // https://developers.google.com/youtube/terms/required-minimum-functionality
  // Vimeo (block.cljs:1290-1305) gets the same `allow` list
  // minus picture-in-picture/web-share and NO referrerpolicy; bilibili sets
  // neither (the `.embed-iframe` class already removes the border).
  const embedAttrs = (): Record<string, string> => {
    const src = embed() ?? "";
    if (/(?:^|\/\/)(?:www\.)?(?:youtube\.com|youtube-nocookie\.com)\/embed\//.test(src))
      return {
        allow: "accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share",
        referrerpolicy: "strict-origin-when-cross-origin",
      };
    if (src.includes("player.vimeo.com"))
      return { allow: "accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope" };
    return {};
  };
  const isYoutubeEmbed = () => /(?:^|\/\/)(?:www\.)?(?:youtube\.com|youtube-nocookie\.com)\/embed\//.test(embed() ?? "");
  let player: YoutubePlayer | undefined;
  onMount(() => {
    if (!isYoutubeEmbed()) return;
    let mounted = true;
    void loadYoutubeApi().then((api) => {
      if (!mounted || !api) return;
      try {
        const registered = new api.Player(iframeId, { events: {
          onReady: () => undefined,
          onStateChange: (event) => {
            if (mounted && event.data === 1) recordYoutubePlayback(iframeId);
          },
        } });
        if (!mounted) {
          registered.destroy?.();
          return;
        }
        player = registered;
        registerYoutubePlayer(iframeId, registered);
      } catch {
        // Offline, blocked, or malformed API responses leave the timestamp label usable.
      }
    });
    onCleanup(() => {
      mounted = false;
      unregisterYoutubePlayer(iframeId, player);
      player?.destroy?.();
    });
  });
  return (
    <Show
      when={embed()}
      fallback={
        <Show
          when={safeUrl() && /\.(mp4|webm|ogg)(\?|$)/i.test(url())}
          fallback={safeUrl()
            ? <ExternalLink dest={safeUrl()!}>{url()}</ExternalLink>
            : <span>{url()}</span>}
        >
          <video class="embed-video" src={safeUrl()} controls />
        </Show>
      }
    >
      <div class="embed-iframe-wrap">
        <iframe id={isYoutubeEmbed() ? iframeId : undefined} class="embed-iframe" src={embed()!} allowfullscreen title={t("macro-macro-video")} {...embedAttrs()} />
      </div>
    </Show>
  );
}

// A {{tweet URL}} / {{twitter URL}} macro (`twitter` is OG's alias for `tweet`) —
// rendered as a link (no third-party script embedding).
export function TweetMacro(props: { body: string }): JSX.Element {
  const url = () => props.body.replace(/^(tweet|twitter)\s*/i, "").trim();
  const safeUrl = () => httpUrl(url());
  return (
    <Show when={safeUrl()} fallback={<span>🐦 {url()}</span>}>
      <ExternalLink class="external-link tweet-link" dest={safeUrl()!}>🐦 {url()}</ExternalLink>
    </Show>
  );
}

// `{{youtube-timestamp <seconds>}}` seeks the shared selected YouTube player.
export function YoutubeTimestamp(props: { body: string }): JSX.Element {
  const secs = () => {
    const raw = props.body.replace(/^youtube-timestamp\s*/i, "").trim();
    const n = parseInt(raw, 10);
    return Number.isFinite(n) ? n : 0;
  };
  const label = () => {
    const s = Math.max(0, secs());
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    const sec = s % 60;
    const pad = (x: number) => String(x).padStart(2, "0");
    return h > 0 ? `${h}:${pad(m)}:${pad(sec)}` : `${m}:${pad(sec)}`;
  };
  return (
    <a
      class="youtube-ts"
      title={renderMessage(msg("media-youtube-timestamp-seek"))}
      onClick={(event) => {
        event.preventDefault();
        event.stopPropagation();
        // OG timestamp click stops the event and calls seekTo(seconds, true)
        // on get-player's result (og-1.0.0 6e7afa8eb,
        // extensions/video/youtube.cljs:103-111).
        youtubePlayerForTarget(event.currentTarget)?.seekTo(secs(), true);
      }}
    >
      ⏱ {label()}
    </a>
  );
}

// `{{cloze answer}}` (optionally `{{cloze answer\\cue}}`) — in OG this is hidden
// only inside the SRS flashcard-review loop. Tine has no SRS engine, so we degrade
// to a click-to-reveal: shows the cue (or `[...]`) until clicked, then the answer.
export function ClozeMacro(props: { body: string }): JSX.Element {
  const [revealed, setRevealed] = createSignal(false);
  const parts = () => props.body.replace(/^cloze\s*/i, "").trim().split(/\\\\/);
  const answer = () => (parts()[0] ?? "").trim();
  const cue = () => parts()[1]?.trim();
  return (
    <span
      class="cloze"
      classList={{ revealed: revealed() }}
      title={revealed() ? t("macro-macro-click-to-hide") : t("macro-macro-click-to-reveal")}
      onClick={(e) => {
        e.stopPropagation();
        setRevealed((v) => !v);
      }}
    >
      {revealed() ? answer() : (cue() ?? "[...]")}
    </span>
  );
}

// `{{zotero-imported-file ...}}` / `{{zotero-linked-file ...}}` — OG resolves the
// Zotero item-key to a real attachment via its Zotero connector (data dir + item
// metadata + storage config). Tine has no Zotero integration, so resolving would
// yield a dead link; we degrade to a muted, non-navigating label rather than a
// broken link. Flagged as a known parity gap (niche).
export function ZoteroMacro(props: { body: string }): JSX.Element {
  const arg = () => props.body.replace(/^zotero-(imported|linked)-file\s*/i, "").trim();
  return (
    <span class="zotero-ref" title={t("macro-macro-zotero-integration-isn-t-supported-in-tine")}>
      📎 {arg() || t("macro-macro-zotero-attachment")}
    </span>
  );
}
