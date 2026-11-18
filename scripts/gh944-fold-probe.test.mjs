import test from 'node:test';
import assert from 'node:assert/strict';
import { summarizeFold, classifyFold } from './lib/gh944-fold-probe.mjs';

const frame = (t, top, scrollTop, scrollHeight = 3000, extra = {}) => ({ t, top, scrollTop, scrollHeight, clientHeight: 800,
  docTop: top === null ? null : top + scrollTop, agendaHeight: 700, agendaRows: 8, ...extra });
const run = (frames, over = {}) => ({ frames, commands: {}, before: [], after: [], ...over });

test('a block that stays put is no movement, and counts only query commands', () => {
  const summary = summarizeFold(run([frame(0, 400, 900), frame(16, 400, 900), frame(32, 400, 900)],
    { commands: { query_run: 1, save_page: 1 } }));
  assert.equal(classifyFold(summary), 'no-movement');
  assert.deepEqual(summary.queryCommands, { query_run: 1 });
  assert.equal(summary.framesOverHalfPixel, 0);
});

test('a lasting displacement while the page still scrolls is a real jump', () => {
  const summary = summarizeFold(run([frame(0, 400, 900), frame(16, 316, 900), frame(32, 316, 900)]));
  assert.equal(summary.maxExcursion, 84);
  assert.equal(summary.finalDisplacement, -84);
  assert.equal(classifyFold(summary), 'jump-persistent');
});

test('a bounce that returns is a transient jump with a maximum excursion', () => {
  const summary = summarizeFold(run([frame(0, 400, 900), frame(16, 340, 900), frame(32, 400, 900)]));
  assert.equal(summary.maxExcursion, 60);
  assert.equal(summary.finalDisplacement, 0);
  assert.equal(classifyFold(summary), 'jump-transient');
});

test('C837: movement because the page now fits the window, or clamped to its new end, is by design', () => {
  const fits = summarizeFold(run([frame(0, 400, 120, 1000), frame(16, 280, 0, 800)]));
  assert.equal(fits.stillScrolls, false);
  assert.equal(classifyFold(fits), 'c837-fits-window');
  const clamped = summarizeFold(run([frame(0, 400, 900, 1700), frame(16, 300, 800, 1600)]));
  assert.equal(classifyFold(clamped), 'c837-clamped-to-end');
});

test('a target that disappears is reported, not read as a zero-pixel position', () => {
  const summary = summarizeFold(run([frame(0, null, 0), frame(16, null, 0)]));
  assert.equal(classifyFold(summary), 'target-gone');
});
