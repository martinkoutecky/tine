#!/usr/bin/env node
// Marks translations as current against today's English (tine#107, DESIGN rev. 2
// review #7): writes each id's source hash (the English message and everything it
// references) into `src/i18n/locales/<tag>/.source-hashes.json`. Run it after
// translating or reviewing; nothing else writes that file.
//
//   node scripts/i18n-accept.mjs <tag> <id>...   accept these messages
//   node scripts/i18n-accept.mjs <tag> --stale   accept every stale message the locale has
//
// A message the locale does not define cannot be accepted (it is missing, not stale).
import fs from "node:fs";
import path from "node:path";
import { HASHES_FILE, LOCALES_DIR, SOURCE_LOCALE, checkCatalog, readHashes, readLocale, sourceHash } from "./lib/i18n-catalog.mjs";

const [tag, ...rest] = process.argv.slice(2);
if (!tag || tag === SOURCE_LOCALE || !rest.length) {
  console.error("usage: node scripts/i18n-accept.mjs <tag> (<id>... | --stale)");
  process.exit(2);
}
const { errors, report } = checkCatalog();
if (errors.length) {
  console.error(`catalog errors; fix them first:\n${errors.join("\n")}`);
  process.exit(1);
}
if (!report[tag]) {
  console.error(`no locale ${tag}`);
  process.exit(1);
}
const english = readLocale(SOURCE_LOCALE).entries;
const translated = readLocale(tag).entries;
const ids = rest.includes("--stale") ? report[tag].stale : rest;
const hashes = readHashes(tag);
for (const id of ids) {
  if (!english.has(id)) throw new Error(`\`${id}\` is not an English message`);
  if (!translated.has(id)) throw new Error(`${tag} has no \`${id}\` to accept`);
  hashes[id] = sourceHash(id, english);
}
const sorted = Object.fromEntries(Object.entries(hashes).filter(([id]) => english.has(id)).sort(([a], [b]) => (a < b ? -1 : 1)));
fs.writeFileSync(path.join(LOCALES_DIR, tag, HASHES_FILE), `${JSON.stringify(sorted, null, 2)}\n`);
console.log(`${tag}: accepted ${ids.length}; ${checkCatalog().report[tag].stale.length} stale, ${report[tag].missing.length} missing`);
