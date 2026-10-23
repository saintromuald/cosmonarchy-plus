// Usage: node implode.mjs <in> <out>
// Decompress a PKWARE implode blob (without the 0x08 type byte) using implode-decoder.
import fs from 'fs';
import concat from 'concat-stream';
import { Readable } from 'stream';
import dec from '/tmp/opencode/ums-port/tools/node_modules/implode-decoder/index.js';
const src = process.argv[2], dst = process.argv[3];
const buf = fs.readFileSync(src);
const rs = new Readable({ read() { this.push(buf); this.push(null); } });
rs.pipe(dec.createDecoderStream ? dec.createDecoderStream() : dec()).pipe(concat(d => {
  fs.writeFileSync(dst, d);
  console.error(`implode: ${buf.length} -> ${d.length}`);
}));
