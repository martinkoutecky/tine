import { backend } from "../../backend";
import { graphOwner, readOwned } from "../../owned";
import { ensurePageLoaded } from "../workingSet";
import { pageByName } from "../model";
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
    const reading = await readOwned(graphOwner(), backend().getPage(name, "page"));
    if (reading.kind === "stale") throw new Error("The graph changed before the type could be saved.");
    const dto = reading.value ?? captureEmptyPage(name, "page");
    ensurePageLoaded(dto);
    const admitted = pageByName(name);
    if (!admitted || (reading.value && admitted.id !== reading.value.id))
      throw new Error("The property key page changed before the type could be saved.");
  }
  const page = pageByName(name);
  if (page) {
    if (page.readOnly || page.guide) throw new Error("The property key page is read-only.");
    setPageProperty(name, key, value);
    return;
  }
  throw new Error("The property key page could not be loaded.");
}
