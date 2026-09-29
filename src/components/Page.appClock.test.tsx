import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { doc } from "../document/model";
import { setBackendClock } from "../journal";
import type { JournalFeedPage, PageDto, PageRead } from "../types";
import { reloadJournalsFeedFromStart } from "./Page";
import { resetTabsToJournals } from "../router";
import { graphEpoch, setGraphMeta } from "../graphSession";

beforeAll(async () => { await initParser(); });
afterEach(() => {
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.restoreAllMocks();
  resetStore();
  setGraphMeta(null);
  document.body.innerHTML = "";
  resetTabsToJournals();
});

function journalDto(name: string): PageRead {
  const dto: PageDto = {
    name, kind: "journal", title: name, pre_block: null,
    blocks: [{ id: `${name}-id`, raw: name, collapsed: false, children: [] }],
  };
  return dto as PageRead;
}
function feedResponse(pages: PageRead[], as_of_day: number): JournalFeedPage {
  return { pages, next_before_day: null, done: true, as_of_day };
}

describe("journal feed day authority (GH #607)", () => {
  it("loads the backend's day when the WebView's zone rules run an hour ahead", async () => {
    // Mexico City after DST was abolished: the AppImage's bundled ICU reads
    // 00:33 on the 16th, the OS rules (the backend) 23:33 on the 15th.
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2030, 6, 16, 0, 33));
    const now = Date.now();
    setBackendClock({ offset_minutes: -new Date(now).getTimezoneOffset() - 60, unix_ms: now });
    try {
      const api = vi.spyOn(backend(), "journalFeedPage")
        .mockResolvedValue(feedResponse([journalDto("backend-day")], 20300715));
      await reloadJournalsFeedFromStart({ graphEpoch: graphEpoch(), isLive: () => true });
      expect(api).toHaveBeenCalledTimes(1);
      expect(doc.feed).toContain("backend-day");
    } finally {
      setBackendClock({ offset_minutes: -new Date(now).getTimezoneOffset(), unix_ms: now });
    }
  });
});
