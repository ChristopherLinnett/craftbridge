// scoping-plan draw.io diagram kit — the ONE place diagram styling lives.
// Every diagram in diagrams/src/*.mjs builds its cells through this kit, so palette,
// typography and edge styling stay identical across a plan (and across plans).
// Colours come from theme.json (C below); never pass a raw hex that isn't a theme token.
//
// A diagram module:
//   import { diagram } from '../../build/drawio-kit.mjs';
//   const id = 'd01-journey';
//   export default { id, xml: diagram({ id, name: 'Journey', width: 1400, height: 520 }, (d) => { … }) };
//
// Build-time lint (printed by build-diagrams.mjs, errors fail the build): partial node overlaps,
// nodes spilling out of their group, cells outside the page, unknown edge endpoints, duplicate
// keys, fonts below 9.5 px, and colours that are not theme tokens. Rendered checks (label
// overflow, edges through nodes, label collisions) run in review.mjs.
import { COLOR, TOKEN_HEXES, THEME } from './lib/common.mjs';

export const C = {
  page: COLOR.page,
  card: COLOR.card,
  surface: COLOR.surface,
  border: COLOR.border,
  borderStrong: COLOR.borderStrong,
  primary: COLOR.primary,
  magenta: COLOR.magenta,
  lilac: COLOR.lilac,
  info: COLOR.info,
  success: COLOR.success,
  warn: COLOR.warn,
  amber: COLOR.amber,
  textHi: COLOR.textHi,
  textMid: COLOR.textMid,
  textLow: COLOR.textLow,
  edge: COLOR.edge,
};

const FONT = `fontFamily=${THEME.font.sans};`;
const MONO = `${THEME.font.mono},ui-monospace,monospace`;

// Node kinds. "new/mod/chk/none" double as the change-type legend used by
// architecture and module diagrams.
const KIND = {
  neutral: `fillColor=${C.surface};strokeColor=${C.borderStrong};fontColor=${C.textMid};`,
  quiet: `fillColor=${C.card};strokeColor=${C.border};fontColor=${C.textLow};`,
  new: `fillColor=${C.primary};strokeColor=${C.primary};fontColor=${C.page};`,
  mod: `fillColor=${C.primary};fillOpacity=16;strokeColor=${C.primary};fontColor=${C.textHi};`,
  chk: `fillColor=${C.surface};strokeColor=${C.info};dashed=1;dashPattern=4 3;fontColor=${C.textMid};`,
  none: `fillColor=${C.card};strokeColor=${C.border};fontColor=${C.textLow};`,
  magenta: `fillColor=${C.magenta};strokeColor=${C.magenta};fontColor=#ffffff;`,
  lilac: `fillColor=${C.lilac};strokeColor=${C.lilac};fontColor=#ffffff;`,
  info: `fillColor=${C.info};strokeColor=${C.info};fontColor=${C.page};`,
  success: `fillColor=${C.success};strokeColor=${C.success};fontColor=#ffffff;`,
  warn: `fillColor=${C.warn};fillOpacity=18;strokeColor=${C.warn};fontColor=${C.textHi};`,
  amber: `fillColor=${C.amber};fillOpacity=18;strokeColor=${C.amber};fontColor=${C.textHi};`,
};
export const KINDS = Object.keys(KIND);

const EDGE = {
  default: `strokeColor=${C.edge};fontColor=${C.textMid};`,
  accent: `strokeColor=${C.primary};fontColor=${C.primary};dashed=1;dashPattern=6 4;`,
  primary: `strokeColor=${C.primary};fontColor=${C.primary};`,
  info: `strokeColor=${C.info};fontColor=${C.info};dashed=1;dashPattern=4 3;`,
  quiet: `strokeColor=${C.borderStrong};fontColor=${C.textLow};dashed=1;dashPattern=3 3;`,
  warn: `strokeColor=${C.warn};fontColor=${C.warn};dashed=1;dashPattern=6 4;`,
  success: `strokeColor=${C.success};fontColor=${C.success};`,
};
export const EDGE_KINDS = Object.keys(EDGE);

