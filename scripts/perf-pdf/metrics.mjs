// Turn a collector dump into per-trial metrics (evidence harness; never integrate).
// frame row: [rafT, blank, stale, sharp, visArea, vpArea, canvasPx, canvases, scrollTop, scrollLeft, scale, blankPages, sampleMs]
export const F = { t: 0, blank: 1, stale: 2, sharp: 3, vis: 4, vp: 5, canvasPx: 6, canvases: 7, top: 8, left: 9, scale: 10, blankPages: 11 };

const q = (xs, p) => {
  if (!xs.length) return null;
  const s = [...xs].sort((a, b) => a - b);
  return s[Math.min(s.length - 1, Math.max(0, Math.ceil(p * s.length) - 1))];
};
const r1 = (x) => (x == null ? null : Math.round(x * 10) / 10);

/** Metrics over frames with ta <= t <= tb. */
export function windowMetrics(frames, ta, tb) {
  const fr = frames.filter((f) => f[F.t] >= ta && f[F.t] <= tb);
  const iv = [];
  let blankMs = 0, staleMs = 0, peakBlank = 0, blankFrames = 0, longMs = 0;
  for (let i = 0; i < fr.length; i++) {
    const dt = i + 1 < fr.length ? fr[i + 1][F.t] - fr[i][F.t] : 0;
    if (i + 1 < fr.length) { iv.push(dt); if (dt > 50) longMs += dt; }
    const vp = fr[i][F.vp] || 1;
    blankMs += (fr[i][F.blank] / vp) * dt;
    staleMs += (fr[i][F.stale] / vp) * dt;
    peakBlank = Math.max(peakBlank, fr[i][F.blank] / vp);
    if (fr[i][F.blank] > 0) blankFrames++;
  }
  return {
    frames: fr.length,
    ms: fr.length ? r1(fr[fr.length - 1][F.t] - fr[0][F.t]) : 0,
    p50: r1(q(iv, 0.5)), p95: r1(q(iv, 0.95)), max: r1(iv.length ? Math.max(...iv) : null),
    over50: iv.filter((x) => x > 50).length, over100: iv.filter((x) => x > 100).length, longMs: r1(longMs),
    blankMs: r1(blankMs), peakBlankPct: r1(peakBlank * 100), blankFrames, staleMs: r1(staleMs),
    canvasMPxPeak: r1(Math.max(0, ...fr.map((f) => f[F.canvasPx])) / 1e6),
    canvasMPxEnd: fr.length ? r1(fr[fr.length - 1][F.canvasPx] / 1e6) : null,
  };
}

/** First frame at/after t0 where the visible area is entirely sharp. */
export function firstSharp(frames, t0) {
  for (const f of frames) if (f[F.t] >= t0 && f[F.vis] > 0 && f[F.blank] === 0 && f[F.stale] === 0) return f[F.t];
  return null;
}
/** First frame at/after t0 where something visible differs from the frame before t0 (scale/scroll/coverage). */
export function firstChange(frames, t0, key = (f) => `${f[F.scale]}|${f[F.top]}|${f[F.left]}`) {
  let before = null;
  for (const f of frames) {
    if (f[F.t] < t0) { before = key(f); continue; }
    if (before !== null && key(f) !== before) return f[F.t];
  }
  return null;
}
export const ms = (a, b) => (a == null || b == null ? null : r1(a - b));
export function rssPeak(samples) {
  if (!samples.length) return null;
  return { mainMiB: r1(Math.max(...samples.map((s) => s[1])) / 1024), webMiB: r1(Math.max(...samples.map((s) => s[2])) / 1024),
    webEndMiB: r1(samples[samples.length - 1][2] / 1024), mainEndMiB: r1(samples[samples.length - 1][1] / 1024),
    // Growth within the trial (web content + main), robust to what earlier trials left behind.
    webDeltaMiB: r1((Math.max(...samples.map((s) => s[2])) - samples[0][2]) / 1024),
    mainDeltaMiB: r1((Math.max(...samples.map((s) => s[1])) - samples[0][1]) / 1024) };
}
