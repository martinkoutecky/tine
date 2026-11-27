// GH #987: evidence for which writer moved the top of the focused Journals editor.
// Test-only: an own `scrollTop` accessor on the pane forwards to the prototype and
// logs every assignment with its stack; a scroll whose offset no logged assignment
// produced is `native` (anchoring or a browser clamp). A frame sampler reads the
// editor's top before (animation frame) and after (its ResizeObserver phase) each
// rendering update, so a displacement the same update corrects is attributed, not
// failed. The labels map stack frames through the build's source maps.
import fs from "node:fs";
import path from "node:path";
// A transitive dependency (vite's); without it, labels stay bundle positions.
const { SourceMapConsumer } = await import("source-map-js").catch(() => ({}));

/** Install on the focused block editor's pane. Returns the editor's top below the pane top. */
export async function watchHeldEditor(browser) {
  return browser.execute(() => {
    const editor = document.activeElement;
    const scroller = editor?.closest(".main-content");
    if (!scroller) throw new Error("no focused editor in the main pane");
    const proto = Object.getOwnPropertyDescriptor(Element.prototype, "scrollTop");
    const top = () => editor.getBoundingClientRect().top - scroller.getBoundingClientRect().top;
    const w = window.__held = { editor, baseline: top(), frame: 0, writes: [], natives: [], frames: [], known: proto.get.call(scroller) };
    Object.defineProperty(scroller, "scrollTop", { configurable: true,
      get() { return proto.get.call(this); },
      set(value) {
        const before = proto.get.call(this);
        proto.set.call(this, value);
        w.known = proto.get.call(this);
        w.writes.push({ frame: w.frame, value, before, applied: w.known, stack: String(new Error().stack).split("\n").slice(1, 6) });
      } });
    scroller.addEventListener("scroll", () => {
      const now = proto.get.call(scroller);
      if (Math.abs(now - w.known) > 0.01) w.natives.push({ frame: w.frame, from: w.known, to: now, top: top() });
      w.known = now;
    }, { passive: true });
    // Work by source: every read of the editor's box, keyed by its caller's stack (a hold check reads it once).
    const rect = editor.getBoundingClientRect;
    w.reads = {};
    editor.getBoundingClientRect = function () {
      const key = String(new Error().stack).split("\n").slice(1, 4).join("\n");
      w.reads[key] = (w.reads[key] || 0) + 1;
      return rect.call(this);
    };
    // `pre` is read in the animation frame, before this rendering update's ResizeObserver corrections; `post` in a
    // test observer of a ticker resized every frame. Observers created later are called later, so it runs after the
    // app's corrections and before paint: what the frame shows.
    const ticker = document.createElement("div");
    ticker.style.cssText = "position:fixed;left:0;top:0;height:1px;width:1px;visibility:hidden;pointer-events:none";
    document.body.append(ticker);
    let pre = 0;
    new ResizeObserver(() => {
      if (!editor.isConnected) return;
      const post = top() - w.baseline;
      const last = w.frames.at(-1);
      if (!last || last.pre !== pre || last.post !== post) w.frames.push({ frame: w.frame, pre, post, scroll: proto.get.call(scroller) });
    }).observe(ticker);
    const sample = () => {
      if (!editor.isConnected) return;
      w.frame++;
      pre = top() - w.baseline;
      ticker.style.width = `${1 + w.frame % 2}px`;
      requestAnimationFrame(sample);
    };
    requestAnimationFrame(sample);
    return w.baseline;
  });
}

/** Wait until the editor's top and the pane's offset have not changed for `quiet` rendered frames. */
export async function waitQuiet(browser, quiet = 30, timeout = 20000) {
  await browser.waitUntil(() => browser.execute(quiet => {
    const w = window.__held;
    const last = w.frames.at(-1);
    return !w.editor.isConnected || !last || w.frame - last.frame >= quiet;
  }, quiet), { timeout, interval: 50, timeoutMsg: "the held editor did not stay still for the quiet frames" });
}

/** Grow a test-owned 16 px spacer at the top of the day list, settle, then remove it. On a live hold the editor
 * stays within 1 px both ways (an author write by the hold, or zero writes when native anchoring compensated). */
export async function armingProbe(browser) {
  return browser.executeAsync(done => {
    const w = window.__held;
    const page = w.editor.closest(".page");
    const settle = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(resolve, 0))));
    const at = () => w.editor.getBoundingClientRect().top - w.editor.closest(".main-content").getBoundingClientRect().top - w.baseline;
    (async () => {
      const first = w.writes.length, natives = w.natives.length;
      const spacer = document.createElement("div");
      spacer.style.height = "16px";
      page.prepend(spacer);
      await settle(); await settle();
      const grown = at();
      const growWrites = w.writes.length - first, growNatives = w.natives.length - natives;
      spacer.remove();
      await settle(); await settle();
      done({ grown, shrunk: at(), growWrites, shrinkWrites: w.writes.length - first - growWrites, growNatives, writes: w.writes.slice(first) });
    })();
  });
}

export async function heldEditorEvidence(browser) {
  return browser.execute(() => {
    const w = window.__held;
    return { baseline: w.baseline, frames: w.frames, writes: w.writes, natives: w.natives, reads: { ...w.reads }, connected: w.editor.isConnected, sameEditor: document.activeElement === w.editor,
      top: w.editor.getBoundingClientRect().top - w.editor.closest(".main-content").getBoundingClientRect().top - w.baseline };
  });
}

/** The editor-box read counts so far (to subtract the probe's from the change's). */
export async function readCounts(browser) {
  return browser.execute(() => ({ ...window.__held.reads }));
}

/** Map stack lines to `src/file:line` through `dist`'s source maps (app bundles only). */
function stackMapper(dist) {
  const consumers = new Map();
  const consumer = file => {
    if (!consumers.has(file)) {
      const map = path.join(dist, "assets", `${file}.map`);
      consumers.set(file, SourceMapConsumer && fs.existsSync(map) ? new SourceMapConsumer(JSON.parse(fs.readFileSync(map, "utf8"))) : null);
    }
    return consumers.get(file);
  };
  return lines => lines.map(line => {
    const match = /([^/@\s]+\.js):(\d+):(\d+)\)?$/.exec(line);
    if (!match) return null;
    const original = consumer(match[1])?.originalPositionFor({ line: Number(match[2]), column: Number(match[3]) - 1 });
    return original?.source ? `${original.name ?? "?"}@${original.source.replace(/^.*\/src\//, "src/")}:${original.line}` : `${match[1]}:${match[2]}:${match[3]}`;
  }).filter(Boolean);
}

/** Name each logged write by the first app frame of its stack, through `dist`'s source maps. */
export function labelWrites(writes, dist) {
  const map = stackMapper(dist);
  return writes.map(write => {
    const frames = map(write.stack);
    return { ...write, stack: undefined, source: frames[0] ?? "unknown", frames };
  });
}

/** Editor-box reads by caller (its first two app frames), minus the counts in `since`; test reads are dropped. */
export function labelReads(reads, dist, since = {}) {
  const map = stackMapper(dist);
  const out = {};
  for (const [key, count] of Object.entries(reads)) {
    const delta = count - (since[key] || 0);
    const frames = map(key.split("\n"));
    if (!delta || !frames.length) continue;
    const label = frames.slice(0, 2).join(" < ");
    out[label] = (out[label] || 0) + delta;
  }
  return out;
}
