import { editorIn } from "../tests/editorDriver";
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { For } from 'solid-js';
import { render } from 'solid-js/web';
import { installKeybindings } from '../keybindings';
import { initParser } from '../render/parse';
import { pageByName, resetStore } from '../document';
import { loadSingle } from '../document/workingSet';
import { doc } from '../document/model';
import { Block } from './Block';

beforeAll(async()=>{await initParser()});
let dispose=()=>{},keys=()=>{};
afterEach(()=>{dispose();keys();resetStore();document.body.innerHTML=''});
describe('independent GH519 selection preservation',()=>{
  for(const [label,text,start,end,direction] of [
    ['interior caret','a longer block',4,4,'none'],
    ['forward selection','a longer block',2,8,'forward'],
    ['backward selection','a longer block',2,8,'backward'],
    ['UTF16 interior caret','a 😀 longer block',5,5,'none'],
    ['code body caret','```js\nhello world\n```',4,4,'none'],
    ['code body selection','```js\nhello world\n```',2,8,'backward'],
  ] as const){
    for(const reverse of [false,true]){
      it(`${label} ${reverse?'outdent':'indent'}`,async()=>{
        const current={id:'current',raw:text,collapsed:false,children:[]};
        const parent={id:'previous',raw:'Previous',collapsed:false,children:reverse?[current]:[]};
        loadSingle({name:'Probe',kind:'page',title:'Probe',pre_block:null,blocks:reverse?[parent]:[parent,current]});
        keys=installKeybindings();
        const root=document.createElement('div');document.body.append(root);
        dispose=render(()=><For each={pageByName('Probe')?.roots??[]}>{id=><Block id={id}/>}</For>,root);
        root.querySelector('[data-block-id="current"] .block-content')!.dispatchEvent(new MouseEvent('mousedown',{bubbles:true,button:0}));
        document.dispatchEvent(new MouseEvent('mouseup',{bubbles:true,button:0}));
        const editor=()=>editorIn(root);
        await vi.waitFor(()=>expect(document.activeElement).toBe(editor().element));
        editor().select(start,end,direction);
        editor().keydown({key:'Tab',code:'Tab',shiftKey:reverse,bubbles:true,cancelable:true});
        await vi.waitFor(()=>expect(doc.byId.current.parent).toBe(reverse?null:'previous'));
        await vi.waitFor(()=>expect([editor().selection()[0],editor().selection()[1]]).toEqual([start,end]));
        if(start!==end)expect(editor().direction()).toBe(direction);
        expect(doc.byId.current.raw).toBe(text);
        expect(document.activeElement).toBe(editor().element);
      });
    }
  }
});
