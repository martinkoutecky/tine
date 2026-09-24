// Graph preparation for the hosted-Windows measurement probe
// (scripts/diagnose-win-measure.mjs). Diagnostic only: builds the three
// measurement graphs, applies a deterministic overlay whose needles have
// known hit counts, and derives the targets (hub page, rename target) from
// the files themselves so no graph content is hard-coded here.
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { generateRealisticGraph } from "./generate-realistic-graph.mjs";

// Needles planted by the overlay (all graphs) or present by construction.
export const PROBE = Object.freeze({
  onePage: "Probe One Block",
  sixtyPage: "Probe Sixty Blocks",
  rare: "quokkarare",
  zero: "qqzxjvnohit",
  ascii2: "re",
  cjk1: "证",
  cjk2: "数据",
  cjk3: "龙虾面",
  mixed: "tine测试",
  asciiUnlinkedPage: "Zanzibar",
  cjkPage: "龙井",
  extDelete: "extdelneedle",
  extModify: "extmodneedle",
  extAdd: "extaddneedle",
  renameTo: "Renamed Probe 543",
  editWord: "quill",
});

const CJK_WORDS = ["研究", "数据", "方法", "问题", "结果", "模型", "系统", "分析", "设计", "实验", "理论", "证明"];

function rng(seed) {
  let state = seed >>> 0 || 1;
  return () => {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    return (state >>> 0) / 4294967296;
  };
}

const eolOf = (text) => (text.includes("\r\n") ? "\r\n" : "\n");

/** Append one top-level block, keeping the file's own line ending. */
export function appendBlock(file, text) {
  const current = fs.readFileSync(file, "utf8");
  const eol = eolOf(current);
  const sep = current.length === 0 || current.endsWith("\n") ? "" : eol;
  fs.writeFileSync(file, `${current}${sep}- ${text}${eol}`);
}

function listMarkdown(root) {
  const out = [];
  for (const dir of ["pages", "journals"]) {
    const base = path.join(root, dir);
    if (!fs.existsSync(base)) continue;
    for (const name of fs.readdirSync(base).sort()) {
      if (name.endsWith(".md")) out.push(path.join(dir, name));
    }
  }
  return out;
}

/** Split a Markdown outline into block texts (bullet line plus continuations). */
export function blocksOf(text) {
  const blocks = [];
  let current = null;
  for (const line of text.split(/\r?\n/)) {
    if (/^\s*-( |$)/.test(line)) {
      if (current !== null) blocks.push(current);
      current = line.replace(/^\s*-\s?/, "");
    } else if (current !== null) {
      current += `\n${line}`;
    }
  }
  if (current !== null) blocks.push(current);
  return blocks;
}

/** A copied fixture may carry read-only modes; the app must be able to write. */
export function makeWritable(root) {
  for (const entry of fs.readdirSync(root, { withFileTypes: true })) {
    const full = path.join(root, entry.name);
    fs.chmodSync(full, entry.isDirectory() ? 0o755 : 0o644);
    if (entry.isDirectory()) makeWritable(full);
  }
  fs.chmodSync(root, 0o755);
}

const pageNameOfFile = (rel) => path.basename(rel, ".md").replaceAll("___", "/");

export async function buildGraph({ kind, dest, publicGraph, today = new Date() }) {
  fs.rmSync(dest, { recursive: true, force: true });
  if (kind === "real295") {
    if (!publicGraph || !fs.existsSync(publicGraph)) throw new Error("real295 needs TINE_295_PUBLIC_GRAPH");
    fs.cpSync(publicGraph, dest, { recursive: true });
    makeWritable(dest);
  } else if (kind === "realistic10k") {
    await generateRealisticGraph({ root: dest, pages: 7000, journals: 3000, seed: 543, today });
  } else if (kind === "realistic1k") {
    await generateRealisticGraph({ root: dest, pages: 700, journals: 300, seed: 543, today });
  } else if (kind === "tiny") {
    await generateRealisticGraph({ root: dest, pages: 30, journals: 10, seed: 7, today });
  } else {
    throw new Error(`unknown graph ${kind}`);
  }
  applyOverlay(dest, kind !== "real295");
  return analyze(dest, kind);
}

