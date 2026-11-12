# Brand sources

The plan carries EffectCraft's own look. Every file here is copied unchanged from `storytold/effectcraft` at `10376058` (`git show 10376058:<path>`), except the icon, which is resized.

| File | Source | Licence |
|---|---|---|
| `effectcraft-icon-256.png` | `assets/app-icon/effectcraft-1024.png`, resized to 256 px with `sips -Z 256` | MIT OR Apache-2.0 (`LICENSE-app-icon.txt`, from `assets/app-icon/LICENSE.txt`) |
| `fonts/Inter-*.ttf` | `assets/fonts/Inter-*.ttf` | SIL OFL 1.1 (`fonts/OFL-Inter.txt`) |
| `fonts/JetBrainsMono-Regular.ttf` | `assets/fonts/JetBrainsMono-Regular.ttf` | SIL OFL 1.1 (`fonts/OFL-JetBrainsMono.txt`) |

Colours in `../theme.json` are EffectCraft's dark interface tokens (`crates/ui-egui/src/theme.rs:148-191`) and the app colour of its icon (`assets/app-icon/README.md`, `#e0368f`).

The ArtCraft wordmark and mark (`docs/brand/`) are trademarks licensed only for EffectCraft itself (`docs/brand/LICENSE-brand.txt`); this plan is for an independent SDK (PD2), so they are not used.
