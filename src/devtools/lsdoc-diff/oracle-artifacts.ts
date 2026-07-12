import { canon } from "./vendor/compare.mjs";
import type { Projection } from "./mldoc-client";

interface Comparison {
  matches: boolean;
  shifts: number;
}

type JsonObject = Record<string, unknown>;

/**
 * Recognize the exact issue #82 artifact caused by mldoc's leaked
 * `end_string("``")` rolling window. This deliberately does not treat arbitrary
 * code-span differences as equivalent: the only accepted delta moves one
 * literal backtick from the start of a Code node to the preceding Plain node.
 */
export function isMldocBacktickStateArtifact(lsdoc: Projection, mldoc: Projection): boolean {
  const left = canon({ blocks: lsdoc.blocks, refs: lsdoc.refs });
  const right = canon({ blocks: mldoc.blocks, refs: mldoc.refs });
  const result = compare(left, right);
  return result.matches && result.shifts > 0;
}

export function shouldQuarantineMldocBacktickStateArtifact(
  freshRangeParsesAgree: boolean,
  lsdoc: Projection,
  mldoc: Projection,
): boolean {
  return freshRangeParsesAgree && isMldocBacktickStateArtifact(lsdoc, mldoc);
}

/** Locate the lsdoc block that contains the already-classified ownership shift. */
export function mldocBacktickArtifactSourceSpan(
  lsdoc: Projection,
  mldoc: Projection,
): [number, number] | null {
  if (!isMldocBacktickStateArtifact(lsdoc, mldoc)) return null;
  const mldocPairs = new Set<string>();
  collectPlainCodePairs(canon(mldoc.blocks), mldocPairs);
  return findLsdocShiftSpan(lsdoc.blocks, mldocPairs, null);
}

function compare(left: unknown, right: unknown): Comparison {
  if (Object.is(left, right)) return { matches: true, shifts: 0 };
  if (Array.isArray(left) || Array.isArray(right)) {
    if (!Array.isArray(left) || !Array.isArray(right) || left.length !== right.length) {
      return { matches: false, shifts: 0 };
    }
    let shifts = 0;
    for (let i = 0; i < left.length; i++) {
      if (i + 1 < left.length && isBacktickOwnershipShift(left[i], left[i + 1], right[i], right[i + 1])) {
        shifts++;
        i++;
        continue;
      }
      const item = compare(left[i], right[i]);
      if (!item.matches) return item;
      shifts += item.shifts;
    }
    return { matches: true, shifts };
  }
  if (isObject(left) || isObject(right)) {
    if (!isObject(left) || !isObject(right)) return { matches: false, shifts: 0 };
    const leftKeys = Object.keys(left);
    const rightKeys = Object.keys(right);
    if (leftKeys.length !== rightKeys.length || leftKeys.some((key) => !Object.hasOwn(right, key))) {
      return { matches: false, shifts: 0 };
    }
    let shifts = 0;
    for (const key of leftKeys) {
      const field = compare(left[key], right[key]);
      if (!field.matches) return field;
      shifts += field.shifts;
    }
    return { matches: true, shifts };
  }
  return { matches: false, shifts: 0 };
}

function isBacktickOwnershipShift(
  lsdocPlain: unknown,
  lsdocCode: unknown,
  mldocPlain: unknown,
  mldocCode: unknown,
): boolean {
  if (![lsdocPlain, lsdocCode, mldocPlain, mldocCode].every(isTextInline)) return false;
  return lsdocPlain.k === "plain"
    && lsdocCode.k === "code"
    && mldocPlain.k === "plain"
    && mldocCode.k === "code"
    && mldocPlain.text === `${lsdocPlain.text}\``
    && lsdocCode.text === `\`${mldocCode.text}`;
}

function isTextInline(value: unknown): value is { k: string; text: string } {
  return isObject(value)
    && Object.keys(value).length === 2
    && typeof value.k === "string"
    && typeof value.text === "string";
}

function isObject(value: unknown): value is JsonObject {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function collectPlainCodePairs(value: unknown, pairs: Set<string>): void {
  if (Array.isArray(value)) {
    for (let i = 0; i + 1 < value.length; i++) {
      if (isTextInline(value[i]) && isTextInline(value[i + 1]) && value[i].k === "plain" && value[i + 1].k === "code") {
        pairs.add(pairKey(value[i].text, value[i + 1].text));
      }
    }
    value.forEach((item) => collectPlainCodePairs(item, pairs));
  } else if (isObject(value)) {
    Object.values(value).forEach((item) => collectPlainCodePairs(item, pairs));
  }
}

function findLsdocShiftSpan(
  value: unknown,
  mldocPairs: Set<string>,
  ownerSpan: [number, number] | null,
): [number, number] | null {
  if (Array.isArray(value)) {
    for (let i = 0; i + 1 < value.length; i++) {
      const plain = value[i];
      const code = value[i + 1];
      if (
        ownerSpan
        && isRawTextInline(plain)
        && isRawTextInline(code)
        && plain.k === "plain"
        && code.k === "code"
        && code.text.startsWith("`")
        && mldocPairs.has(pairKey(`${plain.text}\``, code.text.slice(1)))
      ) return ownerSpan;
    }
    for (const item of value) {
      const found = findLsdocShiftSpan(item, mldocPairs, ownerSpan);
      if (found) return found;
    }
  } else if (isObject(value)) {
    const own = blockSpan(value) ?? ownerSpan;
    for (const item of Object.values(value)) {
      const found = findLsdocShiftSpan(item, mldocPairs, own);
      if (found) return found;
    }
  }
  return null;
}

function blockSpan(value: JsonObject): [number, number] | null {
  const span = value.span;
  return typeof value.kind === "string"
    && Array.isArray(span)
    && span.length === 2
    && Number.isInteger(span[0])
    && Number.isInteger(span[1])
    ? [span[0] as number, span[1] as number]
    : null;
}

function isRawTextInline(value: unknown): value is { k: string; text: string } {
  return isObject(value) && typeof value.k === "string" && typeof value.text === "string";
}

const pairKey = (plain: string, code: string): string => `${plain.length}:${plain}${code}`;
