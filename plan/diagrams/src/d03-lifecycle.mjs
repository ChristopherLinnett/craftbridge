import { diagram } from '../../build/drawio-kit.mjs';

// Question: what states does an engine inside an app go through?
// Columns x 60 · 400 · 740 (100 px gutters hold the labels). Rows y 132 · 312. State w 240, h 76.
// Row 1: Starting → Ready ⇄ Exporting. Row 2 under each: Failed to start · CPU only · Closed.
// Ready⇄Exporting and Ready⇄CPU only are pairs of parallel edges at matched fractions; Ready→Closed
// is the one diagonal, through the empty cell between them.
const id = 'd03-lifecycle';
const col = [60, 400, 740];
const row = [132, 312];
const W = 240, H = 76;
export default {
  id,
  xml: diagram({ id, name: 'An engine’s life inside an app', width: 1100, height: 'auto' }, (d) => {
    const { C } = d;
    d.title('An engine serves calls in order, exports one job at a time and falls back to the CPU', 'One EffectCraft session per engine, on its own thread (L1–L7)', { w: 760 });
    d.legend({ x: 840, y: 30, items: ['neutral', 'warn'], names: { neutral: 'State', warn: 'Failure' } });

    const st = (key, c, r, main, sub, kind = 'neutral') => d.node(key, main, { x: col[c], y: row[r], w: W, h: H, kind, sub, shape: 'pill' });
    st('start', 0, 0, 'Starting', 'folders · session · GPU if any');
    st('ready', 1, 0, 'Ready', 'calls run in order (L1)');
    st('export', 2, 0, 'Exporting', 'only progress, cancel, info (L3)');
    st('failed', 0, 1, 'Failed to start', 'folders not writable → Io', 'warn');
    st('cpu', 1, 1, 'CPU only', 'GPU lost, refused or suspended (L7)');
    st('closed', 2, 1, 'Closed', 'export stopped · GPU released');

    d.edge('start', 'ready', { exit: 'r', entry: 'l' });
    d.edge('start', 'failed', { exit: 'b', entry: 't', kind: 'warn' });
    d.edge('ready', 'export', { exit: [1, 0.3], entry: [0, 0.3], label: 'export' });
    d.edge('export', 'ready', { exit: [0, 0.7], entry: [1, 0.7], label: 'done · cancelled' });
    d.edge('ready', 'cpu', { exit: [0.3, 1], entry: [0.3, 0], label: 'GPU lost · background' });
    d.edge('cpu', 'ready', { exit: [0.7, 0], entry: [0.7, 1], label: 'GPU back' });
    d.edge('ready', 'closed', { exit: [1, 1], entry: [0, 0], label: 'close' });
    d.edge('export', 'closed', { exit: 'b', entry: 't', label: 'close (stops the export)' });

    d.text('The CPU renderer is the reference in every state; the GPU compositor matches it within its own tolerance.', { x: 60, y: row[1] + H + 28, w: 920, h: 22, size: 12, color: C.textMid, bold: true });
  }),
};
