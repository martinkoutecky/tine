export type PluginGrammarKind = "packageId" | "contributionId" | "entry";
export interface PluginGrammarRule {
  type: "string";
  pattern: string;
  maxLength?: number;
  not?: { pattern: string };
}
export const pluginGrammar: Record<PluginGrammarKind, PluginGrammarRule>;
export function parsePluginGrammar(value: unknown, kind: PluginGrammarKind): string | null;
