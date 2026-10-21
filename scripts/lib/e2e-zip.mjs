// A minimal, independent zip reader for E2E journeys that check what a
// launch or safety backup holds (ADR 0074: one zip per snapshot, entries at
// their graph-relative paths). It reads the central directory and inflates
// one entry with node's zlib; it shares no code with Tine's writer, so a
// journey that passes proves the zip opens in an ordinary reader.
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";

const EOCD = 0x06054b50;
const CENTRAL = 0x02014b50;
const LOCAL = 0x04034b50;

/** `{ name, method, compressed, size, offset }` for every central-directory entry. */
export function zipEntries(file) {
  const buf = fs.readFileSync(file);
  let eocd = -1;
  for (let at = buf.length - 22; at >= Math.max(0, buf.length - 22 - 0xffff); at--) {
    if (buf.readUInt32LE(at) === EOCD) { eocd = at; break; }
  }
  if (eocd < 0) throw new Error(`${file}: no end-of-central-directory record`);
  const count = buf.readUInt16LE(eocd + 10);
  let at = buf.readUInt32LE(eocd + 16);
  const out = [];
  for (let i = 0; i < count; i++) {
    if (buf.readUInt32LE(at) !== CENTRAL) throw new Error(`${file}: bad central entry ${i}`);
    const nameLen = buf.readUInt16LE(at + 28);
    const extraLen = buf.readUInt16LE(at + 30);
    const commentLen = buf.readUInt16LE(at + 32);
    out.push({
      name: buf.subarray(at + 46, at + 46 + nameLen).toString("utf8"),
      method: buf.readUInt16LE(at + 10),
      compressed: buf.readUInt32LE(at + 20),
      size: buf.readUInt32LE(at + 24),
      offset: buf.readUInt32LE(at + 42),
    });
    at += 46 + nameLen + extraLen + commentLen;
  }
  return out;
}

/** The bytes of entry `name` in `file`, or `undefined` when it has none. */
export function zipEntry(file, name) {
  const entry = zipEntries(file).find((e) => e.name === name);
  if (!entry) return undefined;
  return entryBytes(file, fs.readFileSync(file), entry);
}

function entryBytes(file, buf, entry) {
  const { name } = entry;
  const at = entry.offset;
  if (buf.readUInt32LE(at) !== LOCAL) throw new Error(`${file}: bad local header for ${name}`);
  const start = at + 30 + buf.readUInt16LE(at + 26) + buf.readUInt16LE(at + 28);
  const raw = buf.subarray(start, start + entry.compressed);
  const bytes = entry.method === 0 ? Buffer.from(raw)
    : entry.method === 8 ? zlib.inflateRawSync(raw)
    : (() => { throw new Error(`${file}: ${name} uses method ${entry.method}`); })();
  if (bytes.length !== entry.size) throw new Error(`${file}: ${name} inflated to ${bytes.length}, expected ${entry.size}`);
  return bytes;
}

/** Extract every entry of `file` under `destDir` (refuses entries that would escape it). */
export function zipExtractAll(file, destDir) {
  const buf = fs.readFileSync(file);
  const root = path.resolve(destDir);
  for (const entry of zipEntries(file)) {
    const out = path.resolve(root, entry.name);
    if (out !== root && !out.startsWith(root + path.sep)) throw new Error(`${file}: entry ${entry.name} escapes ${destDir}`);
    if (entry.name.endsWith("/")) { fs.mkdirSync(out, { recursive: true }); continue; }
    fs.mkdirSync(path.dirname(out), { recursive: true });
    fs.writeFileSync(out, entryBytes(file, buf, entry));
  }
}

/** Every `*.zip` under `dir` (recursively) whose file name contains `marker`. */
export function findZips(dir, marker, found = []) {
  let entries = [];
  try { entries = fs.readdirSync(dir, { withFileTypes: true }); } catch { return found; }
  for (const entry of entries) {
    const full = `${dir}/${entry.name}`;
    if (entry.isDirectory()) findZips(full, marker, found);
    else if (entry.name.endsWith(".zip") && !entry.name.startsWith(".partial-") && entry.name.includes(marker)) found.push(full);
  }
  return found;
}
