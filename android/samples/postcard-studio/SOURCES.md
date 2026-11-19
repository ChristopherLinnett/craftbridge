# Postcard Studio for Android: asset sources

The app's own artwork. Each was drawn for the sample as the simplest shape it needs, copies nothing, and is the repository's own work under its licences, MIT OR Apache-2.0. The sample also bundles two files it makes or copies at build time and keeps none of in its tree: the running example's template, which `tools/birthday-card.sh` makes from `conformance/effects/birthday-card.jsonl`, and `conformance/effects/fixtures/sam.png`, listed in [`conformance/effects/fixtures/SOURCES.md`](../../../conformance/effects/fixtures/SOURCES.md).

| File | What it holds | Licence |
|---|---|---|
| `app/src/main/res/drawable/ic_launcher_foreground.xml` | the launcher icon's card with a play mark, as a vector | MIT OR Apache-2.0 |
| `app/src/main/res/drawable/ic_export.xml` | the export notification's small icon, a card with a play mark, as a vector | MIT OR Apache-2.0 |
