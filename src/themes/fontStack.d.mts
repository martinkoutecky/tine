export const FONT_STACK_MAX_LENGTH: number;
export const FONT_STACK_MAX_FAMILIES: number;
export const FONT_FAMILY_MAX_LENGTH: number;
export const BLOCK_LINE_HEIGHT_MIN: number;
export const BLOCK_LINE_HEIGHT_MAX: number;
export type FontFamilyKind = "generic" | "system" | "named";
export interface FontFamily { name: string; kind: FontFamilyKind }
export type FontStackResult = { ok: true; families: FontFamily[]; css: string } | { ok: false; error: string };
export function parseFontStack(value: unknown): FontStackResult;
export function isBlockLineHeight(value: unknown): value is number;
