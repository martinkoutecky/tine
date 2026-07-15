// Expanded audio player overlay. Opened by the "Expand" button on an inline audio
// embed (render/inline.tsx → MediaEmbed). A dimmed, ~90vw panel "over" the app
// (like opening a video with no picture) with a wide streaming scrubber plus
// ±5s / ±15s skip, play/pause, speed, and a time read-out. Esc / click-out / ✕
// closes. It deliberately does not decode the whole track merely to draw a
// waveform: long compressed recordings can expand into gigabytes of PCM.

import { For, Show, createEffect, createResource, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { audioPlayer, setAudioPlayer } from "../ui";
import { backend, isTauri } from "../backend";
import { acquireMediaBlobFallback, type MediaBlobLease } from "../mediaBlobFallback";

/** Bare `assets/`-relative path of a media URL (mirrors inline.tsx's helper). */
function relOf(url: string): string | null {
  const i = url.indexOf("assets/");
  return i === -1 ? null : url.slice(i + "assets/".length);
}
const isExternal = (u: string) => /^(https?:|data:|blob:)/.test(u);

function fmtTime(t: number): string {
  if (!isFinite(t) || t < 0) t = 0;
  const m = Math.floor(t / 60);
  const s = Math.floor(t % 60);
  return `${m}:${String(s).padStart(2, "0")}`;
}

/** Paint a streaming progress bar with a played/unplayed split and playhead. */
function drawWave(canvas: HTMLCanvasElement | undefined, progress: number): void {
  if (!canvas) return;
  const dpr = window.devicePixelRatio || 1;
  const cssW = canvas.clientWidth || 800;
  const cssH = canvas.clientHeight || 96;
  const w = Math.round(cssW * dpr);
  const h = Math.round(cssH * dpr);
  if (canvas.width !== w || canvas.height !== h) {
    canvas.width = w;
    canvas.height = h;
  }
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, cssW, cssH);
  const cs = getComputedStyle(canvas);
  const played = cs.getPropertyValue("--accent").trim() || "#4c8bf5";
  const unplayed = cs.getPropertyValue("--text-muted").trim() || "rgba(140,140,150,0.55)";
  const mid = cssH / 2;
  const p = Math.max(0, Math.min(1, progress));
  // Faint baseline so the scrubber reads as a seekable track even for quiet/silent
  // audio (whose peak bars are near-invisible) or before metadata loads.
  ctx.globalAlpha = 0.35;
  ctx.fillStyle = unplayed;
  ctx.fillRect(0, mid - 1, cssW, 2);
  ctx.globalAlpha = 1;
  ctx.fillStyle = unplayed;
  ctx.fillRect(0, mid - 3, cssW, 6);
  ctx.fillStyle = played;
  ctx.fillRect(0, mid - 3, cssW * p, 6);
  ctx.fillStyle = played;
  ctx.fillRect(Math.min(cssW - 2, p * cssW), 2, 2, cssH - 4);
}

