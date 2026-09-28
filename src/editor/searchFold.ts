// Search comparison policy shared by frontend previews and the native search grammar.
// Kept Mn ranges have canonical combining class 0, 8, 9, 84, 91, 103, 118, 129, 130 or 132.
// Generated from Unicode category/combining data; native search is authoritative.
const keptMarkRanges = "34f 7a6-7b0 900-902 93a 941-948 94d 955-957 962-963 981 9c1-9c4 9cd 9e2-9e3 a01-a02 a41-a42 a47-a48 a4b-a4d a51 a70-a71 a75 a81-a82 ac1-ac5 ac7-ac8 acd ae2-ae3 afa-aff b01 b3f b41-b44 b4d b55-b56 b62-b63 b82 bc0 bcd c00 c04 c3e-c40 c46-c48 c4a-c4d c55-c56 c62-c63 c81 cbf cc6 ccc-ccd ce2-ce3 d00-d01 d3b-d3c d41-d44 d4d d62-d63 d81 dca dd2-dd4 dd6 e31 e34-e3a e47 e4c-e4e eb1 eb4-ebc ecc-ecd f71-f7e f80-f81 f84 f8d-f97 f99-fbc 102d-1030 1032-1036 1039-103a 103d-103e 1058-1059 105e-1060 1071-1074 1082 1085-1086 109d 1712-1714 1732-1733 1752-1753 1772-1773 17b4-17b5 17b7-17bd 17c6 17c9-17d3 180b-180d 180f 1885-1886 1920-1922 1927-1928 1932 1a1b 1a56 1a58-1a5e 1a60 1a62 1a65-1a6c 1a73-1a74 1b00-1b03 1b36-1b3a 1b3c 1b42 1b80-1b81 1ba2-1ba5 1ba8-1ba9 1bab-1bad 1be8-1be9 1bed 1bef-1bf1 1c2c-1c33 1c36 2d7f 3099-309a a802 a806 a80b a825-a826 a82c a8c4-a8c5 a8ff a926-a92a a947-a951 a980-a982 a9b6-a9b9 a9bc-a9bd a9e5 aa29-aa2e aa31-aa32 aa35-aa36 aa43 aa4c aa7c aaec-aaed aaf6 abe5 abe8 abed fe00-fe0f 10a01-10a03 10a05-10a06 10a0c 10a0e 10a3f 11001 11038-11046 11070 11073-11074 1107f-11081 110b3-110b6 110b9 110c2 11127-1112b 1112d-11134 11180-11181 111b6-111be 111c9 111cb-111cc 111cf 1122f-11231 11234 11237 1123e 112df 112e3-112e8 112ea 11300-11301 11340 11438-1143f 11442-11444 114b3-114b8 114ba 114bf-114c0 114c2 115b2-115b5 115bc-115bd 115bf 115dc-115dd 11633-1163a 1163d 1163f-11640 116ab 116ad 116b0-116b5 1171d-1171f 11722-11725 11727-1172b 1182f-11837 11839 1193b-1193c 1193e 119d4-119d7 119da-119db 119e0 11a01-11a0a 11a33-11a38 11a3b-11a3e 11a47 11a51-11a56 11a59-11a5b 11a8a-11a96 11a98-11a99 11c30-11c36 11c38-11c3d 11c3f 11c92-11ca7 11caa-11cb0 11cb2-11cb3 11cb5-11cb6 11d31-11d36 11d3a 11d3c-11d3d 11d3f-11d41 11d43-11d45 11d47 11d90-11d91 11d95 11d97 11ef3-11ef4 16f4f 16f8f-16f92 16fe4 1bc9d 1cf00-1cf2d 1cf30-1cf46 1da00-1da36 1da3b-1da6c 1da75 1da84 1da9b-1da9f 1daa1-1daaf e0100-e01ef".split(" ").map((range) => range.split("-").map((hex) => parseInt(hex, 16)));
const mn = /\p{Mn}/u;
const ignorable = /[\u034f\u17b4-\u17b5\u180b-\u180d\u180f\ufe00-\ufe0f\u{e0100}-\u{e01ef}]/u;
const cyrillic = /[\u0400-\u052f\u1c80-\u1c8f\u2de0-\u2dff\ua640-\ua69f]/u;
const strokes: Record<string, string> = { ł: "l", ø: "o", đ: "d", ħ: "h", ŧ: "t" };

function keepMark(char: string): boolean {
  const code = char.codePointAt(0)!;
  let lo = 0, hi = keptMarkRanges.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (keptMarkRanges[mid][0] <= code) lo = mid + 1;
    else hi = mid;
  }
  if (!lo) return false;
  const [first, last] = keptMarkRanges[lo - 1];
  return code >= first && code <= (last ?? first);
}

export function searchFold(value: string, removeAccents = true): string {
  if (/^[\x00-\x7f]*$/.test(value)) return value.toLowerCase();
  const lowered = value.toLowerCase();
  if (!removeAccents) return lowered.normalize("NFKC");
  let out = "";
  let base = "";
  for (const scalar of lowered.normalize("NFKD")) {
    const char = strokes[scalar] ?? scalar;
    const mark = mn.test(char);
    if (!mark || (!ignorable.test(char) && (keepMark(char)
      || (cyrillic.test(base) && !(base === "е" && char === "\u0308"))))) out += char;
    if (!mark) base = char;
  }
  return out.normalize("NFC");
}
