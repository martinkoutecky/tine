// Queries run-10 re-review (2026-10-14): planning pickers on a breadcrumb ancestor in a
// query surface refuse when results change while open. Fail-before: 2 of 4 failed on 175ba202d.
import { afterEach, beforeAll, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { initParser } from '../render/parse';
import { resetStore, node } from '../document';
import { doc, setDoc } from '../document/model';
import { adoptResults, beginResultsChange, captureResultsTicket } from '../queryResultsEpoch';
import { LiveRefGroup } from './LiveRefGroup';
import { DatePicker } from './DatePicker';
import { closeDatePicker, datePicker } from '../ui';
import { setToasts } from '../toasts';
import type { BlockDto } from '../types';

beforeAll(initParser);
let dispose: (() => void) | undefined;
afterEach(() => { dispose?.(); closeDatePicker(); resetStore(); setToasts([]); document.body.replaceChildren(); });
const raw = 'TODO Ancestor\nSCHEDULED: <2026-10-14 Wed>\nDEADLINE: <2026-10-14 Wed>';
async function openAncestor(which: string) {
  setDoc({ byId: {
    ancestor: { id: 'ancestor', raw, collapsed: false, parent: null, page: 'Source', children: ['hit'] },
    hit: { id: 'hit', raw: 'Selected result', collapsed: false, parent: 'ancestor', page: 'Source', children: [] },
  }, pages: [{ id: 'pages/Source.md', name: 'Source', kind: 'page', title: 'Source', preBlock: null, roots: ['ancestor'], format: 'md', readOnly: false, guide: false }], feed: ['Source'], loaded: true });
  const blocks: BlockDto[] = [{ id: 'hit', raw: 'Selected result', collapsed: false, children: [] }];
  adoptResults(blocks, captureResultsTicket());
  const root = document.createElement('div'); document.body.append(root);
  dispose = render(() => <><LiveRefGroup page="Source" kind="page" path="pages/Source.md" blocks={blocks} surface="query" showBreadcrumb eager /><DatePicker /></>, root);
  await Promise.resolve();
  const crumb = root.querySelector<HTMLButtonElement>('button.ref-crumb');
  expect(crumb).not.toBeNull();
  crumb!.click();
  await Promise.resolve();
  const chip = root.querySelector<HTMLElement>(`[data-block-id="ancestor"] .date-chip.${which}`);
  expect(chip).not.toBeNull();
  chip!.click();
  expect(datePicker()?.blockId).toBe('ancestor');
  root.querySelector<HTMLButtonElement>('[data-day="20"]')!.click();
  return root;
}
function done(root: HTMLElement) { [...root.querySelectorAll<HTMLButtonElement>('.dp-btn')].find(b => b.textContent === 'Done')!.click(); }

it.each(['scheduled', 'deadline'])('query breadcrumb ancestor %s refuses same-ID replacement', async which => {
  const root = await openAncestor(which);
  const opened = node('ancestor');
  const finish = beginResultsChange();
  setDoc('byId', 'ancestor', { ...doc.byId.ancestor, raw: raw.replace('Ancestor', 'Replacement') });
  finish(); await Promise.resolve();
  expect(node('ancestor')).toBe(opened);
  const before = doc.byId.ancestor.raw;
  done(root);
  console.info(JSON.stringify({ commit: '175ba202d3b1d86a93e014543fcbaef30ba2c277', journey: `query breadcrumb ancestor ${which} replacement`, expected: { raw: before, pickerOpen: true }, observed: { raw: doc.byId.ancestor.raw, pickerOpen: datePicker() !== null }, classification: 'product' }));
  expect(doc.byId.ancestor.raw, 'replacement must remain unchanged').toBe(before);
  expect(datePicker()).not.toBeNull();
});

it.each(['scheduled', 'deadline'])('unchanged query breadcrumb ancestor %s remains editable', async which => {
  const root = await openAncestor(which);
  done(root);
  expect(doc.byId.ancestor.raw).toContain(`${which.toUpperCase()}: <2026-10-20 Tue>`);
  expect(datePicker()).toBeNull();
});
