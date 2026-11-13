import { diagram } from '../../build/drawio-kit.mjs';

// Question: in what order is CraftBridge built, and what decides each step?
// Three phase groups left→right: two stages, then three, then three. Stage w 110, h 124, gap 16;
// a group is 24 + n·110 + (n−1)·16 + 24 wide (284 for two, 410 for three), gutter 30:
// groups at x 40 · 354 · 794. Stages y 156.
// Gates are stages of their own (magenta), never slivers in the gutter. Every edge joins neighbours.
const id = 'd06-rollout';
const GX = [40, 354, 794], GW = [284, 410, 410], SW = 110, SH = 124, Y = 156;
const sx = (g, i) => GX[g] + 24 + i * (SW + 16);
export default {
  id,
  xml: diagram({ id, name: 'Build order', width: 1400, height: 'auto' }, (d) => {
    const { C } = d;
    d.title('Each wave stands on the one before; two gates decide when to move on and when to publish', 'Effort from §31 · gates from §26', { w: 960 });
    d.legend({ x: 1040, y: 30, items: ['neutral', 'magenta'], names: { neutral: 'Wave', magenta: 'Gate' } });

    d.group('p1', 'Foundations', { x: GX[0], y: 104, w: GW[0], h: 196, accent: C.primary });
    d.group('p2', 'Build', { x: GX[1], y: 104, w: GW[1], h: 196 });
    d.group('p3', 'Prove and release', { x: GX[2], y: 104, w: GW[2], h: 196 });

    const st = (key, g, i, main, sub, kind = 'neutral') => d.node(key, main, { x: sx(g, i), y: Y, w: SW, h: SH, kind, sub, size: 13, align: 'left', valign: 'top' });
    st('w0', 0, 0, 'W0', 'Prerequisites: platform builds, gate · 4\u00a0d');
    st('w1', 0, 1, 'W1', 'Official EffectCraft pin + adaptations list · 1.5\u00a0d');
    st('w2', 1, 0, 'W2', 'Core + EffectCraft crate; desktop conformance · 20\u00a0d');
    st('w3', 1, 1, 'W3', 'Bindings + packages; simulator, emulator · 16\u00a0d');
    st('w4', 1, 2, 'W4', 'GPU surfaces + decoders; devices · 14\u00a0d');
    st('g1', 2, 0, 'G1', 'Device acceptance: the whole corpus on three phones', 'magenta');
    st('w5', 2, 1, 'W5', 'Samples, guides, staged release · 9\u00a0d');
    st('g2', 2, 2, 'G2', 'The owner decides to publish', 'magenta');

    const chain = ['w0', 'w1', 'w2', 'w3', 'w4', 'g1', 'w5', 'g2'];
    for (let i = 0; i < chain.length - 1; i++) d.edge(chain[i], chain[i + 1], { exit: 'r', entry: 'l' });

    d.text('64.5 engineer-days for W0–W5. Hardware encoding in CraftBridge’s own exporter (W6, 6.5 d) follows the first release.', { x: 40, y: 320, w: 1320, h: 24, size: 12, color: C.textMid, bold: true });
  }),
};
