import { afterEach, expect, it, vi } from "vitest";
import { backend } from "../../backend";
import { loadRoutedPage } from "../workingSet";
import { bindHost, isConflicted, pageByName, node, resetStore } from "../index";
import { bindTestHost, mailPage } from "./wiring.test.support";
import type { PageDto } from "../../types";

afterEach(() => { resetStore(); vi.restoreAllMocks(); });

it.each([false,true])("recovery refreshes a view-only loaded page from host custody (conflict: %s)",async conflict => {
  const dto=(raw: string): PageDto & {id:string} => ({name:"P",kind:"page",title:"P",id:"pages/P.md",rev:raw,pre_block:null,
    blocks:[{id:"recovery-block",raw,collapsed:false,children:[]}]});
  loadRoutedPage(dto("before"));
  const delivery=await bindTestHost();
  // No editor acquisition exists. The replacement host has imported text that
  // may still be conflicted; a disk DTO/ordinary route load cannot supply it.
  vi.spyOn(backend(),"pageOpen").mockImplementation(async (session,id,page) => {
    setTimeout(()=>delivery.deliver({session,key:page.path!,
      answer:{id,version:2,took:false,outcome:{kind:"applied"}},
      page:mailPage(2,dto("recovered input"),{conflict})}),0);
    return {key:page.path!,baselineEntry:false};
  });
  await bindHost({refreshLoaded:true});
  expect(node(pageByName("P")!.roots[0])?.raw).toBe("recovered input");
  expect(isConflicted("P")).toBe(conflict);
});
