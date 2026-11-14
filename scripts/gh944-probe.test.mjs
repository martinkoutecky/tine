import test from 'node:test';
import assert from 'node:assert/strict';
import { JSDOM } from 'jsdom';
import { installAgendaProbe, summarizeKeys } from './lib/gh944-probe.mjs';

test('captures a single animation-frame dip even when both endpoints agree, and distinguishes row text from replacement', async () => {
  const dom = new JSDOM('<div class="main-content"><div id="outside">unrelated</div><div class="agenda-block"><div class="ls-block">task</div></div><div class="ls-block"><textarea class="block-editor"></textarea></div></div>', { runScripts: 'outside-only' });
  const w = dom.window;
  try {
    let frame, top = 100, editorHeight = 48;
    w.requestAnimationFrame = callback => { frame = callback; return 1; };
    w.cancelAnimationFrame = () => {};
    const agenda = w.document.querySelector('.agenda-block');
    agenda.getBoundingClientRect = () => ({ top, height: 200 });
    const editor = w.document.querySelector('textarea');
    editor.closest('.ls-block').getBoundingClientRect = () => ({ height: editorHeight });
    editor.focus();
    w.eval(`(${installAgendaProbe.toString()})()`);
    const probe = w.__gh944;
    probe.begin('a');
    w.document.dispatchEvent(new w.KeyboardEvent('keydown', { key: 'a' }));
    frame();
    top = 76; editorHeight = 24; frame();
    top = 100; editorHeight = 48; frame();
    w.document.querySelector('#outside').textContent = 'outside changed';
    await Promise.resolve();
    assert.equal(probe.current.mutations.length, 0, 'exclude mutations outside the list');
    agenda.firstElementChild.firstChild.data = 'edited task';
    await Promise.resolve();
    agenda.firstElementChild.replaceWith(w.document.createElement('div'));
    agenda.firstElementChild.className = 'ls-block';
    await Promise.resolve();
    probe.finish();
    agenda.firstElementChild.textContent = 'between keys';
    await Promise.resolve();
    assert.equal(probe.allMutations.length, 3);
    assert.equal(probe.allMutations.at(-1).keyIndex, null, 'retain batches between observation windows');
    agenda.replaceWith(agenda.cloneNode(true));
    await Promise.resolve();
    assert.equal(probe.allMutations.length, 4, 'replacing the whole list must remain observable');
    assert.equal(probe.allMutations.at(-1).replacedRows, 1);
    assert.equal(probe.allMutations.at(-1).removedRows, 1);
    const raw = probe.stop();
    const [summary] = summarizeKeys(raw);
    assert.equal(summary.maxTopExcursion, 24);
    assert.equal(summary.framesOverHalfPixel, 1);
    assert.equal(summary.maxTransientOvershoot, 24);
    assert.equal(summary.persistentTopDelta, 0);
    assert.equal(summary.oneFrameEditorDips, 1);
    assert.equal(summary.events, 1);
    assert.equal(summary.frames, 3, 'unchanged frames must also be retained');
    assert.equal(summary.mutationBatches, 2);
    assert.equal(summary.textOnlyRowBatches, 1);
    assert.equal(summary.rowsReplaced, 1);
    assert.equal(summary.rowsRemoved, 1);
    assert.equal(raw[0].frames[2].height, 200);
    assert.equal(raw[0].frames[2].scrollTop, 0);
  } finally { w.close(); }
});

test('a lasting newline movement is reported separately from transient overshoot', () => {
  const frames = [100, 124, 124].map((top, i) => ({ top, source: i === 1 ? 'raf' : 'endpoint', editorHeight: 24 }));
  const [summary] = summarizeKeys([{ key: 'Enter', frames, events: [{}], mutations: [] }]);
  assert.equal(summary.maxTopExcursion, 24);
  assert.equal(summary.persistentTopDelta, 24);
  assert.equal(summary.maxTransientOvershoot, 0);
  assert.equal(summary.oneFrameEditorDips, 0);
});

test('a temporarily absent list is a recorded symptom, not a fabricated zero-pixel position', () => {
  const frames = [100, null, 100].map(top => ({ top, source: 'raf', editorHeight: 24 }));
  const [summary] = summarizeKeys([{ key: 'a', frames, events: [{}], mutations: [] }]);
  assert.equal(summary.missingListFrames, 1);
  assert.equal(summary.maxTopExcursion, 0);
  assert.equal(summary.framesOverHalfPixel, 0);
  assert.equal(summary.persistentTopDelta, 0);
});
