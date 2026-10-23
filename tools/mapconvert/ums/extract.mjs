// Extract scenario.chk from a SC map (MPQ) via scm-extractor stream.
import fs from 'fs';
import createExtractor from 'scm-extractor';
import concat from 'concat-stream';
import { Readable } from 'stream';

const map = process.argv[2];
const out = process.argv[3];
const buf = fs.readFileSync(map);
const rs = new Readable({ read() { this.push(buf); this.push(null); } });
rs.pipe(createExtractor()).pipe(concat(data => {
  fs.writeFileSync(out, data);
  console.log(`wrote ${out}: ${data.length} bytes, magic ${String.fromCharCode(...data.slice(0,4))}`);
}));
