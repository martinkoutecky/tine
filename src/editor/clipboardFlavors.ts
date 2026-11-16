interface ClipboardFlavors {
  types: readonly string[];
  uriList: string;
  plainText: string;
  eventFileCount: number;
}

const lines = (text: string) => text.split(/\r\n|[\r\n]/).map((line) => line.trim()).filter(Boolean);
const localPath = (line: string) => /^(?:["'])?(?:file:|content:|[a-z]:[\\/]|[\\/]|~\/)/i.test(line);
const webUri = (line: string) => {
  try {
    const uri = new URL(line);
    return (uri.protocol === "http:" || uri.protocol === "https:") && !!uri.hostname;
  } catch { return false; }
};

/** A URI list is also a browser link flavor, not proof of copied files.
 * Unknown/non-web entries stay on native ingress: only that API may supply
 * paths. Keep this boundary independent of DOM/editor APIs for other editors. */
export function classifyClipboardFlavors({ types, uriList, plainText, eventFileCount }: ClipboardFlavors) {
  const hasUriList = types.includes("text/uri-list");
  const gnomeFiles = types.includes("x-special/gnome-copied-files");
  const filesMarker = types.includes("Files");
  if (!hasUriList && !gnomeFiles && !filesMarker && !eventFileCount) {
    return { hasFiles: false, text: plainText, canFallbackToText: false };
  }
  const entries = hasUriList ? lines(uriList).filter((line) => !line.startsWith("#")) : [];
  const uriFiles = entries.some((entry) => !webUri(entry));
  const plainLines = lines(plainText);
  const pathText = plainLines.some(localPath);
  const definiteFiles = eventFileCount > 0 || gnomeFiles || uriFiles || (hasUriList && pathText);
  const text = plainText || (!uriFiles ? entries.join("\n") : "");
  // A Files sentinel alone can accompany a web link. Recover only with web
  // evidence, never an ambiguous filename, local path, or real file payload.
  const webText = (entries.length > 0 && !uriFiles)
    || (plainLines.length > 0 && plainLines.every(webUri));
  return {
    hasFiles: definiteFiles || filesMarker,
    text,
    canFallbackToText: !definiteFiles && !pathText && webText && !!text.trim(),
  };
}

/** Snapshot event data synchronously, including unreadable File items, before
 * a native read can yield. The discriminator above stays independent of DOM. */
export function clipboardEventFlavors(data: DataTransfer | null, plainText: string) {
  const fileItems = Array.from(data?.items ?? []).filter((item) => item.kind === "file");
  const eventFiles = fileItems.map((item) => item.getAsFile()).filter((file): file is File => file !== null);
  return {
    ...classifyClipboardFlavors({
      types: Array.from(data?.types ?? []),
      uriList: data?.getData("text/uri-list") ?? "",
      plainText,
      eventFileCount: fileItems.length,
    }),
    eventFiles,
  };
}