export function AudioOverlay(): JSX.Element {
  // Resolve to a range-aware native URL for graph assets (same path as the inline
  // embed), or the direct URL for external/http audio.
  const [src] = createResource(
    () => audioPlayer()?.url ?? null,
    async (u) => {
      if (isExternal(u)) return u;
      const r = relOf(u);
      return r ? await backend().streamAsset(r) : "";
    }
  );
  const [blobFallback, setBlobFallback] = createSignal("");
  const resolvedSrc = () => blobFallback() || src();
  let tryingBlobFallback = false;
  let blobLease: MediaBlobLease | null = null;
  let fallbackAbort: AbortController | null = null;
  let fallbackGeneration = 0;
  const releaseBlobFallback = () => {
    fallbackGeneration += 1;
    fallbackAbort?.abort();
    fallbackAbort = null;
    blobLease?.release();
    blobLease = null;
    setBlobFallback("");
    tryingBlobFallback = false;
  };
  const retryAsBoundedBlob = () => {
    const u = audioPlayer()?.url;
    if (!u || isExternal(u) || tryingBlobFallback || blobFallback()) return;
    const rel = relOf(u);
    if (!rel) return;
    tryingBlobFallback = true;
    const generation = fallbackGeneration;
    const abort = new AbortController();
    fallbackAbort = abort;
    const ext = rel.split(".").pop()?.toLowerCase();
    const mime = ext === "mp3" || ext === "mpeg" ? "audio/mpeg" :
      ext === "m4a" || ext === "aac" ? "audio/mp4" :
      ext === "wav" ? "audio/wav" :
      ext === "ogg" || ext === "oga" ? "audio/ogg" :
      ext === "opus" ? "audio/opus" :
      ext === "flac" ? "audio/flac" : "application/octet-stream";
    void acquireMediaBlobFallback(rel, "audio", mime, abort.signal).then((lease) => {
      if (generation !== fallbackGeneration || audioPlayer()?.url !== u) {
        lease.release();
        return;
      }
      blobLease = lease;
      setBlobFallback(lease.url);
    }).catch(() => {});
  };
  onCleanup(releaseBlobFallback);

  // The component is app-lifetime mounted; closing the inner Show or switching
  // tracks must explicitly end the old fallback lease and invalidate late reads.
  let activeUrl: string | null = null;
  createEffect(() => {
    const next = audioPlayer()?.url ?? null;
    if (next === activeUrl) return;
    activeUrl = next;
    releaseBlobFallback();
  });

  let audioEl: HTMLAudioElement | undefined;
  let canvasEl: HTMLCanvasElement | undefined;
  const [playing, setPlaying] = createSignal(false);
  const [cur, setCur] = createSignal(0);
  const [dur, setDur] = createSignal(0);
  const [rate, setRate] = createSignal(1);

  const close = () => {
    audioEl?.pause();
    releaseBlobFallback();
    setAudioPlayer(null);
  };
  const togglePlay = () => {
    if (!audioEl) return;
    if (audioEl.paused) void audioEl.play().catch(() => {});
    else audioEl.pause();
  };
  const skip = (d: number) => {
    if (audioEl) audioEl.currentTime = Math.max(0, Math.min(dur(), audioEl.currentTime + d));
  };
  const seekTo = (ratio: number) => {
    if (audioEl && dur()) audioEl.currentTime = Math.max(0, Math.min(1, ratio)) * dur();
  };

  // Screenshot-harness hook (web/mock only — headless WebKit can't decode the
  // fixture media inline, so the in-app "Expand" button never appears there).
  // No effect in the shipped Tauri app.
  onMount(() => {
    if (!isTauri()) {
      (window as unknown as { __tineOpenAudio?: (u: string, n: string) => void }).__tineOpenAudio = (
        u: string,
        n: string
      ) => setAudioPlayer({ url: u, name: n });
    }
  });

  // Esc closes; Space toggles play (capture phase so it wins over block handlers).
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!audioPlayer()) return;
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopImmediatePropagation();
        close();
      } else if (e.key === " " || e.code === "Space") {
        e.preventDefault();
        e.stopImmediatePropagation();
        togglePlay();
      }
    };
    window.addEventListener("keydown", onKey, true);
    onCleanup(() => window.removeEventListener("keydown", onKey, true));
  });

  // Redraw whenever the position or duration changes.
  createEffect(() => drawWave(canvasEl, dur() ? cur() / dur() : 0));

  // Draw once the canvas has layout (it mounts with the overlay) and on resize —
  // the per-frame loop only runs during playback, so without this a paused or
  // still-loading track would show an empty scrubber.
  createEffect(() => {
    if (!audioPlayer()) return;
    const redraw = () => drawWave(canvasEl, dur() ? cur() / dur() : 0);
    requestAnimationFrame(redraw);
    window.addEventListener("resize", redraw);
    onCleanup(() => window.removeEventListener("resize", redraw));
  });

  // Smooth playhead while playing (timeupdate alone fires only ~4×/s).
  let raf = 0;
  const tick = () => {
    if (audioEl) setCur(audioEl.currentTime);
    raf = requestAnimationFrame(tick);
  };
  onCleanup(() => cancelAnimationFrame(raf));

  const onWavePointer = (e: PointerEvent) => {
    const c = canvasEl;
    if (!c) return;
    e.preventDefault();
    const r = c.getBoundingClientRect();
    const at = (x: number) => seekTo((x - r.left) / r.width);
    at(e.clientX);
    const onMove = (me: PointerEvent) => at(me.clientX);
    const onUp = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  };

  return (
    <Show when={audioPlayer()}>
      <div class="audio-overlay" onClick={close}>
        <div class="audio-panel" onClick={(e) => e.stopPropagation()}>
          <div class="audio-head">
            <span class="audio-title mono">{audioPlayer()!.name}</span>
            <button class="audio-close" title="Close (Esc)" onClick={close}>
              ✕
            </button>
          </div>
          <canvas ref={canvasEl} class="audio-wave" onPointerDown={onWavePointer} />
          <div class="audio-controls">
            <button class="audio-btn" title="Back 15s" onClick={() => skip(-15)}>⏮ 15s</button>
            <button class="audio-btn" title="Back 5s" onClick={() => skip(-5)}>⏪ 5s</button>
            <button class="audio-btn audio-play" title="Play / Pause (Space)" onClick={togglePlay}>
              {playing() ? "⏸" : "▶"}
            </button>
            <button class="audio-btn" title="Forward 5s" onClick={() => skip(5)}>5s ⏩</button>
            <button class="audio-btn" title="Forward 15s" onClick={() => skip(15)}>15s ⏭</button>
            <span class="audio-time mono">
              {fmtTime(cur())} / {fmtTime(dur())}
            </span>
            <select
              class="audio-rate settings-select"
              value={String(rate())}
              onChange={(e) => {
                const v = Number(e.currentTarget.value);
                setRate(v);
                if (audioEl) audioEl.playbackRate = v;
              }}
            >
              <For each={[0.5, 0.75, 1, 1.25, 1.5, 2]}>
                {(r) => <option value={String(r)}>{r}×</option>}
              </For>
            </select>
          </div>
          <audio
            ref={audioEl}
            style={{ display: "none" }}
            src={resolvedSrc() || undefined}
            autoplay
            onError={retryAsBoundedBlob}
            onLoadedMetadata={(e) => setDur(e.currentTarget.duration || 0)}
            onDurationChange={(e) => setDur(e.currentTarget.duration || 0)}
            onPlay={() => {
              setPlaying(true);
              cancelAnimationFrame(raf);
              raf = requestAnimationFrame(tick);
            }}
            onPause={() => {
              setPlaying(false);
              cancelAnimationFrame(raf);
            }}
            onTimeUpdate={(e) => setCur(e.currentTarget.currentTime)}
            onEnded={() => setPlaying(false)}
          />
        </div>
      </div>
    </Show>
  );
}