function applyOverlay(root, realistic) {
  const pages = path.join(root, "pages");
  fs.mkdirSync(pages, { recursive: true });
  // The existing page list, before any page is added, drives the overlay.
  const existing = fs.readdirSync(pages).filter((n) => n.endsWith(".md")).sort();
  fs.writeFileSync(path.join(pages, `${PROBE.onePage}.md`), "- probe one block start\n");
  let sixty = "";
  for (let i = 1; i <= 60; i++) {
    sixty += `- probe sixty block ${i} filler text for the edit measurement${i % 10 === 5 ? ` ${PROBE.rare}` : ""}\n`;
  }
  fs.writeFileSync(path.join(pages, `${PROBE.sixtyPage}.md`), sixty);
  for (let i = 1; i <= 5; i++) {
    fs.writeFileSync(path.join(pages, `Ext Delete ${i}.md`), `- ${PROBE.extDelete} item ${i}\n`);
  }
  if (!realistic) return;
  // CJK overlay: ~10% of pages gain one unsegmented Chinese block drawn from
  // a fixed vocabulary; some carry the 3-char and mixed needles.
  const random = rng(4321);
  let three = 0;
  let mixed = 0;
  existing.forEach((name, i) => {
    if (i % 10 !== 3) return;
    const words = Array.from({ length: 3 }, () => CJK_WORDS[Math.floor(random() * CJK_WORDS.length)]);
    let text = words.join("");
    if (three < 7 && i % 20 === 3) { text += PROBE.cjk3; three++; }
    else if (mixed < 12 && i % 20 === 13) { text += PROBE.mixed; mixed++; }
    appendBlock(path.join(pages, name), text);
  });
  // Plain-text (unlinked) mentions of the ASCII and 2-char CJK overlay pages.
  let asciiMentions = 0;
  let cjkMentions = 0;
  existing.forEach((name, i) => {
    if (i % 10 === 5 && cjkMentions < 50) { appendBlock(path.join(pages, name), `今天下午喝了${PROBE.cjkPage}，很香。`); cjkMentions++; }
    if (i % 10 === 6 && asciiMentions < 50) { appendBlock(path.join(pages, name), `we talked about ${PROBE.asciiUnlinkedPage.toLowerCase()} trips`); asciiMentions++; }
  });
  fs.writeFileSync(path.join(pages, `${PROBE.asciiUnlinkedPage}.md`), "- the ascii unlinked-reference target page\n");
  fs.writeFileSync(path.join(pages, `${PROBE.cjkPage}.md`), "- 这是一个中文页面。\n");
}

