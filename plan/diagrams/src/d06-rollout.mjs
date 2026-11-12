import { diagram } from '../../build/drawio-kit.mjs';

// Question: in what order is CraftBridge built, and what decides each step?
// Three phase groups left→right, three stages each. Stage w 110, h 124, gap 16; group w 410
// (24 + 3·110 + 2·16 + 24), gutter 30: groups at x 40 · 480 · 920. Stages y 156.
// Gates are stages of their own (magenta), never slivers in the gutter. Every edge joins neighbours.
const id = 'd06-rollout';
const GX = [40, 480, 920], GW = 410, SW = 110, SH = 124, Y = 156;
const sx = (g, i) => GX[g] + 24 + i * (SW + 16);
export default {
  id,
  xml: diagram({ id, name: 'Build order', width: 1400, height: 'auto' }, (d) => {
    const { C } = d;
    d.title('Each wave stands on the one before; three gates decide when to move on and when to publish', 'Effort from §31 · gates from §26', { w: 960 });
    d.legend({ x: 1040, y: 30, items: ['neutral', 'magenta'], names: { neutral: 'Wave', magenta: 'Gate' } });

    d.group('p1', 'Upstream first', { x: GX[0], y: 104, w: GW, h: 196, accent: C.primary });
    d.group('p2', 'Build', { x: GX[1], y: 104, w: GW, h: 196 });
    d.group('p3', 'Prove and release', { x: GX[2], y: 104, w: GW, h: 196 });

    const st = (key, g, i, main, sub, kind = 'neutral') => d.node(key, main, { x: sx(g, i), y: Y, w: SW, h: SH, kind, sub, size: 13, align: 'left', valign: 'top' });
    st('w0', 0, 0, 'W0', 'Prerequisites: repo, toolchains, gate · 2 d');
    st('w1', 0, 1, 'W1', 'Seams S1–S7 + the seam branch · 10 d');
    st('g1', 0, 2, 'G1', 'Seam branch passes the desktop gate', 'magenta');
    st('w2', 1, 0, 'W2', 'Core + EffectCraft crate; desktop conformance · 14 d');
    st('w3', 1, 1, 'W3', 'Bindings + packages; simulator, emulator · 16 d');
    st('w4', 1, 2, 'W4', 'GPU surfaces + decoders; devices · 14 d');
    st('g2', 2, 0, 'G2', 'Device acceptance: the whole corpus on three phones', 'magenta');
    st('w5', 2, 1, 'W5', 'Samples, guides, staged release · 9.5 d');
    st('g3', 2, 2, 'G3', 'The owner decides to publish', 'magenta');

    const chain = ['w0', 'w1', 'g1', 'w2', 'w3', 'w4', 'g2', 'w5', 'g3'];
    for (let i = 0; i < chain.length - 1; i++) d.edge(chain[i], chain[i + 1], { exit: 'r', entry: 'l' });

    d.text('65.5 engineer-days for W0–W5. Hardware encoding (W6, 5 d) follows the merge of seam S7.', { x: 40, y: 320, w: 1320, h: 24, size: 12, color: C.textMid, bold: true });
  }),
};
