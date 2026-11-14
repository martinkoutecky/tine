// Self-contained for WebDriver.execute; all geometry is in CSS pixels.
export function installAgendaProbe() {
  const find = () => document.querySelector('.agenda-block');
  const rows = () => [...(find()?.querySelectorAll('.ls-block') ?? [])];
  const scroller = document.querySelector('.main-content');
  const p = window.__gh944 = { keys: [], allMutations: [], current: null, stopped: false };
  let known = new Set(rows());
  let observedAgenda = find();
  const sample = source => {
    const box = find()?.getBoundingClientRect();
    const editor = document.activeElement?.matches('textarea.block-editor') ? document.activeElement : null;
    return { t: performance.now(), source, top: box?.top ?? null, height: box?.height ?? null,
      editorHeight: editor?.closest('.ls-block')?.getBoundingClientRect().height ?? null,
      editorBlockId: editor?.closest('.ls-block')?.dataset.blockId ?? null,
      textareaHeight: editor?.getBoundingClientRect().height ?? null, scrollTop: scroller?.scrollTop ?? null };
  };
  const observer = new MutationObserver(records => {
    records = records.filter(r => observedAgenda.contains(r.target) || [...r.addedNodes, ...r.removedNodes]
      .some(node => node === observedAgenda || (node.nodeType === 1 && node.matches('.agenda-block'))));
    if (!records.length) return;
    observedAgenda = find() ?? observedAgenda;
    const current = new Set(rows());
    const replacedRows = [...current].filter(row => !known.has(row)).length;
    const removedRows = [...known].filter(row => !current.has(row)).length;
    known = current;
    const touched = new Set(records.map(r => (r.target.nodeType === 1 ? r.target : r.target.parentElement)?.closest('.ls-block')).filter(Boolean));
    const batch = { t: performance.now(), records: records.length, replacedRows, removedRows,
      touchedRows: touched.size, textWithinOneRow: !replacedRows && !removedRows && touched.size === 1,
      characterData: records.filter(r => r.type === 'characterData').length,
      childList: records.filter(r => r.type === 'childList').length };
    p.allMutations.push({ ...batch, keyIndex: p.current ? p.keys.length - 1 : null });
    p.current?.mutations.push(batch);
  });
  // Parent observation detects replacing the list itself as well as its rows.
  observer.observe(find().parentElement, { childList: true, subtree: true, characterData: true });
  const onKey = event => {
    if (p.current && event.key !== 'Shift') {
      p.current.events.push({ key: event.key, t: performance.now() });
      p.current.frames.push(sample('keydown'));
    }
  };
  document.addEventListener('keydown', onKey, true);
  const tick = () => {
    if (p.stopped) return;
    p.current?.frames.push(sample('raf'));
    p.raf = requestAnimationFrame(tick);
  };
  p.begin = key => {
    p.current = { key, frames: [sample('before')], events: [], mutations: [] };
    p.keys.push(p.current);
  };
  p.finish = () => { p.current.frames.push(sample('after')); p.current = null; };
  p.stop = () => {
    p.stopped = true;
    cancelAnimationFrame(p.raf);
    observer.disconnect();
    document.removeEventListener('keydown', onKey, true);
    return p.keys;
  };
  p.raf = requestAnimationFrame(tick);
  return { rows: known.size, initial: sample('initial') };
}

export function summarizeKeys(keys) {
  return keys.map(key => {
    const before = key.frames[0], after = key.frames.at(-1);
    const tops = key.frames.map(f => f.top).filter(top => top !== null);
    const firstTop = before.top ?? tops[0], lastTop = after.top ?? tops.at(-1);
    const low = Math.min(firstTop, lastTop), high = Math.max(firstTop, lastTop);
    const raf = key.frames.filter(f => f.source === 'raf');
    return { key: key.key, frames: raf.length, events: key.events.length,
      maxTopExcursion: tops.length ? Math.max(0, ...tops.map(top => Math.abs(top - firstTop))) : null,
      topRange: tops.length ? Math.max(...tops) - Math.min(...tops) : null,
      missingListFrames: raf.filter(f => f.top === null).length,
      framesOverHalfPixel: raf.filter(f => f.top !== null && Math.abs(f.top - firstTop) > 0.5).length,
      maxTransientOvershoot: Math.max(0, ...tops.map(top => Math.max(low - top, top - high))),
      persistentTopDelta: before.top !== null && after.top !== null ? after.top - before.top : null,
      oneFrameEditorDips: key.frames.filter((f, i, all) => i > 0 && i < all.length - 1
        && f.editorHeight !== null && all[i - 1].editorHeight !== null && all[i + 1].editorHeight !== null
        && f.editorHeight < Math.min(all[i - 1].editorHeight, all[i + 1].editorHeight) - 0.5).length,
      mutationBatches: key.mutations.length,
      rowsReplaced: key.mutations.reduce((n, m) => n + m.replacedRows, 0),
      rowsRemoved: key.mutations.reduce((n, m) => n + m.removedRows, 0),
      textOnlyRowBatches: key.mutations.filter(m => m.textWithinOneRow).length };
  });
}
