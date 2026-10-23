// analyze.mjs <mapfile|chkfile>
// Static analysis of a StarCraft map for the Cosmonarchy UMS port pipeline.
// Outputs JSON: title, version, sections, forces, triggers (unit IDs, EUD flags),
// text encoding, tileset, size.
import fs from 'fs';
import path from 'path';
import createExtractor from 'scm-extractor';
import concat from 'concat-stream';
import { Readable } from 'stream';
import Chk from 'bw-chk';

const file = process.argv[2];
const buf = fs.readFileSync(file);

if (process.argv[3] === '--strings') {
  const chk = new Chk(buf, { encoding: 'auto' });
  const n = chk._strings._amount || 0;
  for (let i = 1; i <= n; i++) {
    const s = chk._strings.get(i);
    if (s) console.log(i + '\t' + s.replace(/[\r\n\t]/g, ' '));
  }
  process.exit(0);
}

function analyze(chkBuf, sourceName) {
  const chk = new Chk(chkBuf, { encoding: 'auto' });
  const result = {
    source: sourceName,
    title: chk.title,
    description: (chk.description || '').slice(0, 120),
    size: chk.size,
    tileset: chk.tilesetName,
    encoding: chk.encoding,
    forces: chk.forces.map(f => f.players.length ? {
      name: f.name, flags: f.flags,
      players: f.players.map(p => ({ id: p.id, type: p.type, race: p.race })),
    } : null).filter(Boolean),
    ver: null,
    hasSTRx: false,
    triggers: null,
    unitCount: chk.units.length,
    strings: chk.strings ? chk.strings.size : null,
  };

  // sections (VER and STRx read from raw CHK; bw-chk doesn't retain VER)
  let pos = 0;
  let ver = null, gameType = null, hasSTRx = false;
  const sections = [];
  while (pos + 8 <= chkBuf.length) {
    const name = chkBuf.toString('latin1', pos, pos + 4);
    const size = chkBuf.readUInt32LE(pos + 4);
    if (size > 0x4000000) break;
    sections.push({ name, size });
    if (name === 'VER ' && size >= 2) {
      ver = chkBuf.readUInt16LE(pos + 8);
      gameType = size >= 3 ? chkBuf.readUInt8(pos + 10) : null;
    }
    if (name === 'STRx') hasSTRx = true;
    pos += 8 + size;
  }
  result.ver = ver;
  result.gameType = gameType;
  result.hasSTRx = hasSTRx;
  result.sections = sections.map(s => s.name.trimEnd());

  // triggers
  let trigCount = 0, conds = 0, acts = 0;
  const unitIds = new Set();
  const actionKinds = new Map();
  const condKinds = new Map();
  let eudConds = 0, eudActs = 0, comments = 0;
  for (const t of chk.triggers()) {
    trigCount++;
    for (const c of t.allConditions()) {
      const raw = c.rawByteView();
      conds++;
      condKinds.set(c.id(), (condKinds.get(c.id()) || 0) + 1);
      // EUD condition: internal data (bytes 18-19) nonzero
      if (raw.readUInt16LE(18) !== 0) eudConds++;
      const p = c.params();
      if (p.unitId !== undefined) unitIds.add(p.unitId);
    }
    for (const a of t.allActions()) {
      acts++;
      actionKinds.set(a.id(), (actionKinds.get(a.id()) || 0) + 1);
      const p = a.params();
      if (p.unitId !== undefined) unitIds.add(p.unitId);
      // EUD write: SetDeaths (45) with unitId 0 and number 1..3
      if (a.id() === 45 && p.unitId === 0 && p.unitAmount >= 1 && p.unitAmount <= 3) eudActs++;
      if (p.text && typeof p.text === 'string' && /^#|^trigger comment|^\s*spawn\s|condition\s/i.test(p.text.trim())) comments++;
    }
  }
  result.triggers = {
    count: trigCount, conditions: conds, actions: acts,
    unitIds: [...unitIds].sort((a, b) => a - b),
    actionKinds: Object.fromEntries([...actionKinds.entries()].sort((a, b) => a[0] - b[0])),
    condKinds: Object.fromEntries([...condKinds.entries()].sort((a, b) => a[0] - b[0])),
    eudConditions: eudConds, eudActions: eudActs, commentActions: comments,
  };
  return result;
}

function finish(chkBuf, sourceName) {
  try {
    const r = analyze(chkBuf, sourceName);
    console.log(JSON.stringify(r));
  } catch (e) {
    console.log(JSON.stringify({ source: sourceName, error: e.message }));
  }
}

if (file.toLowerCase().endsWith('.chk')) {
  finish(buf, path.basename(file));
} else {
  const rs = new Readable({ read() { this.push(buf); this.push(null); } });
  rs.pipe(createExtractor()).pipe(concat(d => finish(d, path.basename(file))));
}