const esc = (s) => String(s)
  .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

// HTML label: bold-ish main line + optional small sub line(s). Callers pass
// plain text; the kit owns all markup so labels are always escaped once.
function label(main, sub, { subColor, mono } = {}) {
  const m = esc(main).replace(/\n/g, '<br>');
  if (!sub) return m;
  // Monospace field lists keep their column padding: HTML labels collapse repeated spaces.
  const s = (mono ? esc(sub).replace(/ {2,}/g, (m) => '&nbsp;'.repeat(m.length)) : esc(sub)).replace(/\n/g, '<br>');
  const ff = mono ? `font-family:${MONO};` : '';
  return `${m}<br><span style="font-size:10.5px;font-weight:400;${ff}color:${subColor || 'inherit'};opacity:.85">${s}</span>`;
}

// Lint results per diagram id, read by build-diagrams.mjs after each module is imported.
export const LINT = new Map();

function lint(id, width, height, geo, edges, cells, meta = {}) {
  const errors = [];
  const warnings = [];
  const keys = new Map();
  for (const g of geo) {
    if (keys.has(g.key)) errors.push(`duplicate key "${g.key}"`);
    keys.set(g.key, g);
    if (g.x < 0 || g.y < 0 || g.x + g.w > width + 1 || g.y + g.h > height + 1) {
      warnings.push(`"${g.key}" sits outside the ${width}×${height} page (x ${g.x}, y ${g.y}, w ${g.w}, h ${g.h})`);
    }
    if (g.size != null && g.size < 9.5) warnings.push(`"${g.key}" font ${g.size}px is below the 9.5px minimum`);
  }
  for (const e of edges) {
    for (const end of [e.from, e.to]) if (!keys.has(end)) errors.push(`edge ${e.key} points at unknown node "${end}"`);
  }
  // Every cell id must be unique — including the ids the kit generates for text (t<n>) and edges (e<n>).
  const ids = cells.map((c) => (c.match(/^<mxCell id="([^"]+)"/) || [])[1]);
  const seen = new Set();
  for (const x of ids) { if (seen.has(x)) errors.push(`cell id "${x}" is used twice — keys like t<n> / e<n> collide with kit-generated ids; use descriptive keys`); seen.add(x); }
  const boxes = geo.filter((g) => g.type === 'node');
  const inter = (a, b) => Math.max(0, Math.min(a.x + a.w, b.x + b.w) - Math.max(a.x, b.x)) * Math.max(0, Math.min(a.y + a.h, b.y + b.h) - Math.max(a.y, b.y));
  const inside = (a, b) => a.x >= b.x - 0.5 && a.y >= b.y - 0.5 && a.x + a.w <= b.x + b.w + 0.5 && a.y + a.h <= b.y + b.h + 0.5;
  for (let i = 0; i < boxes.length; i++) {
    for (let j = i + 1; j < boxes.length; j++) {
      const a = boxes[i]; const b = boxes[j];
      if (inter(a, b) > 4 && !inside(a, b) && !inside(b, a)) errors.push(`nodes "${a.key}" and "${b.key}" overlap`);
    }
  }
  // Slack: a group (or a row of equal-height groups, which share one bottom on purpose) ends within
  // ~40 px of what it holds. Edge waypoints count as content: a loop routed under a row needs room.
  const groups = geo.filter((x) => x.type === 'group');
  const contentBottom = (g) => Math.max(-Infinity,
    ...geo.filter((x) => x !== g && x.type !== 'group' && inside(x, g)).map((k) => k.y + k.h),
    ...edges.flatMap((e) => e.points || []).filter(([x, y]) => x >= g.x && x <= g.x + g.w && y >= g.y && y <= g.y + g.h).map(([, y]) => y + 24));
  for (const g of groups) {
    for (const b of boxes) if (inter(b, g) > 4 && !inside(b, g)) warnings.push(`node "${b.key}" spills out of group "${g.key}"`);
    const row = groups.filter((o) => Math.abs(o.y - g.y) < 2 && Math.abs(o.h - g.h) < 2);
    const bottom = Math.max(...row.map(contentBottom));
    if (bottom > -Infinity) {
      const gap = g.y + g.h - bottom;
      if (gap > 48) warnings.push(`group "${g.key}"${row.length > 1 ? ' (and its row)' : ''} has ${Math.round(gap)}px of empty space below its content — size groups to what they hold (≤ 40 px)`);
    }
  }
  // Colour carries meaning only with a legend that names every kind used.
  const used = new Set(boxes.filter((b) => b.kind && !/^lg_/.test(b.key) && !/^h_/.test(b.key) && !['neutral', 'quiet'].includes(b.kind)).map((b) => b.kind));
  if (meta.legend) {
    for (const k of used) if (!meta.legend.items.includes(k)) warnings.push(`kind "${k}" is used but the legend does not name it — add it to d.legend({ items })`);
    for (const [k, n] of Object.entries(meta.legend.names)) if (n.length > 16) warnings.push(`legend label "${n}" (${k}) wraps in its pill — keep it to 16 characters`);
  } else if (used.size >= 2) warnings.push(`${used.size} colour kinds (${[...used].join(', ')}) carry meaning but there is no legend — add d.legend({ x, y, items, names })`);
  // Waypoints on the outer margin mean the edge is being routed around the diagram, not through it.
  for (const e of edges) for (const [x, y] of e.points || []) {
    if (x < 36 || x > width - 36 || y > height - 24) { warnings.push(`edge ${e.from}→${e.to} has a waypoint on the page margin (${x}, ${y}) — re-layout instead of routing around the outside`); break; }
  }
  const hexes = cells.join('\n').match(/#[0-9a-fA-F]{6}\b|#[0-9a-fA-F]{3}\b/g) || [];
  for (const h of new Set(hexes.map((x) => x.toLowerCase()))) if (!TOKEN_HEXES.has(h)) errors.push(`colour ${h} is not a theme token (theme.json)`);
  LINT.set(id, { errors, warnings });
}

// height: a number, or 'auto' — the page ends 40 px below the lowest cell (sequence lifelines included).
export function diagram({ id, name, width, height }, build) {
  const cells = [];
  const geo = [];
  const edges = [];
  const lines = [];
  const meta = {};
  const sequences = [];
  let n = 0;
  const nextId = (p) => `${p}${++n}`;

  const ctx = {
    C,
    // Rounded box. kind ∈ KINDS; shape: 'rect' | 'pill' | 'cyl' | 'doc' | 'diamond' | 'circle'.
    node(key, main, { x, y, w = 180, h = 64, kind = 'neutral', sub, size = 13, bold = true, shape = 'rect', align = 'center', valign = 'middle', subColor, mono, extra = '' }) {
      if (!KIND[kind]) throw new Error(`${id}: unknown node kind "${kind}" (use ${KINDS.join(', ')})`);
      const shapeStyle = {
        rect: 'rounded=1;arcSize=10;',
        pill: 'rounded=1;arcSize=50;',
        cyl: 'shape=cylinder3;boundedLbl=1;backgroundOutline=1;size=10;',
        doc: 'shape=document;boundedLbl=1;size=0.14;',
        diamond: 'rhombus;',
        circle: 'ellipse;aspect=fixed;',
      }[shape];
      if (!shapeStyle) throw new Error(`${id}: unknown shape "${shape}"`);
      const style = `${shapeStyle}whiteSpace=wrap;html=1;${FONT}fontSize=${size};${bold ? 'fontStyle=1;' : ''}align=${align};verticalAlign=${valign};spacing=8;${valign === 'top' ? 'spacingTop=10;' : ''}${KIND[kind]}${extra}`;
      cells.push(`<mxCell id="${key}" value="${esc(label(main, sub, { subColor, mono }))}" style="${style}" vertex="1" parent="1"><mxGeometry x="${x}" y="${y}" width="${w}" height="${h}" as="geometry"/></mxCell>`);
      geo.push({ key, type: 'node', x, y, w, h, size, kind });
      return key;
    },
    // Container / swimlane-like group: label top-left, sits behind nodes (add it first).
    group(key, title, { x, y, w, h, kind = 'quiet', sub, accent }) {
      const stroke = accent ? `strokeColor=${accent};` : '';
      const style = `rounded=1;arcSize=3;whiteSpace=wrap;html=1;${FONT}fontSize=11;fontStyle=1;align=left;verticalAlign=top;spacingLeft=14;spacingTop=8;${KIND[kind]}${stroke}`;
      const title2 = sub ? `${esc(title.toUpperCase())}<span style="font-weight:400;color:${C.textLow}">  ·  ${esc(sub)}</span>` : esc(title.toUpperCase());
      cells.push(`<mxCell id="${key}" value="${esc(title2)}" style="${style}" vertex="1" parent="1"><mxGeometry x="${x}" y="${y}" width="${w}" height="${h}" as="geometry"/></mxCell>`);
      geo.push({ key, type: 'group', x, y, w, h });
      return key;
    },
    text(main, { x, y, w = 400, h = 24, size = 12, color = C.textMid, bold = false, align = 'left', sub, subColor }) {
      const key = nextId('t');
      const style = `text;html=1;whiteSpace=wrap;${FONT}fontSize=${size};fontColor=${color};${bold ? 'fontStyle=1;' : ''}align=${align};verticalAlign=middle;`;
      cells.push(`<mxCell id="${key}" value="${esc(label(main, sub, { subColor }))}" style="${style}" vertex="1" parent="1"><mxGeometry x="${x}" y="${y}" width="${w}" height="${h}" as="geometry"/></mxCell>`);
      geo.push({ key, type: 'text', x, y, w, h, size });
      return key;
    },
    // Every diagram starts with a title that states the question it answers, and a sub line
    // naming the example / source it is drawn from.
    title(main, sub, { x = 40, y = 20, w = 1100 } = {}) {
      ctx.text(main, { x, y, w, h: 30, size: 18, color: C.textHi, bold: true });
      if (sub) ctx.text(sub, { x, y: y + 30, w, h: 22, size: 12, color: C.textLow });
    },
    // Plain filled rectangle (timeline bars, ducking dips…); no label markup.
    bar(key, { x, y, w, h, fill, stroke, opacity, text = '', size = 11, color = C.page, bold = true, dashed = false, radius = 4 }) {
      const style = `rounded=1;absoluteArcSize=1;arcSize=${radius * 2};html=1;whiteSpace=wrap;${FONT}fontSize=${size};${bold ? 'fontStyle=1;' : ''}fillColor=${fill};strokeColor=${stroke || fill};fontColor=${color};${opacity != null ? `fillOpacity=${opacity};` : ''}${dashed ? 'dashed=1;dashPattern=4 3;' : ''}`;
      cells.push(`<mxCell id="${key}" value="${esc(text)}" style="${style}" vertex="1" parent="1"><mxGeometry x="${x}" y="${y}" width="${w}" height="${h}" as="geometry"/></mxCell>`);
      geo.push({ key, type: 'bar', x, y, w, h, size: text ? size : null });
      return key;
    },
    line(key, { x1, y1, x2, y2, color = C.borderStrong, width = 1, dashed = false, arrow = false }) {
      lines.push({ key, idx: cells.length, y: Math.max(y1, y2) });
      const style = `endArrow=${arrow ? 'block' : 'none'};endFill=1;html=1;strokeColor=${color};strokeWidth=${width};${dashed ? 'dashed=1;dashPattern=4 3;' : ''}`;
      cells.push(`<mxCell id="${key}" style="${style}" edge="1" parent="1"><mxGeometry relative="1" as="geometry"><mxPoint x="${x1}" y="${y1}" as="sourcePoint"/><mxPoint x="${x2}" y="${y2}" as="targetPoint"/></mxGeometry></mxCell>`);
      return key;
    },
    // Edge between two node keys. exit/entry: 'r','l','t','b' or [x,y] fractions.
    // points: explicit waypoints — use them whenever the automatic route would cross a node.
    edge(from, to, { label: lbl = '', kind = 'default', exit, entry, points = [], curved = false, width = 1.6, straight = false, endArrow = 'block', startArrow = 'none', labelPos, labelAt } = {}) {
      if (!EDGE[kind]) throw new Error(`${id}: unknown edge kind "${kind}" (use ${EDGE_KINDS.join(', ')})`);
      const key = nextId('e');
      const side = (s) => ({ r: [1, 0.5], l: [0, 0.5], t: [0.5, 0], b: [0.5, 1] }[s] || s);
      const ex = exit ? side(exit) : null;
      const en = entry ? side(entry) : null;
      const route = straight ? '' : (curved ? 'curved=1;' : 'edgeStyle=orthogonalEdgeStyle;rounded=1;');
      const style = `${route}jumpStyle=arc;jumpSize=7;html=1;${FONT}fontSize=10.5;strokeWidth=${width};endArrow=${endArrow};endFill=1;startArrow=${startArrow};startFill=1;labelBackgroundColor=${C.page};${EDGE[kind]}` +
        (ex ? `exitX=${ex[0]};exitY=${ex[1]};exitDx=0;exitDy=0;` : '') +
        (en ? `entryX=${en[0]};entryY=${en[1]};entryDx=0;entryDy=0;` : '');
      const pts = points.length ? `<Array as="points">${points.map(([x, y]) => `<mxPoint x="${x}" y="${y}"/>`).join('')}</Array>` : '';
      const off = labelPos != null ? `<mxPoint x="${labelPos}" y="0" as="offset"/>` : '';
      cells.push(`<mxCell id="${key}" value="${esc(lbl)}" style="${style}" edge="1" parent="1" source="${from}" target="${to}"><mxGeometry ${labelAt != null ? `x="${labelAt}" ` : ''}relative="1" as="geometry">${pts}${off}</mxGeometry></mxCell>`);
      edges.push({ key, from, to, points });
      return key;
    },
    // Colour legend. Required whenever colour encodes meaning (change type, owner, risk…).
    legend({ x, y, items = ['new', 'mod', 'chk', 'none'], names: custom = {} }) {
      meta.legend = { items, names: {} };
      const names = { new: 'New', mod: 'Modified', chk: 'Verify only', none: 'Untouched', warn: 'Risk / guard', success: 'Existing, reused', info: 'External / platform', lilac: 'User action', ...custom };
      items.forEach((k, i) => {
        meta.legend.names[k] = names[k];
        ctx.node(`lg_${k}`, names[k], { x: x + i * 132, y, w: 120, h: 28, kind: k, size: 10.5 });
      });
    },

    // ------------------------------------------------------------ layout helpers
    // Prefer these over hand-placed coordinates: they keep spacing on one rhythm.

    // Evenly spaced row (dir 'x') or column (dir 'y') of nodes; optional chaining edges.
    // items: [{ key, main, ...nodeOpts }]. Returns the keys.
    row(items, { x, y, w = 180, h = 64, gap = 40, dir = 'x', chain = false, edgeKind = 'default', ...common }) {
      const keys = items.map((it, i) => {
        const { key, main, ...opts } = it;
        const pos = dir === 'x' ? { x: x + i * (w + gap), y } : { x, y: y + i * (h + gap) };
        return ctx.node(key, main, { w, h, ...common, ...opts, ...pos });
      });
      if (chain) keys.slice(1).forEach((k, i) => ctx.edge(keys[i], k, { kind: edgeKind, exit: dir === 'x' ? 'r' : 'b', entry: dir === 'x' ? 'l' : 't' }));
      return keys;
    },

    // Sequence diagram: lane headers with lifelines, then numbered messages top to bottom.
    // lanes: [[key, title, sub, kind], …]. Returns { msg(from, to, text, { kind, ret }), self(at, text, kind), get y() }.
    // bottom: omit it and the lifelines end just below the last message (the usual choice).
    // msg() and self() return their step number, so notes can cite steps without hand-counting:
    //   const pay = msg('prov', 'stripe', 'charge'); … d.node('note', `Money moves at step ${pay} only`, …)
    sequence(lanes, { left = 40, top = 92, laneW = 136, pitch = 148, headH = 50, bottom, start = 170, step = 46, selfStep = 52, minLabelW = 330 } = {}) {
      const X = {};
      const seq = { lifelines: [], bottom };
      sequences.push(seq);
      lanes.forEach(([k, name, sub, kind], i) => {
        const x = left + i * pitch;
        X[k] = x + laneW / 2;
        ctx.node(`h_${k}`, name, { x, y: top, w: laneW, h: headH, kind, sub, size: 11.5 });
        seq.lifelines.push(cells.length);
        ctx.line(`ll_${k}`, { x1: X[k], y1: top + headH, x2: X[k], y2: bottom ?? 0, color: C.border, dashed: true });
      });
      let y = start;
      let n2 = 0;
      seq.end = () => y;
      const colour = { default: C.edge, accent: C.primary, info: C.info, lilac: C.lilac, success: C.success, warn: C.warn };
      return {
        X,
        get y() { return y; },
        msg(from, to, text, { kind = 'default', ret = false } = {}) {
          n2 += 1;
          const color = colour[kind];
          ctx.line(`m${n2}`, { x1: X[from], y1: y, x2: X[to], y2: y, color, width: 1.6, dashed: ret, arrow: true });
          const lx = Math.min(X[from], X[to]);
          const w = Math.max(Math.abs(X[to] - X[from]), minLabelW);
          ctx.text(`${n2}  ${text}`, { x: lx + 4, y: y - 19, w, h: 16, size: 10.5, color: kind === 'default' ? C.textMid : color });
          y += step;
          return n2;
        },
        self(at, text, kind = 'neutral') {
          n2 += 1;
          ctx.node(`s${n2}`, `${n2}  ${text}`, { x: X[at] - 70, y: y - 16, w: 280, h: 34, kind, size: 10.5, bold: false, align: 'left' });
          y += selfStep;
          return n2;
        },
        get count() { return n2; },
      };
    },
  };

  build(ctx);
  // Sequences without an explicit bottom: lifelines end just below the last message.
  for (const sq of sequences) {
    if (sq.bottom != null) continue;
    const b = sq.end() + 10;
    for (const i of sq.lifelines) cells[i] = cells[i].replace(/(<mxPoint x="[\d.]+" y=")[\d.]+(" as="targetPoint"\/>)/, `$1${b}$2`);
    for (const x of lines) if (sq.lifelines.includes(x.idx)) x.y = b;
  }
  if (height === 'auto') height = Math.ceil(Math.max(...geo.map((g) => g.y + g.h), ...lines.map((l) => l.y)) + 40);
  lint(id, width, height, geo, edges, cells, meta);

  return `<mxfile host="drawio-cli" agent="scoping-plan" version="31.4.5">
  <diagram id="${id}" name="${esc(name)}">
    <mxGraphModel dx="0" dy="0" grid="0" gridSize="10" guides="1" tooltips="1" connect="1" arrows="1" fold="1" page="1" pageScale="1" pageWidth="${width}" pageHeight="${height}" math="0" shadow="0" background="none" adaptiveColors="none">
      <root>
        <mxCell id="0"/>
        <mxCell id="1" parent="0"/>
        ${cells.join('\n        ')}
      </root>
    </mxGraphModel>
  </diagram>
</mxfile>
`;
}
