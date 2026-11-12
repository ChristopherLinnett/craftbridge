// Mermaid twins: extract every ```mermaid block of the master and render it with mermaid-cli
// (syntax validation + a PNG to look at). Used by review.mjs (mermaid) and check.mjs (--final).
// Installed by scoping-plan — change it in the skill's kit/, never in a plan.
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { ROOT, COLOR, THEME } from './common.mjs';

export function mermaidBlocks(master) {
  return [...master.matchAll(/```mermaid\n([\s\S]*?)```/g)].map((m, i) => {
    const tag = m[1].match(/^%% @drawio (\S+) \| (.+)$/m);
    return { id: tag ? tag[1] : `untagged-${i + 1}`, caption: tag ? tag[2] : '', code: m[1], line: master.slice(0, m.index).split('\n').length };
  });
}

export function mmdcPath() {
  const p = path.join(ROOT, 'node_modules', '.bin', 'mmdc');
  return fs.existsSync(p) ? p : null;
}

// Renders each block to <outDir>/<id>.png; returns [{ id, line, png, error }].
export function renderMermaid(blocks, outDir) {
  const mmdc = mmdcPath();
  if (!mmdc) return null;
  fs.mkdirSync(outDir, { recursive: true });
  const cfg = path.join(outDir, '.mermaid-config.json');
  fs.writeFileSync(cfg, JSON.stringify({
    theme: 'dark',
    themeVariables: { fontFamily: `${THEME.font.sans}, sans-serif`, background: COLOR.page, primaryColor: COLOR.surface, primaryBorderColor: COLOR.primary, lineColor: COLOR.edge, primaryTextColor: COLOR.textHi },
  }));
  const pcfg = path.join(outDir, '.puppeteer-config.json');
  fs.writeFileSync(pcfg, JSON.stringify({ headless: 'shell' }));
  const out = [];
  for (const b of blocks) {
    const src = path.join(outDir, `.${b.id}.mmd`);
    const png = path.join(outDir, `${b.id}.png`);
    fs.writeFileSync(src, b.code);
    const r = spawnSync(mmdc, ['-i', src, '-o', png, '-c', cfg, '-p', pcfg, '-b', COLOR.page, '-w', '1400', '-s', '1.5', '-q'], { encoding: 'utf8', timeout: 120000 });
    fs.rmSync(src, { force: true });
    const error = r.status !== 0 ? (r.stderr || r.stdout || 'mermaid-cli failed').split('\n').filter((l) => l.trim()).slice(0, 3).join(' · ') : null;
    out.push({ id: b.id, line: b.line, png, error });
  }
  fs.rmSync(cfg, { force: true });
  fs.rmSync(pcfg, { force: true });
  return out;
}

// Twin parity: does the mermaid twin carry the same story as its draw.io diagram? Counted, not
// parsed in full: nodes and edges for flowcharts, participants and messages for sequences.
// A twin that has lost a node or a step (or grown one) is out of date.
export function twinCounts(code) {
  const body = code.split('\n').filter((l) => !/^\s*%%/.test(l));
  const type = (body.find((l) => l.trim()) || '').trim().split(/\s+/)[0];
  if (/^(flowchart|graph)$/.test(type)) {
    const ids = new Set();
    let edges = 0;
    for (const l of body.slice(1)) {
      if (/^\s*(subgraph|end|classDef|class|style|linkStyle|direction)\b/.test(l)) continue;
      const line = l.replace(/"[^"]*"/g, '""');
      for (const m of line.matchAll(/(?:^|[\s&>-])([A-Za-z_][\w]*)\s*(?:\[|\(|\{|>)/g)) ids.add(m[1]);
      edges += (line.match(/-->|---|-\.->|==>|--[^-\s][^>]*-->/g) || []).length;
      for (const m of line.matchAll(/(?:-->|---|-\.->|==>)\s*(?:\|[^|]*\|\s*)?([A-Za-z_][\w]*)/g)) ids.add(m[1]);
    }
    return { type: 'flowchart', nodes: ids.size, edges };
  }
  if (type === 'sequenceDiagram') {
    const parts = new Set();
    let msgs = 0;
    for (const l of body.slice(1)) {
      const p = l.match(/^\s*(?:participant|actor)\s+(\S+)/); if (p) { parts.add(p[1]); continue; }
      const m = l.match(/^\s*([^\s-]+?)\s*(?:-{1,2}>>|-{1,2}>|-{1,2}x|-{1,2}\))\s*([^:\s]+)\s*:/);
      if (m) { msgs++; parts.add(m[1]); parts.add(m[2]); }
    }
    return { type: 'sequence', nodes: parts.size, edges: msgs };
  }
  if (/^stateDiagram/.test(type)) {
    const states = new Set(); let edges = 0;
    for (const l of body.slice(1)) {
      const m = l.match(/^\s*(\S+)\s*-->\s*([^:\s]+)/);
      if (m) { for (const x of [m[1], m[2]]) if (x !== '[*]') states.add(x); if (m[1] !== '[*]' && m[2] !== '[*]') edges++; continue; }
      const d = l.match(/^\s*state\s+(?:"[^"]*"\s+as\s+)?(\w+)/); if (d) states.add(d[1]);
    }
    return { type: 'flowchart', nodes: states.size, edges };
  }
  if (type === 'erDiagram') {
    const ents = new Set(); let edges = 0;
    for (const l of body.slice(1)) {
      const m = l.match(/^\s*([A-Z_][\w]*)\s+[|}o][|o]?--[|o{][|{]?\s+([A-Z_][\w]*)/); if (m) { ents.add(m[1]); ents.add(m[2]); edges++; continue; }
      const e = l.match(/^\s*([A-Z_][\w]*)\s*\{/); if (e) ents.add(e[1]);
    }
    return { type: 'flowchart', nodes: ents.size, edges };
  }
  return null;
}
// Differences worth a look: more than one node/edge and more than a quarter apart.
export function parityProblems(twin, drawio) {
  if (!twin || !drawio || twin.type !== drawio.type) return [];
  const out = [];
  for (const [k, what] of [['nodes', twin.type === 'sequence' ? 'lanes' : 'nodes'], ['edges', twin.type === 'sequence' ? 'messages' : 'edges']]) {
    const d = Math.abs(twin[k] - drawio[k]);
    if (d > 1 && d > 0.25 * Math.max(twin[k], drawio[k])) out.push(`the mermaid twin has ${twin[k]} ${what}, the draw.io diagram ${drawio[k]} — the twins tell different stories`);
  }
  return out;
}
export function drawioCounts(xml) {
  const cells = [...xml.matchAll(/<mxCell\b([^>]*?)\/?>/g)].map((m) => Object.fromEntries([...m[1].matchAll(/([\w:-]+)="([^"]*)"/g)].map((x) => [x[1], x[2]])));
  const heads = cells.filter((c) => /^h_/.test(c.id || '')).length;
  if (heads) return { type: 'sequence', nodes: heads, edges: cells.filter((c) => /^m\d+$/.test(c.id || '') || /^s\d+$/.test(c.id || '')).length };
  const nodes = cells.filter((c) => c.vertex === '1' && !/^(lg_|t\d)/.test(c.id) && !(c.style || '').startsWith('text;') && !(c.style || '').includes('absoluteArcSize=1') && !((c.style || '').includes('arcSize=3;') && (c.style || '').includes('spacingLeft=14'))).length;
  return { type: 'flowchart', nodes, edges: cells.filter((c) => c.edge === '1' && c.source && c.target).length };
}
