// Shared browser/native fixture: top-level and nested sibling lists both window,
// with loaded images above an editor near the end of a section.
export const gh745Page = () => ({
  name: "GH745 outline", title: "GH745 outline", kind: "page", pre_block: null,
  blocks: Array.from({ length: 10 }, (_, section) => ({
    id: `section-${section}`, raw: `Section ${section}`, collapsed: false,
    children: Array.from({ length: 100 }, (_, row) => ({
      id: `row-${section}-${row}`, collapsed: false, children: [],
      raw: `Section ${section} row ${row} text for structural editing${row === 5 || row === 45 ? "\n![loaded image](../assets/gh745.png)" : ""}`,
    })),
  })),
});

export const gh745Markdown = () => gh745Page().blocks.map((section) =>
  `- ${section.raw}\n${section.children.map((row) => `  - ${row.raw.replaceAll("\n", "\n    ")}\n`).join("")}`
).join("");

// 800 x 600 PNG, real intrinsic dimensions without network resources.
export function gh745Png(zlib) {
  const crc = (data) => {
    let c = 0xffffffff;
    for (const byte of data) {
      c ^= byte;
      for (let i = 0; i < 8; i++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    }
    return (c ^ 0xffffffff) >>> 0;
  };
  const chunk = (name, data) => {
    const body = Buffer.concat([Buffer.from(name), data]);
    const size = Buffer.alloc(4); size.writeUInt32BE(data.length);
    const sum = Buffer.alloc(4); sum.writeUInt32BE(crc(body));
    return Buffer.concat([size, body, sum]);
  };
  const header = Buffer.alloc(13); header.writeUInt32BE(800); header.writeUInt32BE(600, 4); header[8] = 8; header[9] = 2;
  const row = Buffer.alloc(2401, 140); row[0] = 0;
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", header), chunk("IDAT", zlib.deflateSync(Buffer.concat(Array.from({ length: 600 }, () => row)))), chunk("IEND", Buffer.alloc(0))]);
}
