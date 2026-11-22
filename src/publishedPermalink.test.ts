import { expect, it, vi } from "vitest";
import {
  openPublishedPermalink, publishedPermalinkForWorkspace, publishedPermalinkHash, setQuickSharePrefix,
} from "./publishedPermalink";
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
  const openHash = (hash: string, paneRouter = router, current = () => true) =>
    openPublishedPermalink(hash, paneRouter, api, linkTarget, current);
  const open = (target: Parameters<typeof publishedPermalinkHash>[0], current = () => true) =>
    openHash(publishedPermalinkHash(target), router, current);
  return { router, open, openHash };
}

it.each([" /CAFÉ/ ", "/Résumé/"])("opens a page permalink with the link's canonical owner: %s", async page => {
  const { router, open } = await fixture({ "pages/cafe.md": "title:: Cafe\u0301\nalias:: Re\u0301sume\u0301\n\n- A note\n" });
  expect((await open({kind: "page", page})).status).toBe("opened");
  expect(router.route()).toMatchObject({ kind: "page", name: "Cafe\u0301", path: "pages/cafe.md" });
});

it.each([null, "tine-fixture/0123456789abcdef0123/key"])(
  "opens and reopens alias and date permalinks (share prefix: %s)",
  async prefix => {
    const { openHash } = await fixture({
      "pages/Alpha.md": "alias:: Shared, Empty\n\n- First owner\n",
      "pages/Z.md": "alias:: Shared\n\n- Second owner\n",
      "pages/Empty.md": "",
      "pages/Links.md": "- [[2026-10-26]] [[launch day]]\n",
      "journals/2026_10_26.md": "alias:: launch day\n\n- Journal body\n",
    });
    const hash = (target: Parameters<typeof publishedPermalinkHash>[0]) => {
      const permalink = publishedPermalinkHash(target);
      return prefix ? `#${prefix}${permalink.slice(1)}` : permalink;
    };
    setQuickSharePrefix(prefix);
    try {
      for (const [page, path] of [
        ["Shared", "pages/Alpha.md"],
        ["Empty", "pages/Empty.md"],
        ["2026-10-26", "journals/2026_10_26.md"],
        ["launch day", "journals/2026_10_26.md"],
      ]) {
        const opened = await openHash(hash({ kind: "page", page }));
        expect(opened).toMatchObject({ status: "opened", route: { kind: "page", path } });
        if (opened.status !== "opened") throw new Error(`Could not open ${page}`);
        const written = publishedPermalinkForWorkspace(1, 1, opened.route);
        expect(written).toMatchObject({ kind: "page" });
        if (!written) throw new Error(`No workspace permalink for ${page}`);
        expect(await openHash(hash(written), createPaneRouter())).toMatchObject({
          status: "opened", route: { kind: "page", path },
        });
      }
    } finally {
      setQuickSharePrefix(null);
    }
  },
);

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
