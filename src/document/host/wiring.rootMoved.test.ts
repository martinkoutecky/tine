import { afterEach, expect, it, vi } from "vitest";
import { resetStore, setRaw } from "../index";
import { backend } from "../../backend";
import { startEditing } from "../../editorController";
import { loadRoutedPage } from "../workingSet";
import { bindTestHost } from "./wiring.test.support";
import { setToasts, toasts } from "../../toasts";

afterEach(() => { resetStore(); setToasts([]); vi.restoreAllMocks(); });
it("a detached-root save shows the sticky kept-edits and reopen instruction",async () => {
  resetStore(); setToasts([]);
  loadRoutedPage({ name:"P",kind:"page",title:"P",id:"pages/P.md",rev:"r",pre_block:null,blocks:[{ id:"p",raw:"before",collapsed:false,children:[] }] });
  const host = await bindTestHost();
  const submit = vi.spyOn(backend(),"pageSubmit");
  setRaw("p","after",{ timetracking:false });
  startEditing("p");
  await vi.waitFor(() => expect(submit).toHaveBeenCalledOnce());
  host.notice("pages/P.md",{ saveError:true,operation:"GraphRootMoved" });
  await vi.waitFor(() => expect(toasts().some(t=> t.sticky && t.message.includes("moved, replaced or disconnected") && t.message.includes("kept on this device") && t.message.includes("reopen"))).toBe(true));
  host.notice("pages/P.md",{ saveError:true,operation:"GraphRootMoved",draftError:true });
  await vi.waitFor(() => expect(toasts().some(t=>t.message.includes("recovery copy could not be written"))).toBe(true));
  expect(toasts().some(t=>t.message.includes("Close this window"))).toBe(false);
});
