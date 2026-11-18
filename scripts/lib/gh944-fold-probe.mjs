// GH #944 comment 11: does a fold, unfold or task tick move the block under the pointer?
// `installFoldProbe` is self-contained for WebDriver.execute; geometry is in CSS pixels.
// It follows one block (by id) through every animation frame after an action and counts the
// backend commands the action causes. `summarizeFold` is the pure reduction of those frames.

export function installFoldProbe(targetId) {
  const scroller = document.querySelector('.main-content');
  // The same block can also be listed in the agenda or a references section: follow the page's own row.
  const target = () => [...scroller.querySelectorAll(`[data-block-id="${targetId}"]`)]
    .find(el => !el.closest('.agenda-block, .journal-linked-references, .query-block'));
  const sections = () => [...scroller.querySelectorAll('.agenda-block, .journal-linked-references, .journal-feed-day, .today-queries, .journal-upcoming')];
  const label = el => `${el.className.split(' ')[0]}${el.dataset.feedDay ? `:${el.dataset.feedDay}` : ''}`;
  const snapshot = () => {
    const t = target();
    return sections().map(el => ({ label: label(el), height: el.getBoundingClientRect().height,
      above: !!t && !el.contains(t) && !!(el.compareDocumentPosition(t) & Node.DOCUMENT_POSITION_FOLLOWING) }));
  };
  const p = window.__gh944fold = { frames: [], commands: {}, stopped: false, before: snapshot(), pointerEvents: [] };
  const sample = source => {
    const t = target();
    const box = t?.getBoundingClientRect();
    const agenda = scroller.querySelector('.agenda-block');
    return { t: performance.now(), source, top: box?.top ?? null, height: box?.height ?? null,
      scrollTop: scroller.scrollTop, scrollHeight: scroller.scrollHeight, clientHeight: scroller.clientHeight,
      docTop: box ? box.top - scroller.getBoundingClientRect().top + scroller.scrollTop : null,
      agendaHeight: agenda?.getBoundingClientRect().height ?? null,
      agendaRows: agenda ? agenda.querySelectorAll('.ls-block').length : null };
  };
  // Tauri defines __TAURI_INTERNALS__.invoke read-only; its IPC is a fetch to ipc://localhost/<command>
  // (http://ipc.localhost/<command> on Windows), with postMessage as the fallback. Count the fetches.
  const realFetch = window.fetch;
  const countCommand = url => {
    const m = /^(?:ipc:\/\/localhost|https?:\/\/ipc\.localhost)\/([^?#]+)/.exec(String(url));
    if (m) p.commands[m[1]] = (p.commands[m[1]] ?? 0) + 1;
    return m?.[1];
  };
  window.fetch = function (input, init, ...rest) {
    const cmd = countCommand(typeof input === 'string' || input instanceof URL ? input : input?.url);
    if (cmd === 'query_run' || cmd === 'register_query_subscription') {
      try { (p.queries ??= []).push({ cmd, t: performance.now(), body: typeof init?.body === 'string' ? init.body.slice(0, 400) : `[${init?.body?.constructor?.name}]` }); } catch {}
    }
    return realFetch.call(this, input, init, ...rest);
  };
  p.hooked = window.fetch !== realFetch;
  // Every document-level event that could release a viewport reserve or anchor.
  for (const type of ['pointerdown', 'keydown']) {
    const on = () => p.pointerEvents.push({ type, t: performance.now() });
    document.addEventListener(type, on, true);
    p[`off-${type}`] = () => document.removeEventListener(type, on, true);
  }
  const tick = () => {
    if (p.stopped) return;
    p.frames.push(sample('raf'));
    p.raf = requestAnimationFrame(tick);
  };
  p.frames.push(sample('initial'));
  p.raf = requestAnimationFrame(tick);
  p.stop = () => {
    p.stopped = true;
    cancelAnimationFrame(p.raf);
    window.fetch = realFetch;
    p['off-pointerdown']?.(); p['off-keydown']?.();
    p.frames.push(sample('final'));
    return { frames: p.frames, commands: p.commands, queries: p.queries ?? [], before: p.before, after: snapshot(), pointerEvents: p.pointerEvents };
  };
  return { top: p.frames[0].top, scrollTop: p.frames[0].scrollTop, hooked: !!p.hooked };
}

// Reduce a probe run. `initial` is the frame before the action; later frames follow it.
export function summarizeFold(run) {
  const { frames, commands, before, after } = run;
  const first = frames[0], last = frames.at(-1);
  const tops = frames.map(f => f.top).filter(top => top !== null);
  const queryCommands = Object.entries(commands).filter(([cmd]) => /query|run_|read|refs|linked/i.test(cmd));
  const delta = (a, b) => (a === null || b === null ? null : b - a);
  const heightChanges = before.map((section, i) => ({ label: section.label, above: section.above, before: section.height,
    after: after[i]?.height ?? null, delta: delta(section.height, after[i]?.height ?? null) }))
    .filter(section => section.delta !== null && Math.abs(section.delta) > 0.5);
  const lastFinite = frames.filter(f => f.top !== null).at(-1);
  const maxScroll = Math.max(0, last.scrollHeight - last.clientHeight);
  return {
    frames: frames.length,
    durationMs: last.t - first.t,
    initialTop: first.top,
    finalTop: lastFinite?.top ?? null,
    finalDisplacement: first.top !== null && lastFinite ? lastFinite.top - first.top : null,
    maxExcursion: tops.length ? Math.max(...tops.map(top => Math.abs(top - first.top))) : null,
    framesOverHalfPixel: tops.filter(top => Math.abs(top - first.top) > 0.5).length,
    scrollTopDelta: last.scrollTop - first.scrollTop,
    scrollHeightDelta: last.scrollHeight - first.scrollHeight,
    docTopDelta: delta(first.docTop, lastFinite?.docTop ?? null),
    stillScrolls: maxScroll > 0.5,
    maxScroll,
    scrollDistanceToEnd: maxScroll - last.scrollTop,
    agendaHeightRange: [Math.min(...frames.map(f => f.agendaHeight ?? Infinity)), Math.max(...frames.map(f => f.agendaHeight ?? -Infinity))],
    agendaRowsRange: [Math.min(...frames.map(f => f.agendaRows ?? Infinity)), Math.max(...frames.map(f => f.agendaRows ?? -Infinity))],
    commands, queryCommands: Object.fromEntries(queryCommands),
    sectionHeightChanges: heightChanges,
    missingTargetFrames: frames.filter(f => f.top === null).length,
  };
}

// C837 (D-20261112-C837): the page moving because it no longer needs to scroll is by design.
// A displacement counts as a real jump only when the page still scrolls after the action and
// the scroller did not just clamp to the new, shorter maximum.
export function classifyFold(summary) {
  if (summary.missingTargetFrames === summary.frames) return 'target-gone';
  // WebKit's scrollTop is an integer and layout heights are fractional: each compensation can leave up to a pixel.
  if (summary.maxExcursion === null || summary.maxExcursion <= 2.5) return 'no-movement';
  if (!summary.stillScrolls) return 'c837-fits-window';
  if (summary.scrollTopDelta < -0.5 && summary.scrollDistanceToEnd <= 0.5) return 'c837-clamped-to-end';
  return Math.abs(summary.finalDisplacement ?? 0) > 2.5 ? 'jump-persistent' : 'jump-transient';
}
