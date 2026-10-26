// Test-only: answer the wired client's page Opens from the test's loaded
// document, as the native host does for a file whose bytes are what the window
// loaded. `mockPageHost` answers an Open from the mock graph, which does not
// hold a test's own pages, so an Open made without input (a block entering
// editing, a transfer's pinned endpoints) would install an empty page there.
// Not a test file itself.

import { vi } from "vitest";
import { backend } from "../../backend";
import { pageToDto } from "../convert";
import { pageByName } from "../model";
import { baseRevFor } from "./wiring";
import { mailPage, type TestHost } from "./wiring.test.support";

let version = 1000;

/** Every Open of a loaded page with a file is answered with that page's current
 * text at its installed revision (disk = buffer), applied, not took. Other
 * Opens (a page with no file yet) go to the mock host. */
export function answerOpensFromDocument(host: TestHost): void {
  const b = backend();
  const original = b.pageOpen.bind(b);
  vi.spyOn(b, "pageOpen").mockImplementation(async (session, id, request) => {
    const loaded = pageByName(request.name);
    const dto = loaded?.id && request.path ? pageToDto(request.name) : null;
    if (!dto || !request.path) return original(session, id, request);
    const key = request.path;
    const rev = baseRevFor(request.name) ?? dto.rev ?? "disk-rev";
    version += 1;
    const at = version;
    queueMicrotask(() => host.deliver({ key, page: mailPage(at, { ...dto, rev }),
      answer: { id, version: at, took: false, outcome: { kind: "applied" } } }));
    return { key, baselineEntry: true };
  });
}
