# Conformance fixture sources

The media the corpus (`../corpus.toml`) starts its scenarios with. Each was made for the corpus as the simplest file its scenario needs, copies nothing, and is the repository's own work under its licences, MIT OR Apache-2.0. The other files here (`probe.wat`, `pass.jsonl`, `fail.jsonl`, `sam-values.json`) are text the scenarios read, not assets.

| File | What it holds | Scenario | Licence |
|---|---|---|---|
| `photo.png` | 32 × 32 RGB, flat red (200, 30, 30) | T17: the template's own picture | MIT OR Apache-2.0 |
| `sam.png` | 32 × 32 RGB, flat green (30, 200, 30) | T17: the picture its Photo control is given | MIT OR Apache-2.0 |
| `tone.wav` | half a second of a 440 Hz sine, 16-bit mono PCM at 8 kHz | T15: the audio its WAV export mixes | MIT OR Apache-2.0 |
