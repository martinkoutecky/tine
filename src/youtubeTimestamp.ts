/** YouTube macro argument grammar from OG youtube.cljs parse-timestamp:
 * plain seconds or ^(?:(\d+):)?([0-5]?\d):([0-5]?\d)$.
 * Reject a failed match instead of letting parseInt consume only its prefix. */
export function parseYoutubeTimestamp(raw: string): number | null {
  const argument = raw.trim();
  let seconds: number;
  if (/^\d+$/.test(argument)) {
    seconds = Number(argument);
  } else {
    const match = /^(?:(\d+):)?([0-5]?\d):([0-5]?\d)$/.exec(argument);
    if (!match) return null;
    seconds = Number(match[1] ?? 0) * 3600 + Number(match[2]) * 60 + Number(match[3]);
  }
  return Number.isSafeInteger(seconds) ? seconds : null;
}

export function formatYoutubeTimestamp(seconds: number): string {
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const pad = (value: number) => String(value).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(seconds % 60)}` : `${m}:${pad(seconds % 60)}`;
}
