import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { createEffect, onCleanup } from 'solid-js';
import { installKeybindings, matchesCommand, paletteCommands } from '../keybindings';
import { closeSettings, openSettings, setShortcutOverrides, shortcutOverrides } from '../ui';
import { clearTransientLayersForTest } from '../transientLayers';
import { Settings } from './Settings';
const tick=()=>new Promise(r=>setTimeout(r,0));
let dispose=()=>{},keys=()=>{};
afterEach(()=>{dispose();keys();clearTransientLayersForTest();closeSettings();setShortcutOverrides({});localStorage.removeItem('logseq-claude.shortcuts');document.body.innerHTML=''});
describe('independent GH523 visible clear and persistence',()=>{
  it('preserves legacy disabled configuration without crashing installation',()=>{
    expect(()=>{keys=installKeybindings({'go/find-in-page':'false'})}).not.toThrow();
  });
  it('clears redo primary and alias while retaining its palette action',async()=>{
    setShortcutOverrides({});
    const root=document.createElement('div');document.body.append(root);
    dispose=render(()=>{createEffect(()=>{const cleanup=installKeybindings({...shortcutOverrides()});onCleanup(cleanup)});return <Settings/>},root);openSettings('shortcuts');await tick();
    const row=[...root.querySelectorAll<HTMLElement>('.help-shortcut-row')].find(e=>e.querySelector('.help-shortcut-id')?.textContent==='editor/redo')!;
    [...row.querySelectorAll('button')].find(b=>/clear|unbind/i.test(b.textContent??''))!.click();await tick();
    expect(matchesCommand(new KeyboardEvent('keydown',{key:'z',ctrlKey:true,shiftKey:true}),'editor/redo')).toBe(false);
    expect(matchesCommand(new KeyboardEvent('keydown',{key:'y',ctrlKey:true}),'editor/redo')).toBe(false);
    const action=paletteCommands().find(c=>c.id==='editor/redo');expect(action).toBeDefined();expect(action!.binding).toBe('');
  });
  it('clear during recording ends recording and does not bind the next key',async()=>{
    setShortcutOverrides({});
    const root=document.createElement('div');document.body.append(root);
    dispose=render(()=>{createEffect(()=>{const cleanup=installKeybindings({...shortcutOverrides()});onCleanup(cleanup)});return <Settings/>},root);openSettings('shortcuts');await tick();
    const row=()=>[...root.querySelectorAll<HTMLElement>('.help-shortcut-row')].find(e=>e.querySelector('.help-shortcut-id')?.textContent==='go/find-in-page')!;
    (row().querySelector('.help-keycap-button') as HTMLButtonElement).click();await tick();
    [...row().querySelectorAll('button')].find(b=>/clear|unbind/i.test(b.textContent??''))!.click();await tick();
    const cleared=shortcutOverrides()['go/find-in-page'];
    window.dispatchEvent(new KeyboardEvent('keydown',{key:'j',code:'KeyJ',ctrlKey:true,bubbles:true,cancelable:true}));await tick();
    expect(shortcutOverrides()['go/find-in-page']).toBe(cleared);
    expect(row().querySelector('.recording')).toBeNull();
  });
});
