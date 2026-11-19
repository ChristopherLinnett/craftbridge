# Upstream updates

Each EffectCraft revision `tools/update-upstream.sh` has taken, newest last: the revisions it moved between, the gate's result, the commands added, changed and removed, and each adaptation it named with how the recheck resolved it (plan §24).

## 103760584a60 to 665bfac0b339

- From: EffectCraft `103760584a60074c9aaeef686d2d05d89f08155e`, FilmCraft `d5ead4ba1ee2f1cd13cb86a4239aee2730a3d9f5`
- To: EffectCraft `665bfac0b33904b1dcda6de889f43f43a94ad3f8`, FilmCraft `3f137a7ed420cac79b32145f5243b7f98b31f71c`
- Gate: passed
- API: breaking change: the next release is a major version
- Export: what EffectCraft's own export writes at this revision: its H.264, HEVC and AV1 movies give their AAC track, from its own `craft_audio::aac` encoder, an edit from the encoder's priming (`media_start`) to the end of the last 1,024-sample unit, and no roll sample group (crates/export/src/encode.rs:412-431, :457): the craft-media it writes with (`5a7a63a`) has `TrackConfig::roll_distance`, and the export leaves it unset. Its command line at this revision, exporting a 4 s comp of a second of silence and then a tone, wrote no `sgpd` or `sbgp` and an edit of 4,010 ms from sample 1,024: ffmpeg, and MediaExtractor on the API 29 emulator (encoder delay 1,024, padding 32), read the tone from sample 48,005, where it is, and 480 samples on past the sound; AVFoundation reads it 2,112 samples early, from sample 45,893, and 190,400 samples in all. So EffectCraft's own movies still play early in Apple's readers. The hardware path's movie writer departs from them here: its sound track sits in a roll group of distance −1, and its edit ends at the sound's own last sample.

### Adaptations