/** Count blocks (and page titles) matching each needle, and pick targets. */
export function analyze(root, kind) {
  const files = listMarkdown(root);
  const blocks = [];
  const linkBlocks = new Map(); // lowercased target -> block count
  const linkFiles = new Map(); // lowercased target -> Set(file)
  const pageFiles = new Map(); // lowercased page name -> {name, rel, titled}
  const wordBlocks = new Map();
  let bytes = 0;
  for (const rel of files) {
    const text = fs.readFileSync(path.join(root, rel), "utf8");
    bytes += Buffer.byteLength(text);
    if (rel.startsWith("pages")) {
      const name = pageNameOfFile(rel);
      pageFiles.set(name.toLowerCase(), { name, rel, titled: /^title::/im.test(text.slice(0, 2000)), bytes: Buffer.byteLength(text) });
    }
    for (const block of blocksOf(text)) {
      const lower = block.toLowerCase();
      blocks.push(lower);
      const seen = new Set();
      for (const m of block.matchAll(/\[\[([^\[\]]+?)\]\]/g)) {
        const key = m[1].trim().toLowerCase();
        if (seen.has(key)) continue;
        seen.add(key);
        linkBlocks.set(key, (linkBlocks.get(key) ?? 0) + 1);
        if (!linkFiles.has(key)) linkFiles.set(key, new Set());
        linkFiles.get(key).add(rel);
      }
      if (kind === "real295") {
        // Property lines (`collapsed:: true`) are not searchable prose.
        const prose = lower.split("\n").filter((line) => !/^\s*[\w-]+::/.test(line)).join("\n");
        const words = new Set(prose.match(/[a-z]{5,}/g) ?? []);
        for (const w of words) wordBlocks.set(w, (wordBlocks.get(w) ?? 0) + 1);
      }
    }
  }
  const candidates = [...linkBlocks.entries()]
    .map(([key, count]) => ({ key, count, files: linkFiles.get(key)?.size ?? 0, page: pageFiles.get(key) }))
    .filter((c) => c.page && !c.page.titled && !c.page.name.includes("/") && /^[\x20-\x7e]+$/.test(c.page.name)
      && !/[%#?:]/.test(c.page.name) && !c.page.name.startsWith("Ext ") && !c.page.name.startsWith("Probe "))
    .sort((a, b) => b.count - a.count || a.key.localeCompare(b.key));
  const hub = candidates[0];
  const renameTarget = candidates.filter((c) => c !== hub && c.files <= 300).sort((a, b) => b.files - a.files || a.key.localeCompare(b.key))[0];
  let common = "proof";
  if (kind === "real295") {
    common = [...wordBlocks.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))[0][0];
  }
  const count = (needle) => {
    const n = needle.toLowerCase();
    return blocks.reduce((acc, b) => acc + (b.includes(n) ? 1 : 0), 0);
  };
  const titleCount = (needle) => {
    const n = needle.toLowerCase();
    return [...pageFiles.keys()].reduce((acc, k) => acc + (k.includes(n) ? 1 : 0), 0);
  };
  const needles = {
    common, rare: PROBE.rare, zero: PROBE.zero, ascii2: PROBE.ascii2,
    cjk1: PROBE.cjk1, cjk2: PROBE.cjk2, cjk3: PROBE.cjk3, mixed: PROBE.mixed,
  };
  const expected = {};
  for (const [label, needle] of Object.entries(needles)) {
    expected[label] = { blocks: count(needle), pageTitles: titleCount(needle) };
  }
  const realistic = kind !== "real295";
  return {
    kind,
    files: files.length,
    bytes,
    blocks: blocks.length,
    needles,
    expected,
    hub: hub && { name: hub.page.name, linkBlocks: hub.count, referringFiles: hub.files },
    renameTarget: renameTarget && { name: renameTarget.page.name, linkBlocks: renameTarget.count, referringFiles: renameTarget.files },
    unlinkedPages: [
      ...(hub ? [{ label: "hub", name: hub.page.name, plainMentionBlocks: null }] : []),
      ...(realistic ? [
        { label: "ascii", name: PROBE.asciiUnlinkedPage, plainMentionBlocks: count(PROBE.asciiUnlinkedPage) },
        { label: "cjk2", name: PROBE.cjkPage, plainMentionBlocks: count(PROBE.cjkPage) },
      ] : []),
    ],
    extDeleteBlocks: count(PROBE.extDelete),
  };
}

/** Content fingerprint of every Markdown file, for counting rewritten files. */
export function snapshot(root) {
  const out = new Map();
  for (const rel of listMarkdown(root)) {
    const full = path.join(root, rel);
    try {
      const data = fs.readFileSync(full);
      out.set(rel, { sha: crypto.createHash("sha1").update(data).digest("hex"), mtimeMs: fs.statSync(full).mtimeMs });
    } catch {}
  }
  return out;
}

export function diffSnapshots(before, after) {
  const changed = [];
  const added = [];
  const removed = [];
  for (const [rel, info] of after) {
    const prior = before.get(rel);
    if (!prior) added.push(rel);
    else if (prior.sha !== info.sha) changed.push(rel);
  }
  for (const rel of before.keys()) if (!after.has(rel)) removed.push(rel);
  return { changed, added, removed };
}

/** Phase 7: modify 20, add 5, delete 5 pages outside the app. */
export function externalChange(root) {
  const pages = path.join(root, "pages");
  const skip = (n) => n.startsWith("Probe ") || n.startsWith("Ext ") || n === `${PROBE.asciiUnlinkedPage}.md`
    || n === `${PROBE.cjkPage}.md` || n === `${PROBE.renameTo}.md`;
  const pool = fs.readdirSync(pages).filter((n) => n.endsWith(".md") && !skip(n)).sort();
  const step = Math.max(1, Math.floor(pool.length / 20));
  const modified = [];
  for (let i = 0; modified.length < 20 && i < pool.length; i += step) {
    appendBlock(path.join(pages, pool[i]), `${PROBE.extModify} external edit ${modified.length + 1}`);
    modified.push(`pages/${pool[i]}`);
  }
  const added = [];
  for (let i = 1; i <= 5; i++) {
    const rel = `pages/Ext Added ${i}.md`;
    fs.writeFileSync(path.join(root, rel), `- ${PROBE.extAdd} new page ${i}\n`);
    added.push(rel);
  }
  const deleted = [];
  for (let i = 1; i <= 5; i++) {
    const rel = `pages/Ext Delete ${i}.md`;
    fs.rmSync(path.join(root, rel), { force: true });
    deleted.push(rel);
  }
  return { modified, added, deleted };
}
