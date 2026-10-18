// Webfonts → plain sfnt (TTF/OTF) files that glot's Rust measurer (ttf-parser/rustybuzz) can read.
// Only metrics matter there, so WOFF2's transformed glyf/loca (outlines) are dropped, not rebuilt.
import { brotliDecompressSync, inflateSync } from "node:zlib";
import { createHash } from "node:crypto";

const tagOf = (b: Buffer, o: number) => b.toString("latin1", o, o + 4);

const KNOWN = ["cmap", "head", "hhea", "hmtx", "maxp", "name", "OS/2", "post", "cvt ", "fpgm", "glyf", "loca", "prep", "CFF ",
  "VORG", "EBDT", "EBLC", "gasp", "hdmx", "kern", "LTSH", "PCLT", "VDMX", "vhea", "vmtx", "BASE", "GDEF", "GPOS", "GSUB",
  "EBSC", "JSTF", "MATH", "CBDT", "CBLC", "COLR", "CPAL", "SVG ", "sbix", "acnt", "avar", "bdat", "bloc", "bsln", "cvar",
  "fdsc", "feat", "fmtx", "fvar", "gvar", "hsty", "just", "lcar", "mort", "morx", "opbd", "prop", "trak", "Zapf", "Silf",
  "Glat", "Gloc", "Feat", "Sill"];

function writeSfnt(flavor: number, tables: Map<string, Buffer>): Buffer {
  const tags = [...tables.keys()].sort();
  const n = tags.length;
  const pow = 2 ** Math.floor(Math.log2(n));
  const head = Buffer.alloc(12 + 16 * n);
  head.writeUInt32BE(flavor, 0);
  head.writeUInt16BE(n, 4);
  head.writeUInt16BE(pow * 16, 6);
  head.writeUInt16BE(Math.log2(pow), 8);
  head.writeUInt16BE(n * 16 - pow * 16, 10);
  const parts: Buffer[] = [head];
  let off = head.length;
  tags.forEach((tag, i) => {
    const data = tables.get(tag)!;
    head.write(tag, 12 + 16 * i, "latin1");
    head.writeUInt32BE(0, 16 + 16 * i); // checksum: not verified by readers we target
    head.writeUInt32BE(off, 20 + 16 * i);
    head.writeUInt32BE(data.length, 24 + 16 * i);
    const padded = Buffer.alloc((data.length + 3) & ~3);
    data.copy(padded);
    parts.push(padded);
    off += padded.length;
  });
  return Buffer.concat(parts);
}

function woff1(b: Buffer): Buffer {
  const n = b.readUInt16BE(12);
  const tables = new Map<string, Buffer>();
  for (let i = 0; i < n; i++) {
    const e = 44 + 20 * i;
    const [off, comp, orig] = [b.readUInt32BE(e + 4), b.readUInt32BE(e + 8), b.readUInt32BE(e + 12)];
    const raw = b.subarray(off, off + comp);
    tables.set(tagOf(b, e), comp < orig ? inflateSync(raw) : Buffer.from(raw));
  }
  return writeSfnt(b.readUInt32BE(4), tables);
}

function woff2(b: Buffer): Buffer {
  const flavor = b.readUInt32BE(4);
  if (flavor === 0x74746366) throw new Error("WOFF2 font collections are not supported");
  const n = b.readUInt16BE(12);
  const compressed = b.readUInt32BE(20);
  let p = 48;
  const base128 = () => {
    let v = 0;
    for (let i = 0; i < 5; i++) {
      const x = b[p++];
      v = v * 128 + (x & 0x7f);
      if (!(x & 0x80)) return v;
    }
    throw new Error("bad UIntBase128");
  };
  const dir: { tag: string; transformed: boolean; length: number }[] = [];
  for (let i = 0; i < n; i++) {
    const flags = b[p++];
    let tag: string;
    if ((flags & 0x3f) === 63) { tag = tagOf(b, p); p += 4; } else tag = KNOWN[flags & 0x3f];
    const version = flags >> 6;
    const orig = base128();
    const transformed = tag === "glyf" || tag === "loca" ? version !== 3 : version !== 0;
    dir.push({ tag, transformed, length: transformed ? base128() : orig });
  }
  const data = brotliDecompressSync(b.subarray(p, p + compressed));
  const tables = new Map<string, Buffer>();
  const transformedHmtx: Buffer[] = [];
  let off = 0;
  for (const t of dir) {
    const raw = data.subarray(off, off + t.length);
    off += t.length;
    if (t.transformed && (t.tag === "glyf" || t.tag === "loca")) continue; // outlines: not needed to measure
    if (t.transformed && t.tag === "hmtx") { transformedHmtx.push(raw); continue; }
    tables.set(t.tag, Buffer.from(raw));
  }
  if (transformedHmtx.length) {
    // Transformed hmtx: advances kept, side bearings may be omitted (rebuilt as 0: irrelevant for widths).
    const t = transformedHmtx[0];
    const numH = tables.get("hhea")!.readUInt16BE(34);
    const numGlyphs = tables.get("maxp")!.readUInt16BE(4);
    const flags = t[0];
    const out = Buffer.alloc(numH * 4 + (numGlyphs - numH) * 2);
    let q = 1;
    for (let i = 0; i < numH; i++, q += 2) out.writeUInt16BE(t.readUInt16BE(q), i * 4);
    if (!(flags & 1)) for (let i = 0; i < numH; i++, q += 2) out.writeInt16BE(t.readInt16BE(q), i * 4 + 2);
    if (!(flags & 2)) for (let i = 0; i < numGlyphs - numH; i++, q += 2) out.writeInt16BE(t.readInt16BE(q), numH * 4 + i * 2);
    tables.set("hmtx", out);
  }
  return writeSfnt(flavor, tables);
}

/** Any font response body → sfnt bytes, or undefined if it isn't a font we can read. */
export function toSfnt(b: Buffer): Buffer | undefined {
  if (b.length < 12) return;
  const sig = tagOf(b, 0);
  if (sig === "wOF2") return woff2(b);
  if (sig === "wOFF") return woff1(b);
  if (sig === "OTTO" || sig === "true" || sig === "ttcf" || b.readUInt32BE(0) === 0x00010000) return b;
}

/** Stable file name for a font body: same font, same file, whatever URL hash the bundler used. */
export const fontFileName = (sfnt: Buffer) =>
  createHash("sha256").update(sfnt).digest("hex").slice(0, 16) + (tagOf(sfnt, 0) === "OTTO" ? ".otf" : ".ttf");
