import { diagram } from '../../build/drawio-kit.mjs';

// Question: what crosses each boundary between an app and the engine?
// Groups left→right in call order: typed layer (x 40) · native boundary (x 480) · method table (x 940), 60 px gutters.
// Row 1 (y 136, h 176): api-surface → Engine → method table, centres level so edges run straight.
// Row 2 (y 360, h 124): Folders under Engine (feeds it at start) · requests under the method table.
const id = 'd04-data-model';
const R1 = 136, H1 = 176, R2 = 360, H2 = 124;
export default {
  id,
  xml: diagram({ id, name: 'What crosses each boundary', width: 1400, height: 'auto' }, (d) => {
    const { C } = d;
    d.title('One Engine object carries JSON requests; the typed layer is generated from the engine itself', 'Native boundary in §14 · method table in §14 · api-surface.json in §21', { w: 900 });
    d.legend({ x: 1010, y: 30, items: ['new', 'mod'], names: { new: 'New', mod: 'Seam S4' } });
    const ent = (key, name, fields, { x, y, w = 340, h, kind = 'new' }) =>
      d.node(key, name, { x, y, w, h, kind, sub: fields.join('\n'), mono: true, size: 12.5, align: 'left', valign: 'top' });

    d.group('typed', 'Typed layer', { x: 40, y: 92, w: 380, h: 412, sub: 'Swift · Kotlin' });
    ent('surface', 'api-surface.json  (generated)', ['app · engineVersion · pin', 'commands[763]', '  id · label · params', '  keys: name · required', '        type · values', 'tools[27]: name · inputSchema'], { x: 60, y: R1, h: H1 });

    d.group('boundary', 'Native boundary', { x: 480, y: 92, w: 400, h: 412, sub: 'UniFFI' });
    ent('engine', 'Engine  (object)', ['request(method, params) → JSON', 'request_with_progress(…)', 'frame(params, format) → bytes', 'mcp(line) → line?', 'present · suspend_gpu · close', 'errors: CraftError (8 kinds)'], { x: 500, y: R1, w: 360, h: H1 });
    ent('folders', 'Folders  (record)', ['container · bundle?', 'config · cache · models', 'plugins?   none = plug-ins off', 'fonts[]'], { x: 500, y: R2, w: 360, h: H2 });

    d.group('methods', 'EffectCraft method table', { x: 940, y: 92, w: 420, h: 412, sub: 'craftbridge-effects' });
    ent('table', 'methods  (JSON)', ['info · commands · exec · run', 'props · get · set', 'open · save · saveAs · new', 'frame · export · template.*', 'script · replay · timings', 'mcp'], { x: 960, y: R1, w: 380, h: H1 });
    ent('requests', 'RenderRequest · FrameRequest', ['comp · output · format', 'start · end · codec options', 'time | frame · maxSide | scale', 'in effectcraft-automation'], { x: 960, y: R2, w: 380, h: H2, kind: 'mod' });

    d.edge('surface', 'engine', { exit: 'r', entry: 'l', label: 'request' });
    d.edge('folders', 'engine', { exit: 't', entry: 'b', label: 'start' });
    d.edge('engine', 'table', { exit: 'r', entry: 'l', label: 'JSON' });
    d.edge('table', 'requests', { exit: 'b', entry: 't', label: 'export · frame' });

    d.text('Frames cross as bytes (PNG, straight RGBA, premultiplied BGRA), never as JSON.', { x: 60, y: R2, w: 340, h: 60, size: 12, color: C.textMid });
  }),
};
