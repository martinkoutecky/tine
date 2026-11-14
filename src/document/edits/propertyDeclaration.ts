import { backend } from "../../backend";
import { bindingOwner } from "../../owned";
import { admitRead } from "../readAdmission";
import { ensurePageLoaded } from "../workingSet";
import { pageByName, pageReadOnly } from "../model";
import { captureEmptyPage } from "../convert";
import { setPageProperty } from "./properties";

/** Write `tine.type` on the normalized property-key page. Existing pages are
 * loaded with their base revision; an absent page enters the working set as an
 * empty DTO, then the ordinary page-property edit creates its file. Cost O(one
 * page's blocks) plus one backend read. Graph change, binding collision and
 * read-only failures reject to the caller; this never changes the route. */
export async function ensurePagePropertyOnKeyPage(name: string, key: string, value: string | null): Promise<void> {
  name = name.trim();
  if (!name) throw new Error("A property key needs a page name.");
  if (!pageByName(name)) {
    if (value === null) return;
    const reading = await admitRead(bindingOwner(), () => backend().getPage(name, "page"), dto => [name, ...(dto ? [dto.name] : [])], dto => {
      const refused = ensurePageLoaded(dto ?? captureEmptyPage(name, "page"));
      const admitted = pageByName(name);
      if (refused || !admitted || (dto && admitted.id !== dto.id))
        throw new Error("The property key page changed before the type could be saved.");
    });
    if (reading.kind === "stale") throw new Error("The graph changed before the type could be saved.");
  }
  const page = pageByName(name);
  if (page) {
    if (pageReadOnly(page) || page.guide) throw new Error("The property key page is read-only.");
    setPageProperty(name, key, value);
    return;
  }
  throw new Error("The property key page could not be loaded.");
}