- AD1 (craftbridge-effects): resolved by: read at the new revision: the session struct gained a doc(hidden) project_closed flag, Default-initialized and public only so crates can build the struct; the scratch session comes from Session::new and takes it unset, so the reliance is unchanged; lines refreshed to the grown struct. file.import now asks a file's importer for probe_noting, whose default answers no warning, and the host's importer answers it with effectcraft_media::probe_noting, so an import says what it left out of a file (a 3D model's texture it could not take): CraftBridge's importer now answers it the same way, and AD1 lists both.
  - Session (crates/engine/src/lib.rs, lines 104-269)
- AD4 (craftbridge-effects): resolved by: encode_png keeps its signature and its output: the downscale inside it moved to a pub(crate) fit shared with the new render_pixels, so the PNG the Frame carries is made exactly as before; lines refreshed.
  - Backend::render_with and the Frame it answers, whose PNG encode_png makes from render_rgba8_alpha's pixels (crates/automation/src/lib.rs, lines 53-74)
- AD7 (craftbridge-effects): resolved by: the sequence export writes the same files the same way, named by sequence_path, cut to the module and the same bit depths; only its skip-existing check now asks the context's frame_exists, the same test by another route. The h264 encoder gained 16-bit frames and motion-search presets beside a constructor whose config, try_encode, flush, avcc and sps_pps are untouched, so the software encoder a hardware movie goes on in is still made and fed the same way; lines refreshed.
  - the Targa sequence export: one uncompressed file per frame, named by sequence_path over the comp's first frame, the frame cut to the module's size, 24-bit for RGB and 8-bit grey for the alpha matte (crates/export/src/lib.rs, lines 312-345, 402-416)
  - craft_video::h264's Profile, RateControl, ColorConfig, PacketFormat, EncoderConfig, YuvFrame and Packet, the encoder's constructor, try_encode, flush, avcc and sps_pps, which the software encoder a hardware movie goes on in is made and fed through (storytold/craft-libs:crates/video/src/h264/encode/mod.rs, lines 71-79, 90-98, 114-119, 129-135, 138-201, 205-211, 223-232, 360, 594-602, 639, 685)
- AD8 (craftbridge-media): resolved by: make_video_decoder still opens with the first factory that answers and fails with an error the same way; the no-decoder words now name the entry through undecodable_codec. The software decoders' plane conventions stand: same cuts, same low-bit 10-bit samples, same colour and aspect, with the HEVC one carrying the stream's own chroma where it claimed 4:2:0: the VideoToolbox and MediaCodec decoders take only a stream of 4:2:0 (chroma_format_idc 1, src/layout.rs) and decline every other to the software decoder, so for each stream they take the pictures they return still match its; lines refreshed to the craft-video paths and the widened convert.
  - make_video_decoder, which opens a stream with the first factory that answers, an error included (crates/codecs/src/lib.rs, lines 112-120)
  - the software decoders' planes, cut to the picture's size with 10-bit samples in the low bits, the stream's own colour and pixel aspect and a zero presentation time, which the VideoToolbox and MediaCodec decoders reproduce (crates/codecs/src/video.rs, lines 304-323, 408-437, 676-702)
- AD13 (craftbridge-effects): resolved by: read at the new revision: prop.set's argument list gained factor? and addend? for multiplied and added proportional scrubs; the calls the plugin makes, which give neither, are unaffected; lines refreshed to the widened command.
  - prop.set (crates/engine/src/cmd/prop.rs, lines 1422-1430)
- AD19 (craftbridge-effects): resolved by: read at the new revision: the exporter still writes one file per frame named by sequence_path over the span and the render log at log_path; only the skip-existing test moved into the context; lines refreshed.
  - what the exporter writes: a sequence's file for each frame at sequence_path from the first frame of the span, and the render log at log_path (crates/export/src/lib.rs, lines 308-345)
- AD20 (craftbridge-effects): resolved by: read at the new revision: spawn_task, the poll, the job rows, the cancel and the records are as they were; the only change is that the finished toast now also raises when the applied record carries errors, which the plugin neither gives nor reads; lines refreshed.
  - the session's background tasks: spawn_task, the poll that finishes and applies one, the job rows, the cancel by id and the record each leaves (crates/engine/src/background/jobs.rs, lines 118-155, 157-197, 199-206, 222-279, 282-307)
- AD22 (craftbridge-effects): resolved by: the Lottie import reads through craft-lottie's typed model now instead of raw JSON, and the images' fate is as listed: an embedded asset is stored, a linked one named by u and p against the file's folder, a store that fails skips the image before any item is made, and items follow asset order. Beside Importer::probe the engine gained a defaulted probe_noting, which reports what an import leaves out; every method the adapter's file service answers is still the one the probe asks; lines refreshed.
  - the Lottie import's images: each asset stored, when embedded, or else named by u and p resolved against the Lottie file's folder, an image whose store fails skipped before any item is made, the items made in the order of the assets (crates/lottie/src/import.rs, lines 96-111, 157-226)
  - Services, the file service the probe answers every method of, and Importer::probe, with which a file a call names is told a vector file (crates/engine/src/services.rs, lines 3-58, 104-115)
- AD23 (craftbridge-effects-codegen): resolved by: read whole at the new revision: every tool still reads its file paths through the same argument reads and answers outside the roots through the same confinement sentence, so each tool that is not refused still refuses as recorded; render_frame and render_preview gained times, columns and labels, which compose one contact sheet behind the same confined path check, and the new arguments are numbers and a flag, not paths, so nothing the reading names to a path outside the roots changed; lines refreshed.
  - every tool's own code and input schema, which the reading runs for each tool that is not refused, with every argument the schema names set to a path outside the roots (crates/automation/src/tools.rs, the whole file)

Lines to refresh in `tools/adaptations.toml`, which the revision change moved without touching their items:

- AD1: effectcraft_media::probe (crates/media/src/probe.rs): listed at 17, now at 26
- AD3: Backend::session, the headless session behind the backend (crates/automation/src/backend.rs): listed at 61, now at 68
- AD4: Session::render_rgba8_alpha (crates/engine/src/session/render.rs): listed at 132-143, now at 156-167
- AD4: Session::renderer, the renderer the session's own render uses, which a display frame is rendered with (crates/engine/src/session/render.rs): listed at 11-17, now at 15-21
- AD4: Renderer::active_accel, which offers the accelerator under Backend::Auto only while the project's gpu_acceleration (Mercury GPU Acceleration) is on, and RenderOpts' default backend, Cpu, under which it offers none (crates/render/src/pipeline.rs): listed at 554-562, now at 597-605
- AD4: Backend::render_with and the Frame it answers, whose PNG encode_png makes from render_rgba8_alpha's pixels (crates/automation/src/backend.rs): listed at 122-124, 136-149, now at 129-131, 193-206
- AD4: the headless branch of Backend::render_output, whose resolution of the comp and of the default time the raw path follows through Backend::session, Session::resolve_comp and Session::time (crates/automation/src/backend.rs): listed at 61-66, 136-149, now at 68-73, 193-206
- AD4: no_comp, the words render_output refuses a missing comp with, as BadArgs, before it renders anything; the raw formats ask render_with for them and the plugin maps them to NotFound (crates/automation/src/backend.rs): listed at 142, 181-186, now at 199, 251-256
- AD4: the comp.info figures `frameRate`, `width` and `height`, which the CLI and the plugin read through the get_comp tool (crates/automation/src/tools.rs): listed at 284-286, now at 296-298
- AD5: project.duplicate, comp.open, edit.clear, layer.setSwitch, layer.addItem and file.import, which make the instance (crates/engine/src/cmd/file.rs): listed at 798, now at 811
- AD5: file.findMissing, which names the text layers whose font is missing (crates/engine/src/cmd/file_more.rs): listed at 341-382, 1257, now at 365-406, 1289
- AD5: Backend::headless, the unconfined backend the scratch check runs its commands through (crates/automation/src/backend.rs): listed at 33, now at 40
- AD5: Backend::confinement, Confinement::check and Access, with which staging asks of the folder holding a source as it is named, where the import writes beside it (essential.importTemplate takes its media folder from the path it is given, not where a link leads), whether the session may write there or only read, and checks the staged folder before it makes anything there and the copy's own path once its name is known (crates/automation/src/backend.rs): listed at 53, now at 60
- AD5: Backend::confinement, Confinement::check and Access, with which staging asks of the folder holding a source as it is named, where the import writes beside it (essential.importTemplate takes its media folder from the path it is given, not where a link leads), whether the session may write there or only read, and checks the staged folder before it makes anything there and the copy's own path once its name is known (crates/automation/src/confine.rs): listed at 190, now at 194
- AD7: FileExporter (crates/host/src/lib.rs): listed at 25-89, now at 28-92
- AD7: the compositors' frame, an Image of four f32 channels a pixel, and the CPU compositor's canvas the comp's layers are drawn onto (crates/raster/src/lib.rs): listed at 24, 74-84, now at 27, 77-87
- AD7: the compositors' frame, an Image of four f32 channels a pixel, and the CPU compositor's canvas the comp's layers are drawn onto (crates/render/src/pipeline.rs): listed at 656-661, now at 699-704
- AD7: OutputFormat::is_audio_only, and the export of a job of sound alone, which mixes its sound without rendering a picture (crates/export/src/webm.rs): listed at 328-363, now at 335-370
- AD7: FileExporter's public sink, which takes each finished file in memory in place of the file system, and the Sink type: image sequences call it from the render threads (crates/host/src/lib.rs): listed at 25-30, now at 28-33
- AD7: the WAV export of a job's sound: the module's sample rate clamped to 8000-192000, one or two channels, 16-bit samples, mixed in the comp's span (crates/export/src/webm.rs): listed at 328-363, 404-420, now at 335-370, 411-427
- AD7: wants_audio, which decides whether a movie has sound from the module's Audio setting and the comp (crates/render/src/audio.rs): listed at 37, now at 58
- AD7: craft_video::h264's Profile, RateControl, ColorConfig, PacketFormat, EncoderConfig, YuvFrame and Packet, the encoder's constructor, try_encode, flush, avcc and sps_pps, which the software encoder a hardware movie goes on in is made and fed through (storytold/craft-libs:crates/video/src/h264/encode/mod.rs): listed at 90-98, 114-119, 129-135, 205-211, now at 105-113, 129-134, 144-150, 232-238
- AD7: the software encoder's sequence parameter set, its frame numbers and picture order counts 8 bits each, and its parameter sets and slices all naming ids 0, which is why its sets are renamed beside a hardware encoder's, and its key pictures' `idr_pic_id` counting 0, 1, 0, 1, which is why they are flipped when the hardware's last picture is a key picture numbered 0 (storytold/craft-libs:crates/video/src/h264/encode/mod.rs): listed at 303-304, 846, 1034-1036, now at 361-362, 972, 1164-1166
- AD7: the software encoder's sequence parameter set, its frame numbers and picture order counts 8 bits each, and its parameter sets and slices all naming ids 0, which is why its sets are renamed beside a hardware encoder's, and its key pictures' `idr_pic_id` counting 0, 1, 0, 1, which is why they are flipped when the hardware's last picture is a key picture numbered 0 (storytold/craft-libs:crates/video/src/h264/encode/nal.rs): listed at 54, 133-134, 236-240, now at 56, 135-136, 238-242
- AD7: craft_video::h264::decode's Sps::parse, Pps::parse, NalHeader::parse, nal_type and SliceHeader::parse with its idr_pic_id and header_bits, the end of a slice header, and the sequence parameter set fields that place the idr_pic_id in it, which the software encoder's parameter sets and slices are read with before they are renamed (storytold/craft-libs:crates/video/src/h264/decode/params.rs): listed at 551, 577, now at 555, 581
- AD8: register_video_decoder (crates/codecs/src/lib.rs): listed at 99, now at 100
- AD8: video_decoder_registered (crates/codecs/src/lib.rs): listed at 107, now at 108
- AD8: software_video_decoder (crates/codecs/src/lib.rs): listed at 124, now at 125
- AD8: VideoDecoderFactory (crates/codecs/src/video.rs): listed at 182, now at 179
- AD8: CodecConfig::Av1, of the craft-media filmcraft-isobmff resolves, which the AV1 format description is made from (storytold/craft-libs:crates/media/src/isobmff/codec.rs): listed at 167, now at 172
- AD8: Av1Config::to_bytes, which writes the AV1 configuration record the VideoToolbox format description carries and the Android decoder starts MediaCodec with (storytold/craft-libs:crates/media/src/isobmff/codec.rs): listed at 466-483, now at 517-534
- AD8: Chroma, which the VideoToolbox and MediaCodec decoders reproduce in the frames they return (crates/frame/src/lib.rs): listed at 21-25, now at 25-29
- AD8: PixelData with its Yuv8 and Yuv16, which the VideoToolbox and MediaCodec decoders reproduce in the frames they return (crates/frame/src/lib.rs): listed at 46-55, now at 50-59
- AD8: VideoFrame, which the VideoToolbox and MediaCodec decoders reproduce in the frames they return (crates/frame/src/lib.rs): listed at 57-67, now at 61-71
- AD8: the GOP decode loop, which moves past a sample only once it decoded and feeds it again after an error (crates/codecs/src/gop.rs): listed at 778-779, now at 978-979
- AD8: AvcConfig, of the craft-media filmcraft-isobmff re-exports, whose fields and four-byte length size the hardware encoder's avcC record is built as (storytold/craft-libs:crates/media/src/isobmff/codec.rs): listed at 238-254, now at 289-305
- AD8: HevcConfig, of the same re-export, whose array-form fields and length size the hardware encoder's hvcC record is built as (storytold/craft-libs:crates/media/src/isobmff/codec.rs): listed at 311-328, 393-399, now at 362-379, 444-450
- AD9: Backend::headless (crates/automation/src/backend.rs): listed at 33, now at 40
- AD9: Backend::confine (crates/automation/src/backend.rs): listed at 45, now at 52
- AD9: confine::Confinement::new (crates/automation/src/confine.rs): listed at 177, now at 181
- AD9: check_command, which Backend::exec runs with the confinement before each command (crates/automation/src/confine.rs): listed at 284, now at 288
- AD9: check_lookup, which Backend::exec runs with the confinement before each command (crates/automation/src/confine.rs): listed at 392, now at 396
- AD9: the confinement's check of each step of an engine.batch before the batch runs, and its refusal of a step whose destination only a live lookup can tell (crates/automation/src/confine.rs): listed at 297-318, now at 301-322
- AD9: Backend::exec, the one place the confinement's check_command runs, which a script's app.run reaches the engine past, so no script runs on the phone (PD13) (crates/automation/src/backend.rs): listed at 70-82, now at 77-89
- AD9: RUNS_SCRIPTS and the confinement's refusal of every command that runs a script, which a script run on the session would bypass: its app.run reaches any command by id past Backend::exec (crates/script/src/prelude.js): listed at 2076, now at 2120
- AD10: effectcraft_automation::Error and what each variant displays (crates/automation/src/lib.rs): listed at 33, now at 34
- AD10: the refusals the confinement answers in, from Confinement::check, check_command and check_lookup (crates/automation/src/confine.rs): listed at 190-423, now at 194-427
- AD10: every sentence of the engine's that ends `not available in this build`, as a refusal, as a command's own and as why a command is disabled (crates/engine/src/cmd/file_more.rs): listed at 606, 1080, now at 630, 1104
- AD10: every sentence of the engine's that ends `not available in this build`, as a refusal, as a command's own and as why a command is disabled (crates/engine/src/cmd/file.rs): listed at 321, now at 328
- AD10: every sentence of the engine's that ends `not available in this build`, as a refusal, as a command's own and as why a command is disabled (crates/engine/src/system/library.rs): listed at 459, 760, 871, 926, now at 550, 851, 966, 1021
- AD10: the sentence the engine fails in when the disk will not read or write a file, `cannot read {file}: {why}` or `cannot write {file}: {why}`, as an EngineError::Other, which answers Io (crates/engine/src/cmd/animation.rs): listed at 133, now at 188
- AD10: the sentence the engine fails in when the disk will not read or write a file, `cannot read {file}: {why}` or `cannot write {file}: {why}`, as an EngineError::Other, which answers Io (crates/engine/src/cmd/file_more.rs): listed at 504, 519, now at 528, 543
- AD10: the EngineError::Other disk sentences that open with another verb, `cannot create`, `cannot inspect`, `cannot publish`, `cannot delete`, `cannot remove`, `cannot save` and `cannot install`, which stay CommandFailed (crates/engine/src/cmd/file_more.rs): listed at 438, now at 462
- AD11: the per-command refusal of the confinement, whose wording this mirrors (crates/automation/src/confine.rs): listed at 285-286, now at 289-290
- AD11: file.runScript, which runs the steps of a command script it reads from a file itself and which the confinement withholds as a command that runs scripts, so those steps never reach the engine (crates/engine/src/cmd/file_more.rs): listed at 515-549, now at 539-573
- AD11: the MCP tools that run a command by id and how each reads its command, params and steps (crates/automation/src/tools.rs): listed at 154-174, 202-206, 496-503, 954-967, 1012-1034, now at 166-186, 214-218, 585-592, 1046-1059, 1107-1129
- AD12: the adapter limits and the two required limits of GpuContext::request, and the defaults it asks for every other limit, which the core asks clamped to the adapter, and its device test unclamped to record whether a device takes them as they are (crates/gpu/src/context.rs): listed at 1430-1435, now at 1448-1453
- AD12: the words GpuContext::request reports no adapter in, which the core's reason follows (crates/gpu/src/context.rs): listed at 1426, now at 1444
- AD13: the get_layer tool, the arguments it reads and its refusal of a missing layer, and its spec, whose depth is a whole number (crates/automation/src/tools.rs): listed at 313-322, 633-648, now at 325-334, 722-737
- AD13: flat's list of leaf properties, which get_layer answers through flatten (crates/automation/src/tools.rs): listed at 288-311, now at 300-323
- AD13: the get_property tool, the arguments it reads and its refusals of a missing layer or path (crates/automation/src/tools.rs): listed at 324-327, 649-660, now at 336-339, 738-749
- AD13: the set_property tool, the arguments it reads and its refusal of neither value nor expression (crates/automation/src/tools.rs): listed at 329-358, 661-677, now at 341-370, 750-766
- AD13: the argument reads every tool makes, which drop an argument given as null and refuse a missing one as `missing `name`` (crates/automation/src/tools.rs): listed at 143-160, now at 155-172
- AD13: prop.get (crates/engine/src/cmd/prop.rs): listed at 1421, now at 1496
- AD13: prop.addKey (crates/engine/src/cmd/prop.rs): listed at 1432, now at 1507
- AD13: prop.setExpression (crates/engine/src/cmd/prop.rs): listed at 1443-1451, now at 1518-1526
- AD13: file.newProject and its result, null or `{template}` when the settings open a template (crates/engine/src/cmd/file.rs): listed at 790, now at 803
- AD13: file.open, its result `{path, savedBy, repaired}` and its refusals `missing `path`` and `{path} is not a project EffectCraft can open` (crates/engine/src/cmd/file.rs): listed at 792, now at 805
- AD13: file.save and its refusal `no path: use file.saveAs {path}` (crates/engine/src/cmd/file.rs): listed at 793, now at 806
- AD13: file.saveAs and its refusal `missing `path`` (crates/engine/src/cmd/file.rs): listed at 794, now at 807
- AD13: the confinement's check of a path, which refuses a relative path and `..` and resolves every link that exists before it compares the path with the roots (crates/automation/src/confine.rs): listed at 190-206, 656-666, now at 194-210, 660-670
- AD14: the desktop window's frame latency, LOW_LATENCY's one frame in flight and the reasons given for it, which the preview surface follows (apps/effectcraft/src/main.rs): listed at 475-482, now at 471-478
- AD16: the compositor's own handler installer, one failure path both callbacks call, and the words it gives a lost device and an uncaptured error, which the desktop app gives too (crates/gpu/src/readback.rs): listed at 348-362, now at 391-405
- AD16: the compositor's own handler installer, one failure path both callbacks call, and the words it gives a lost device and an uncaptured error, which the desktop app gives too (apps/effectcraft/src/main.rs): listed at 317-328, now at 329-340
- AD16: Backend::session, the way to the session the app built (crates/automation/src/backend.rs): listed at 61, now at 68
- AD17: the session fields the switch fills and the function types that bind them (crates/engine/src/lib.rs): listed at 207-215, 294, 311, now at 214-222, 301, 318
- AD17: the confinement's check of effect.plugins.load's path and folder and of the session's file reads, which refuse a relative path and a `..` component and resolve links before they read, so the source the loader is handed is a file the confinement let through, and one outside its roots is refused in its words (crates/automation/src/confine.rs): listed at 101, 190-206, 568-571, now at 106, 194-210, 572-575
- AD17: the Plug-in Library's other callers of the loader and the previewer, which the confinement withholds (crates/engine/src/system/library.rs): listed at 459, 760-761, 871, 926-927, now at 550, 851-852, 966, 1021-1022
- AD17: the Plug-in Library's other callers of the loader and the previewer, which the confinement withholds (crates/automation/src/confine.rs): listed at 69-75, now at 71-77
- AD18: the boa dependencies this unifies over: effectcraft-script's and effectcraft-expr's own (crates/expr/Cargo.toml): listed at 20, now at 21
- AD18: the workspace's boa version with its default features off, which the workspace's own entry copies so Android's boa adds only the layout (Cargo.toml): listed at 68, now at 81
- AD19: a movie's one file at the job's path, created with its parent folders, and nothing else while the session has no storage quota (crates/export/src/pipeline.rs): listed at 899-935, now at 913-949
- AD19: a movie's one file at the job's path, created with its parent folders, and nothing else while the session has no storage quota (crates/host/src/lib.rs): listed at 32-89, now at 35-92
- AD20: the session's background tasks: spawn_task, the poll that finishes and applies one, the job rows, the cancel by id and the record each leaves (crates/engine/src/background/jobs.rs): listed at 199-206, 222-279, 282-307, now at 200-207, 223-280, 283-308
- AD20: the session's background tasks: spawn_task, the poll that finishes and applies one, the job rows, the cancel by id and the record each leaves (crates/engine/src/lib.rs): listed at 246, 248, now at 253, 255
- AD20: Confinement::check, the write check the runner asks of the folder and each frame, in its own words (crates/automation/src/confine.rs): listed at 190-206, now at 194-210
- AD20: Confinement::check_command, which checks a command it does not list as a reader or writer only for the absolute paths it is given, as reads (crates/automation/src/confine.rs): listed at 319-324, 360-366, now at 323-328, 364-370
- AD20: ConfinedServices, whose writes pass to the session's own services unchecked (crates/automation/src/confine.rs): listed at 572-574, 581-583, now at 576-578, 585-587
- AD22: the Lottie import's images: each asset stored, when embedded, or else named by u and p resolved against the Lottie file's folder, an image whose store fails skipped before any item is made, the items made in the order of the assets (crates/lottie/src/import.rs): listed at 96-111, now at 88-103
- AD22: vector::is_vector, with which the engine tells vector footage (crates/engine/src/interchange/vector.rs): listed at 259-262, now at 276-279
- AD22: Session::new, and the project, path, state, services, importer and prefs fields the scratch session takes from the real one and the door reads (crates/engine/src/lib.rs): listed at 180, now at 187
- AD22: Backend::headless, confinement, session and exec, which run the probe on the scratch session and reach the real one (crates/automation/src/backend.rs): listed at 33-35, 53-55, 61-66, 69-81, now at 40-42, 60-62, 68-73, 76-88
- AD22: Access, Confinement::check and Confinement::check_command, which say whether the folder holding a source may be written and whether a call may run as it came, and which refuse a batch step's file path given as a $N reference, so every file a step reads is named in the call; check weighs a path resolved through every link, so the probe may read a template through a link of its own to it (crates/automation/src/confine.rs): listed at 190-206, 284-389, 656-666, now at 194-210, 288-393, 660-670
- AD22: ConfinedServices, whose reads are checked against the read roots and whose writes are not, so the probe reads what the session may read and the copy beside which a command writes is the only protection of a bundle (crates/automation/src/confine.rs): listed at 549-593, now at 553-597
- AD22: the MCP tools that run commands, which take params and steps given as JSON or as text holding it alike, so a tool call the door rewrote is written back as JSON (crates/automation/src/tools.rs): listed at 167-174, 202-206, 496-503, 1012-1034, now at 179-186, 214-218, 585-592, 1107-1129
- AD22: Footage::path and the project's items, which name the file a footage item shows (crates/project/src/lib.rs): listed at 60, now at 61
- AD22: the commands that read a project into a session beside `file.open`: `file.openRecent`, which opens one of the session's recent projects through it and runs only while there is one, and which the confinement runs only with its path given, `file.revert`, which opens the session's own path again through it, `file.newProject`, which reads the template project the settings name when they say to, `file.importProject`, which reads a project file the way opening one in the desktop app does, and `file.importAep`, which reads an After Effects project and its footage the same way (crates/automation/src/confine.rs): listed at 367-370, now at 371-374
- AD22: file.import and file.importMultiple, which read the paths parameter, note each path as recent footage and save the settings, which on a session with no settings store writes nothing, and hand each path the engine tells is a project to file.importProject and each .aep to file.importAep inside the command, leaving the rest to be probed as footage, and file.importRecent, which imports the recent file its path or index names through file.import, runs only while there is recent footage, and which the confinement runs only with its path given (crates/engine/src/cmd/file.rs): listed at 378-388, 410-428, now at 390-400, 422-440
- AD22: file.import and file.importMultiple, which read the paths parameter, note each path as recent footage and save the settings, which on a session with no settings store writes nothing, and hand each path the engine tells is a project to file.importProject and each .aep to file.importAep inside the command, leaving the rest to be probed as footage, and file.importRecent, which imports the recent file its path or index names through file.import, runs only while there is recent footage, and which the confinement runs only with its path given (crates/engine/src/cmd/file_more.rs): listed at 1224-1225, now at 1256-1257
- AD23: the confinement, which a reading may build over no folder to ask its refusals (crates/automation/src/confine.rs): listed at 177-181, now at 181-185
- AD23: Confinement::check_command, which answers a refused command in one sentence built from the id before it looks at any parameter (crates/automation/src/confine.rs): listed at 284-287, now at 288-291
- AD23: the headless backend, and the confinement it serves under, that the reading of the tools stands on (crates/automation/src/backend.rs): listed at 33-35, 45-51, now at 40-42, 52-58
- AD23: project.summary, comp.info, app.capabilities and editor.state, the engine queries that the tools taking no argument run, which read only the session (crates/engine/src/cmd/file.rs): listed at 760-778, now at 773-791
- AD23: Backend::exec, which checks each command with the confinement before it runs, and Backend::render_to_file, which checks its path before it renders (crates/automation/src/backend.rs): listed at 68-81, 126-131, now at 75-88, 133-138
- AD23: Confinement::check_command, which checks the file parameters a command names and every other absolute path it is given against the roots before the command runs (crates/automation/src/confine.rs): listed at 325-366, 628-652, now at 329-370, 632-656
- AD23: Confinement::check, which refuses a path outside every root, as a read and as a write (crates/automation/src/confine.rs): listed at 189-206, now at 193-210

### Commands added, changed and removed

```
compatible	changed	command	anim.applyPreset	params "{path | preset, layers?}" -> "{path | preset, layers?} — saved effects and text animators are added to each layer, with their keys at the CTI"
compatible	changed	command	anim.savePreset	params "{path (.ecpreset), name?} — saves the selected properties/effects" -> "{path (.ecpreset), name?} — saves the selected properties/effects; a text animator with a selected property is saved whole"
breaking	changed	command	edit.editOriginal	refusedUnderConfinement false -> true
compatible	changed	command	file.watchFolder	refusedUnderConfinement false -> true
compatible	changed	command	file.watchFolder.poll	refusedUnderConfinement false -> true
compatible	changed	command	library.install	params "{id, version?, wait?} — downloads the release, checks its SHA-256 against the catalog, loads it" -> "{id, version?, wait?} — downloads the release (a GitHub release asset, in the package\'s repo when it names one), checks its SHA-256 against the catalog, loads it"
compatible	changed	command	library.refresh	params "{catalog?: https:// or file:// URL of an index.json (default: the community catalog), wait?}" -> "{catalog?: https:// or file:// URL of an index.json (default: the community catalog; file:// takes a local path, not a network path such as //server/share), wait?}"
compatible	added	command	path.toggleClosed	
compatible	added	key	project.newFolder:items	
compatible	added	key	prop.set:addend	
compatible	added	key	prop.set:factor	
compatible	changed	tool	render_frame	description
compatible	changed	tool	render_frame	input `columns` added
compatible	changed	tool	render_frame	input `labels` added
compatible	changed	tool	render_frame	input `times` added
compatible	changed	tool	render_preview	description
compatible	changed	tool	render_preview	input `columns` added
compatible	changed	tool	render_preview	input `labels` added
compatible	changed	tool	render_preview	input `times` added
# 4 added, 0 removed, 15 changed, 1 breaking
```

