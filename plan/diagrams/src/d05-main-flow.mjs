import { diagram } from '../../build/drawio-kit.mjs';

// Question: what happens, call by call, when Maya exports her card?
// Six lanes in call order, left→right: app · Swift package · engine actor · EffectCraft app crate ·
// CraftBridge's render runner · EffectCraft session. Accent marks the path CraftBridge adds.
// Drawn from §14 (method table), §15 (AD3), §16 (actor, L3), §19 (the render runner) and the CLI's queue loop
// (apps/effectcraft-cli/src/main.rs:958-1035 @ 10376058).
const id = 'd05-main-flow';
export default {
  id,
  xml: diagram({ id, name: 'Maya’s export, call by call', width: 1400, height: 'auto' }, (d) => {
    d.title('An export is one request: CraftBridge’s render runner drives EffectCraft’s own render queue and reports progress', 'Running example: Birthday Card, 180 frames, 1080 × 1080 H.264 · traced from §14, §16, §19');
    const seq = d.sequence([
      ['app', 'Postcard Studio', 'host app', 'lilac'],
      ['swift', 'CraftBridgeEffects', 'Swift package', 'new'],
      ['actor', 'Engine actor', 'craftbridge-core', 'new'],
      ['fx', 'craftbridge-effects', 'method table', 'new'],
      ['auto', 'render runner', 'craftbridge-effects (AD3)', 'new'],
      ['session', 'EffectCraft session', 'unchanged', 'chk'],
    ]);
    const { msg, self } = seq;
    msg('app', 'swift', 'export(card, mp4, .h264, progress)', { kind: 'accent' });
    msg('swift', 'actor', 'requestWithProgress("export", RenderRequest)', { kind: 'accent' });
    msg('actor', 'fx', 'request_with_progress (other calls now wait, L3)', { kind: 'accent' });
    msg('fx', 'auto', 'run(session, request, on_progress)');
    msg('auto', 'session', 'renderQueue.add · renderQueue.render {wait: false}');
    const poll = self('auto', 'every 100 ms: poll_render · render_progress');
    msg('auto', 'actor', 'on_progress(done, total, seconds left)', { ret: true, kind: 'accent' });
    msg('actor', 'swift', 'ProgressListener.progress', { ret: true, kind: 'accent' });
    msg('swift', 'app', 'progress bar', { ret: true, kind: 'lilac' });
    const cancel = msg('app', 'swift', 'cancel (Task cancelled)', { kind: 'warn' });
    msg('swift', 'actor', 'CancelToken.cancel()', { kind: 'warn' });
    msg('actor', 'auto', 'next on_progress returns false', { ret: true, kind: 'warn' });
    msg('auto', 'session', 'stop_render');
    msg('session', 'auto', 'item status: Done or User Stopped (its file removed)', { ret: true });
    msg('auto', 'actor', 'output path, or cancelled', { ret: true });
    msg('actor', 'swift', 'result, or CraftError.Cancelled', { ret: true });
    msg('swift', 'app', 'file URL, or CancellationError', { ret: true, kind: 'lilac' });
    d.node('note', `Steps ${poll}–${cancel + 3}`, { x: 40, y: seq.y + 12, w: 1320, h: 52, kind: 'warn', sub: 'Progress and cancel travel through the engine actor while the export runs; any other call the app makes waits in order until the export ends (L3, L4).', size: 11.5, align: 'left' });
  }),
};
