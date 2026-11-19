import { diagram } from '../../build/drawio-kit.mjs';

// Question: what does CraftBridge add, and what does it build on?
// Columns (node w 230): A 60 · B 320 · C 580 · D 870 · E 1120. Rows (node y): R1 136 · R2 292 · R3 448 · R4 548 · R5 704 · R6 804.
// Groups: app (C, R1) · packages (B, D at R2) · native library (C at R3; B, D, E at R4) ·
// EffectCraft at the pin (A, B, C at R5; A, B at R6) · platform (D, E at R5).
// Calls run top→bottom; cross-row edges turn only in the gutters y 233 · 389 · 531 · 645.
const id = 'd02-architecture';
const col = { A: 60, B: 320, C: 580, D: 870, E: 1120 };
const row = { R1: 136, R2: 292, R3: 448, R4: 548, R5: 704, R6: 804 };
const W = 230, H = 66;
const cx = (c, f = 0.5) => col[c] + W * f;
export default {
  id,
  xml: diagram({ id, name: 'What changes, where', width: 1400, height: 'auto' }, (d) => {
    d.title('CraftBridge wraps the unchanged engine; everything new lives in its own repository', 'New repository ChristopherLinnett/craftbridge · EffectCraft at the official revision rust/Cargo.toml pins, never modified', { w: 820 });
    d.legend({ x: 880, y: 26, items: ['new', 'chk', 'none'], names: { new: 'New', chk: 'Unchanged', none: 'Platform, app' } });

    const n = (key, c, r, main, sub, kind) => d.node(key, main, { x: col[c], y: row[r], w: W, h: H, kind, sub });

    d.group('gapp', 'Host app', { x: 560, y: 92, w: 270, h: 126 });
    n('app', 'C', 'R1', 'Postcard Studio', 'the developer’s own interface', 'none');

    d.group('gpkg', 'Packages', { x: 300, y: 248, w: 820, h: 126 });
    n('swift', 'B', 'R2', 'CraftBridgeEffects + CraftBridge', 'Swift: generated commands + typed calls', 'new');
    n('kotlin', 'D', 'R2', ':effects + :core', 'Kotlin: generated commands + typed calls', 'new');

    d.group('glib', 'Native library', { x: 300, y: 404, w: 1060, h: 226 });
    n('ffi', 'C', 'R3', 'UniFFI exports', 'Engine object · platform handles', 'new');
    n('fx', 'B', 'R4', 'craftbridge-effects', 'method table · adaptations AD1–AD7', 'new');
    n('core', 'D', 'R4', 'craftbridge-core', 'engine thread · GPU device · folders', 'new');
    n('media', 'E', 'R4', 'craftbridge-media', 'hardware decoders → FilmCraft registry', 'new');

    d.group('gup', 'EffectCraft at the pin', { x: 40, y: 660, w: 790, h: 226 });
    n('auto', 'A', 'R5', 'automation', 'Backend · McpServer · tools', 'chk');
    n('host', 'B', 'R5', 'host', 'FileExporter', 'chk');
    n('gpu', 'C', 'R5', 'GPU compositor', 'Gpu::new · render_display', 'chk');
    n('eng', 'A', 'R6', 'engine · text · export', 'Session · render queue · fonts', 'chk');
    n('reg', 'B', 'R6', 'FilmCraft decoder registry', 'filmcraft-codecs at EffectCraft’s pin', 'chk');

    d.group('gplat', 'Platform', { x: 850, y: 660, w: 510, h: 126 });
    n('gfx', 'D', 'R5', 'Metal · Vulkan', 'through wgpu', 'none');
    n('codec', 'E', 'R5', 'VideoToolbox · MediaCodec', 'hardware video decoding', 'none');

    d.edge('app', 'swift', { exit: 'b', entry: [0.8, 0], points: [[cx('C'), 233], [cx('B', 0.8), 233]] });
    d.edge('app', 'kotlin', { exit: 'b', entry: 't', points: [[cx('C'), 233], [cx('D'), 233]] });
    d.edge('swift', 'ffi', { exit: [0.8, 1], entry: [0.3, 0], points: [[cx('B', 0.8), 389], [cx('C', 0.3), 389]], label: 'UniFFI', labelAt: -0.2 });
    d.edge('kotlin', 'ffi', { exit: 'b', entry: [0.7, 0], points: [[cx('D'), 389], [cx('C', 0.7), 389]] });
    d.edge('ffi', 'fx', { exit: [0.3, 1], entry: 't', points: [[cx('C', 0.3), 531], [cx('B'), 531]] });
    d.edge('ffi', 'core', { exit: [0.7, 1], entry: 't', points: [[cx('C', 0.7), 531], [cx('D'), 531]] });
    d.edge('core', 'media', { exit: 'r', entry: 'l' });
    d.edge('fx', 'auto', { exit: [0.3, 1], entry: [0.8, 0], points: [[cx('B', 0.3), 645], [cx('A', 0.8), 645]] });
    d.edge('fx', 'host', { exit: 'b', entry: 't' });
    d.edge('fx', 'gpu', { exit: [0.7, 1], entry: 't', points: [[cx('B', 0.7), 645], [cx('C'), 645]] });
    d.edge('core', 'gfx', { exit: 'b', entry: 't' });
    d.edge('media', 'codec', { exit: 'b', entry: 't' });
    d.edge('auto', 'eng', { exit: 'b', entry: 't' });
    d.edge('eng', 'reg', { exit: 'r', entry: 'l' });
  }),
};
