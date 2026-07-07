export type TsvMatrix = string[][];
export type DelimitedKind = "csv" | "tsv";

function lines(text: string): string[] {
  const normalized = text.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
  const out = normalized.split("\n");
  if (out.length > 1 && out[out.length - 1] === "") out.pop();
  return out;
}

function parseTsv(text: string): TsvMatrix {
  return lines(text).map((line) => line.split("\t"));
}

function parseCsv(text: string): TsvMatrix {
  const rows: string[][] = [];
  let row: string[] = [];
  let field = "";
  let quoted = false;

  const pushField = () => {
    row.push(field);
    field = "";
  };
  const pushRow = () => {
    pushField();
    rows.push(row);
    row = [];
  };

  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (quoted) {
      if (ch === '"') {
        if (text[i + 1] === '"') {
          field += '"';
          i++;
        } else {
          quoted = false;
        }
      } else {
        field += ch;
      }
      continue;
    }
    if (ch === '"' && field === "") {
      quoted = true;
      continue;
    }
    if (ch === ",") {
      pushField();
      continue;
    }
    if (ch === "\r") {
      if (text[i + 1] === "\n") i++;
      pushRow();
      continue;
    }
    if (ch === "\n") {
      pushRow();
      continue;
    }
    field += ch;
  }

  if (field !== "" || row.length > 0 || text.length === 0 || !/[\r\n]$/.test(text)) pushRow();
  return rows;
}

export function looksLikeDelimitedText(text: string): boolean {
  if (text.includes("\t")) return true;
  if (!text.includes(",")) return false;
  const matrix = parseCsv(text);
  if (!matrix.some((row) => row.length > 1)) return false;
  if (text.includes("\n") || text.includes("\r") || text.includes('"')) return true;
  return /,(?=\S)/.test(text);
}

export function parseDelimitedText(text: string, kind?: DelimitedKind): TsvMatrix {
  if (kind === "tsv") return parseTsv(text);
  if (kind === "csv") return parseCsv(text);
  return text.includes("\t") ? parseTsv(text) : parseCsv(text);
}

export function serializeTsv(rows: readonly (readonly (string | null | undefined)[])[]): string {
  return rows
    .map((row) => row.map((cell) => String(cell ?? "").replace(/[\t\r\n]+/g, " ")).join("\t"))
    .join("\n");
}
