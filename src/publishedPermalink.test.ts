import { expect, it, vi } from "vitest";
import { openPublishedPermalink, publishedPermalinkHash } from "./publishedPermalink";
import { createPaneRouter } from "./router";

const holder = vi.hoisted(() => ({ api: null as unknown }));
vi.mock("./backend", () => ({ backend: () => holder.api }));

async function fixture(files: Record<string, string>) {
  vi.resetModules();
  await (await import("./render/parse")).initParser();
  const { testPublishedBackend } = await import("./publishedBackend.methods.test.support");
  const api = testPublishedBackend(files);
  holder.api = api;
  const { installPageIndex, allPages, linkTarget } = await import("./pageIndex");
  installPageIndex();
  await vi.waitFor(() => expect(allPages()).toBeDefined());
  const router = createPaneRouter();
  const open = (target: Parameters<typeof publishedPermalinkHash>[0], current = () => true) =>
    openPublishedPermalink(publishedPermalinkHash(target), router, api, linkTarget, current);
  return { router, open };
}

it.each([" /CAFÉ/ ", "/Résumé/"])("opens a page permalink with the link's canonical owner: %s", async page => {
  const { router, open } = await fixture({ "pages/cafe.md": "title:: Cafe\u0301\nalias:: Re\u0301sume\u0301\n\n- A note\n" });
  expect((await open({kind: "page", page})).status).toBe("opened");
  expect(router.route()).toMatchObject({ kind: "page", name: "Cafe\u0301", path: "pages/cafe.md" });
});

it.each(["md", "org"] as const)("block permalinks use authored %s IDs and skip code examples", async format => {
  const real = "22310000-0000-4000-8000-000000000001";
  const example = "22310000-0000-4000-8000-000000000002";
  const { router, open } = await fixture({
    [`pages/Ids.${format}`]: format === "md"
      ? `- Real\n  id:: ${real}\n- \`\`\`\n  id:: ${example}\n  \`\`\`\n`
      : `* Real\n:PROPERTIES:\n:id: ${real}\n:END:\n* Example\n#+BEGIN_SRC\n:id: ${example}\n#+END_SRC\n`,
  });
  expect((await open({ kind: "block", block: example })).status).toBe("missing");
  expect((await open({ kind: "block", block: real }, () => false)).status).toBe("ignored");
  const reveal = vi.spyOn(router, "openPageAtBlock");
  expect((await open({ kind: "block", block: real })).status).toBe("opened");
  expect(reveal).toHaveBeenCalledWith(expect.objectContaining({ name: "Ids", block: real }));
  expect(router.route()).toMatchObject({ kind: "page", name: "Ids" });
});
