import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { For } from 'solid-js';
import { render } from 'solid-js/web';
import { startEditing } from '../editorController';
import { installKeybindings } from '../keybindings';
import { initParser } from '../render/parse';
import { doc, loadSingle, pageByName, resetStore } from '../store';
import { Block } from './Block';

beforeAll(async()=>{await initParser()});
let dispose=()=>{},keys=()=>{};
afterEach(()=>{dispose();keys();resetStore();document.body.innerHTML=''});
describe('exploratory GH519 newer selection wins',()=>{
  for(const [label,text,start,end,direction] of [
    ['interior caret','a longer block',4,4,'none'],
    ['forward selection','a longer block',2,8,'forward'],
    ['backward selection','a longer block',2,8,'backward'],
    ['UTF16 interior caret','a 😀 longer block',5,5,'none'],
  ] as const){
    for(const reverse of [false,true]){
      it(`${label} ${reverse?'outdent':'indent'}`,async()=>{
        const current={id:'current',raw:text,collapsed:false,children:[]};
        loadSingle({name:'Probe',kind:'page',title:'Probe',pre_block:null,blocks:[current]});
        startEditing('current',0);keys=installKeybindings();
        const root=document.createElement('div');document.body.append(root);
        dispose=render(()=><For each={pageByName('Probe')?.roots??[]}>{id=><Block id={id}/>}</For>,root);
        const editor=()=>root.querySelector('textarea.block-editor') as HTMLTextAreaElement;
        await vi.waitFor(()=>expect(document.activeElement).toBe(editor()));
        editor().setSelectionRange(start,end,direction);
        editor().dispatchEvent(new KeyboardEvent('keydown',{key:'Tab',code:'Tab',shiftKey:reverse,bubbles:true,cancelable:true}));
        await vi.waitFor(()=>expect(doc.byId.current.parent).toBe(null));
        editor().focus();
        editor().setSelectionRange(6,6);
        editor().dispatchEvent(new Event('select',{bubbles:true}));
        await new Promise<void>(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve())));
        expect([editor().selectionStart,editor().selectionEnd]).toEqual([6,6]);
        expect(doc.byId.current.raw).toBe(text);
        expect(document.activeElement).toBe(editor());
      });
    }
  }
});
