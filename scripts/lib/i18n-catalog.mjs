// The localization catalog's one reader (tine#107, docs/contracts/localization.md):
// parses `src/i18n/locales/<tag>/*.ftl` with Fluent's own parser and answers what
// the generator, the checks and the release gate ask. Nothing else reads .ftl.
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { FluentParser, FluentSerializer, Visitor } from "@fluent/syntax";

export const LOCALES_DIR = path.resolve(import.meta.dirname, "../../src/i18n/locales");
export const SOURCE_LOCALE = "en";
/** The only Fluent functions a message may call (DESIGN rev. 2, review #5). */
export const FUNCTIONS = new Set(["NUMBER", "DATETIME"]);
/** Shell messages are read by native code: literal text only (review #3). */
export const SHELL_FILE = "shell.ftl";
export const HASHES_FILE = ".source-hashes.json";

const parser = new FluentParser({ withSpans: false });
const serializer = new FluentSerializer();

/** Every locale directory under `dir`, source locale first. */
export function localeTags(dir = LOCALES_DIR) {
  const tags = fs.readdirSync(dir, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .sort();
  return [SOURCE_LOCALE, ...tags.filter((tag) => tag !== SOURCE_LOCALE)];
}

/** One locale: its entries by id (messages and `-terms`), and every parse error. */
export function readLocale(tag, dir = LOCALES_DIR) {
  const root = path.join(dir, tag);
  const entries = new Map();
  const errors = [];
  for (const file of fs.readdirSync(root).filter((name) => name.endsWith(".ftl")).sort()) {
    const resource = parser.parse(fs.readFileSync(path.join(root, file), "utf8"));
    for (const entry of resource.body) {
      if (entry.type === "Junk") {
        errors.push(`${tag}/${file}: unparsable Fluent: ${entry.annotations.map((a) => a.message).join("; ")} near ${JSON.stringify(entry.content.slice(0, 60))}`);
        continue;
      }
      if (entry.type !== "Message" && entry.type !== "Term") continue;
      const id = entry.type === "Term" ? `-${entry.id.name}` : entry.id.name;
      const prior = entries.get(id);
      if (prior) errors.push(`${tag}/${file}: \`${id}\` is also defined in ${tag}/${prior.file}`);
      else entries.set(id, { id, entry, file });
    }
  }
  return { tag, entries, errors };
}

class Uses extends Visitor {
  constructor() {
    super();
    this.refs = new Set();
    this.variables = new Set();
    this.numeric = new Set();
    this.functions = new Set();
  }
  visitMessageReference(node) {
    this.refs.add(node.attribute ? `${node.id.name}.${node.attribute.name}` : node.id.name);
    this.genericVisit(node);
  }
  visitTermReference(node) {
    this.refs.add(`-${node.id.name}`);
    // A term's arguments are the caller's literals, not the caller's variables.
  }
  visitVariableReference(node) {
    this.variables.add(node.id.name);
  }
  visitFunctionReference(node) {
    this.functions.add(node.id.name);
    this.genericVisit(node);
  }
  visitSelectExpression(node) {
    // A selector over plural categories needs a number (review #5).
    const numericKeys = node.variants.some((variant) => variant.key.type === "Identifier"
      && ["zero", "one", "two", "few", "many"].includes(variant.key.name))
      || node.variants.some((variant) => variant.key.type === "NumberLiteral");
    if (numericKeys && node.selector.type === "VariableReference") this.numeric.add(node.selector.id.name);
    this.genericVisit(node);
  }
}

/** What one entry uses directly. */
export function uses(entry) {
  const visitor = new Uses();
  visitor.visit(entry.entry);
  return visitor;
}

function refTarget(ref) {
  return ref.split(".")[0];
}

/** Ids `id` reaches through references, `id` included; a cycle is reported once. */
export function reach(id, entries, errors = []) {
  const seen = new Set();
  const visit = (current, stack) => {
    if (stack.includes(current)) {
      errors.push(`reference cycle: ${[...stack, current].join(" → ")}`);
      return;
    }
    if (seen.has(current)) return;
    seen.add(current);
    const found = entries.get(current);
    if (!found) return;
    for (const ref of uses(found).refs) visit(refTarget(ref), [...stack, current]);
  };
  visit(id, []);
  return seen;
}

/** The variables a message needs, through message references (terms take literals). */
export function variablesOf(id, entries) {
  const names = new Set();
  const numeric = new Set();
  for (const reached of reach(id, entries)) {
    if (reached.startsWith("-") && reached !== id) continue;
    const found = entries.get(reached);
    if (!found) continue;
    const u = uses(found);
    for (const name of u.variables) names.add(name);
    for (const name of u.numeric) numeric.add(name);
  }
  return { names, numeric };
}

/** SHA-256 over the message and everything it references, in id order (review #7). */
export function sourceHash(id, entries) {
  const hash = createHash("sha256");
  for (const reached of [...reach(id, entries)].sort()) {
    const found = entries.get(reached);
    hash.update(`${reached}\0${found ? serializer.serializeEntry(found.entry) : "<missing>"}\0`);
  }
  return hash.digest("hex").slice(0, 16);
}

/** `-locale-name` and `-locale-matches` from a locale's `meta.ftl`, as plain text. */
export function localeMeta(locale) {
  const text = (id) => {
    const found = locale.entries.get(id);
    const elements = found?.entry.value?.elements ?? [];
    return elements.every((element) => element.type === "TextElement")
      ? elements.map((element) => element.value).join("").trim()
      : null;
  };
  const name = text("-locale-name");
  const matches = (text("-locale-matches") ?? locale.tag).split(/[\s,]+/).filter(Boolean);
  return { tag: locale.tag, name, matches };
}

/** A shell message's literal text, or why it is not literal. */
export function literalText(found) {
  const entry = found.entry;
  if (entry.attributes.length) return { error: "has attributes" };
  const elements = entry.value?.elements ?? [];
  if (!elements.length) return { error: "has no value" };
  if (!elements.every((element) => element.type === "TextElement")) return { error: "is not literal text" };
  return { text: elements.map((element) => element.value).join("") };
}

export function readHashes(tag, dir = LOCALES_DIR) {
  const file = path.join(dir, tag, HASHES_FILE);
  return fs.existsSync(file) ? JSON.parse(fs.readFileSync(file, "utf8")) : {};
}

/**
 * Every catalog rule (DESIGN rev. 2): `errors` fail the build; `missing` and
 * `stale` are per-locale work lists (the release gate requires both empty).
 */
export function checkCatalog(dir = LOCALES_DIR) {
  const errors = [];
  const tags = localeTags(dir);
  const locales = tags.map((tag) => readLocale(tag, dir));
  for (const locale of locales) errors.push(...locale.errors);
  const source = locales[0];
  const report = {};
  for (const locale of locales) {
    const meta = localeMeta(locale);
    if (!meta.name) errors.push(`${locale.tag}: meta.ftl needs a literal \`-locale-name\``);
    for (const [id, found] of locale.entries) {
      const u = uses(found);
      for (const fn of u.functions) {
        if (!FUNCTIONS.has(fn)) errors.push(`${locale.tag}/${found.file}: \`${id}\` calls ${fn}(); only ${[...FUNCTIONS].join(", ")} are supported`);
      }
      for (const ref of u.refs) {
        const target = refTarget(ref);
        const owner = locale.entries.get(target) ?? (locale === source ? undefined : source.entries.get(target));
        if (!owner) errors.push(`${locale.tag}/${found.file}: \`${id}\` references unknown \`${target}\``);
        else if (ref.includes(".") && !owner.entry.attributes.some((a) => a.id.name === ref.split(".")[1])) {
          errors.push(`${locale.tag}/${found.file}: \`${id}\` references unknown attribute \`${ref}\``);
        }
      }
      reach(id, locale.entries, errors);
      if (found.file === SHELL_FILE) {
        const literal = literalText(found);
        if (literal.error) errors.push(`${locale.tag}/${SHELL_FILE}: \`${id}\` ${literal.error}; native code reads shell messages as literal text`);
      }
    }
  }
  for (const locale of locales.slice(1)) {
    const hashes = readHashes(locale.tag, dir);
    const missing = [];
    const stale = [];
    for (const [id, found] of source.entries) {
      if (id.startsWith("-locale-")) continue;
      if (!locale.entries.has(id)) {
        if (!id.startsWith("-")) missing.push(id);
        continue;
      }
      if (hashes[id] !== sourceHash(id, source.entries)) stale.push(id);
      if (found.entry.type !== "Message") continue;
      const english = variablesOf(id, source.entries);
      const translated = variablesOf(id, locale.entries);
      for (const name of translated.names) {
        if (!english.names.has(name)) errors.push(`${locale.tag}: \`${id}\` uses $${name}, which the English message does not pass`);
      }
      for (const name of translated.numeric) {
        if (!english.numeric.has(name) && english.names.has(name)) {
          // A plural selector in the translation over a variable English only prints.
          errors.push(`${locale.tag}: \`${id}\` selects on $${name} by plural category; English passes it as text (make it numeric in English too)`);
        }
      }
    }
    for (const [id, found] of locale.entries) {
      if (!source.entries.has(id) && !id.startsWith("-locale-")) errors.push(`${locale.tag}/${found.file}: \`${id}\` is not in English (orphan)`);
    }
    report[locale.tag] = { missing, stale };
  }
  return { errors, report, locales };
}
