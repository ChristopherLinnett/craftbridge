import { diagram } from '../../build/drawio-kit.mjs';

// Question: how does a template made on the desktop become a card exported on a phone?
// Layout: two phases left→right. Stage w 180, h 132, y 164.
// Phase 1 (x 40, w 438): stages at x 64 and 274 (gap 30). Gutter 40.
// Phase 2 (x 518, w 840): stages at x 542, 746, 950, 1154 (gap 24); last ends 1334.
// Every edge joins neighbours left→right; the cancel loop sits under its own stage.
const id = 'd01-journey';
const W = 180;
export default {
  id,
  xml: diagram({ id, name: 'From a desktop template to a card exported on a phone', width: 1400, height: 430 }, (d) => {
    const { C } = d;
    d.title('A template made on the desktop becomes a card exported on the phone', 'Running example: Leo builds Postcard Studio; Maya makes Sam’s birthday card (1080 × 1080, 30 fps, 6 s, 180 frames)', { w: 960 });
    d.legend({ x: 1040, y: 30, items: ['neutral', 'new'], names: { neutral: 'A person acts', new: 'CraftBridge' } });

    d.group('dev', 'Leo builds the app', { x: 40, y: 104, w: 438, h: 252, sub: 'once, on his Mac', accent: C.primary });
    d.group('use', 'Maya makes the card', { x: 518, y: 104, w: 840, h: 252, sub: 'on her iPhone 15 (Metal) · a friend on a Pixel 8 (Vulkan)' });

    const stage = (key, n, main, sub, x, kind = 'neutral') =>
      d.node(key, `${n}  ${main}`, { x, y: 164, w: W, h: 132, kind, sub, size: 14, align: 'left', valign: 'top' });
    stage('s1', '①', 'Designs the card', 'In EffectCraft: “Social Square” template + Name and Photo controls → Birthday Card.ectemplate', 64);
    stage('s2', '②', 'Adds CraftBridge', 'Swift package or Gradle library; bundles the template in the app', 274, 'new');
    stage('s3', '③', 'Fills in the card', 'Name = “Sam”, a photo from her library', 542);
    stage('s4', '④', 'Live preview', 'Drawn on the GPU straight into the app’s own view', 746, 'new');
    stage('s5', '⑤', 'Exports', 'MP4, 180 frames, progress bar; continues in the background', 950, 'new');
    stage('s6', '⑥', 'Shares the card', 'The file never left the phone', 1154);

    d.edge('s1', 's2', { exit: 'r', entry: 'l' });
    d.edge('s2', 's3', { exit: 'r', entry: 'l' });
    d.edge('s3', 's4', { exit: 'r', entry: 'l' });
    d.edge('s4', 's5', { exit: 'r', entry: 'l' });
    d.edge('s5', 's6', { exit: 'r', entry: 'l' });
    d.edge('s5', 's5', { kind: 'warn', exit: [0.25, 1], entry: [0.75, 1], points: [[995, 336], [1085, 336]], label: 'cancel → export again' });

    d.text('Same engine as the EffectCraft desktop app: a frame rendered on the CPU is byte-for-byte the same on the iPhone, the Pixel and the Mac.', { x: 40, y: 380, w: 1320, h: 24, size: 12, color: C.textMid, bold: true });
  }),
};
