//! Every file a render may write (AD19), so the export's pending-export journal can name them before
//! the first write.
//!
//! EffectCraft resolves the files of a render in `Session::outputs_of`, which is crate-private, so
//! this follows its rules over the public parts it is made of: the module's resolved path, the
//! bit-depth suffix, the segments, then what the exporter writes for each — one file, or a numbered
//! file per frame of a sequence — and the render log beside it. A file the render would never write
//! is listed harmlessly (the journal deletes only what an export changed), and one it does write and
//! the list lacks is left behind by a kill, so every rule here leans to listing more.

use std::path::PathBuf;

use craftbridge_core::Result;
use effectcraft_engine::Session;
use effectcraft_engine::project::render_queue::{OutputFormat, OutputModule, RenderQueueItem, log_path, sequence_path};
use effectcraft_engine::render_queue::output_bits;

/// How many paths go to the journal in one listing. Each listing rewrites the journal, so a render of
/// a very long sequence is listed in a few calls, and the journal's own size bound refuses what is
/// too long to hold without this ever collecting the whole sequence in memory.
const CHUNK: usize = 8192;

/// Hand `list` every file the next render of the queued items may write, [`CHUNK`] paths at a time,
/// and stop at the first refusal `list` answers.
pub(crate) fn list_queued(session: &Session, list: &mut dyn FnMut(&[PathBuf]) -> Result<()>) -> Result<()> {
    let formats = session.exporter.as_ref().map(|exporter| exporter.formats()).unwrap_or_default();
    let mut chunk = Vec::with_capacity(CHUNK);
    for path in queued_files(session, &formats) {
        chunk.push(PathBuf::from(path));
        if chunk.len() == CHUNK {
            list(&chunk)?;
            chunk.clear();
        }
    }
    if chunk.is_empty() { Ok(()) } else { list(&chunk) }
}

/// The files of every item the next render takes, in the queue's order: the items `Session::start_render`
/// takes are the queued ones, `RenderQueueItem::is_queued`.
fn queued_files<'a>(session: &'a Session, formats: &'a [OutputFormat]) -> impl Iterator<Item = String> + 'a {
    session
        .project
        .render_queue
        .iter()
        .filter(|item| item.is_queued())
        .filter_map(move |item| outputs_of(session, item, formats))
        .flatten()
        .flat_map(move |(segment, path)| files_of(session, segment, path))
}

/// The (segment, path) outputs of one item, as `Session::outputs_of` makes them, or none when the
/// render refuses the item (an output module with no output, a missing comp, a format the exporter
/// does not write): the render writes none of its modules then, so the list names none.
fn outputs_of(session: &Session, item: &RenderQueueItem, formats: &[OutputFormat]) -> Option<Vec<(RenderQueueItem, String)>> {
    let mut outputs = Vec::new();
    for module in item.output_modules() {
        let as_module = RenderQueueItem { output: module.clone(), ..item.clone() };
        let path = session.resolve_output(&as_module)?;
        if !formats.contains(&module.format) {
            return None;
        }
        let named = session.prefs.output_name(&path, bits_of(session, module));
        outputs.extend(session.segments(&as_module, &named));
    }
    Some(outputs)
}

/// The bit depth that Append Bit Depth to File Name writes into the name: a module's own half-float
/// choice for EXR, else the project's depth as the format keeps it.
fn bits_of(session: &Session, module: &OutputModule) -> u32 {
    if module.format == OutputFormat::ExrSequence { module.exr_bit_depth.bits() } else { output_bits(module.format, session.project.settings.bit_depth) }
}

/// The files the exporter writes for one output: the numbered file of each frame of a sequence, which
/// is numbered from the first frame of the item's span (`sequence_path`), or the one file of a
/// movie, and the render log beside it, which the exporter writes whenever the item's Log setting
/// asks for one or the render fails, so it is listed for every output.
fn files_of(session: &Session, segment: RenderQueueItem, path: String) -> impl Iterator<Item = String> {
    let sequence = segment.output.format.is_sequence();
    let (first, count) = session.project.comp(segment.comp).map_or((0, 0), |comp| (segment.settings.first_frame(comp), segment.settings.frame_count(comp)));
    let log = log_path(&path);
    let numbered = path.clone();
    let frames = (0..if sequence { count } else { 0 }).map(move |k| sequence_path(&numbered, first.saturating_add(i64::try_from(k).unwrap_or(i64::MAX))));
    frames.chain((!sequence).then_some(path)).chain(std::iter::once(log))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::export;
    use crate::session;
    use craftbridge_core::{CraftError, Platform};
    use effectcraft_automation::Backend;
    use serde_json::{Value, json};
    use std::collections::BTreeSet;
    use std::path::Path;

    /// A project of one solid comp, rendered by EffectCraft's own file exporter, so each rule here is
    /// held against the files a real render writes.
    struct Rig {
        backend: Backend,
        dir: tempfile::TempDir,
    }

    fn rig(width: u32, height: u32, seconds: f64) -> Rig {
        let dir = tempfile::tempdir().expect("a scratch folder");
        let mut backend = Backend::headless(session::build(Platform::current(), &session::tests::folders(None)));
        for (command, params) in [
            ("comp.new", json!({"name": "A", "width": width, "height": height, "duration": seconds, "frameRate": 10})),
            ("layer.newSolid", json!({"comp": 1, "name": "Bg"})),
        ] {
            backend.exec(command, params).expect("the project builds");
        }
        Rig { backend, dir }
    }

    impl Rig {
        fn path(&self, name: &str) -> String {
            self.dir.path().join(name).display().to_string()
        }

        fn exec(&mut self, command: &str, params: Value) {
            self.backend.exec(command, params).unwrap_or_else(|error| panic!("{command}: {error}"));
        }

        fn pref(&mut self, key: &str, value: Value) {
            self.exec("prefs.set", json!({"key": key, "value": value}));
        }

        /// Every file below the scratch folder, by its path from it.
        fn written(&self) -> BTreeSet<String> {
            files_below(self.dir.path())
        }

        /// Run `export` with a lister that records every path it is handed, and answer what was
        /// listed and what was written once the render has ended.
        fn export(&mut self, params: &Value) -> (BTreeSet<String>, BTreeSet<String>) {
            let mut listed = Vec::new();
            let before = self.written();
            let dir = self.dir.path().to_path_buf();
            export(&mut self.backend, params, &mut |_, _, _| true, &mut |paths| {
                assert_eq!(files_below(&dir), before, "every file is listed before the render writes the first");
                listed.extend(paths.iter().cloned());
                Ok(())
            })
            .expect("the render succeeds");
            (listed.iter().map(|path| relative(&self.dir, path)).collect(), self.written())
        }

        /// Queue `item` with its Log setting on and render the queue, so the exporter writes the
        /// render log of each output and every listed name is checked against a file it wrote.
        fn export_logged(&mut self, mut item: Value) -> (BTreeSet<String>, BTreeSet<String>) {
            item.as_object_mut().expect("an item's parameters").insert("log".into(), json!("plusSettings"));
            self.exec("renderQueue.add", item);
            self.export(&json!({"queue": true}))
        }
    }

    fn relative(dir: &tempfile::TempDir, path: &Path) -> String {
        path.strip_prefix(dir.path()).expect("a path below the scratch folder").display().to_string()
    }

    fn files_below(root: &Path) -> BTreeSet<String> {
        let mut found = BTreeSet::new();
        let mut folders = vec![root.to_path_buf()];
        while let Some(folder) = folders.pop() {
            for entry in std::fs::read_dir(&folder).expect("a folder reads").flatten() {
                let path = entry.path();
                if path.is_dir() {
                    folders.push(path);
                } else {
                    found.insert(path.strip_prefix(root).expect("a path below the root").display().to_string());
                }
            }
        }
        found
    }

    fn names(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| (*name).to_string()).collect()
    }

    /// What a render wrote is all listed, and what is listed but not written is the render logs,
    /// which an item whose Log setting is Errors Only writes only when it fails.
    fn assert_listed(listed: &BTreeSet<String>, written: &BTreeSet<String>, unwritten: &[&str]) {
        assert_eq!(listed.difference(written).cloned().collect::<BTreeSet<_>>(), names(unwritten), "listed but not written");
        assert_eq!(written.difference(listed).collect::<Vec<_>>(), Vec::<&String>::new(), "written but not listed");
    }

    #[test]
    fn a_movie_is_its_one_file_and_a_log() {
        let mut rig = rig(64, 48, 1.0);
        let output = rig.path("a.mp4");
        let (listed, written) = rig.export(&json!({"output": output, "format": "h264", "end": 0.5}));
        assert_eq!(written, names(&["a.mp4"]));
        assert_listed(&listed, &written, &["a_RenderLog.txt"]);
        let mut logged = self::rig(64, 48, 1.0);
        let output = logged.path("b.mp4");
        let (listed, written) = logged.export_logged(json!({"comp": 1, "output": output, "format": "h264", "end": 0.5}));
        assert_eq!(written, names(&["b.mp4", "b_RenderLog.txt"]));
        assert_listed(&listed, &written, &[]);
    }

    #[test]
    fn a_sequence_asked_for_at_a_plain_name_writes_numbered_files_never_that_name() {
        let mut rig = rig(64, 48, 1.0);
        let output = rig.path("card.png");
        let (listed, written) = rig.export(&json!({"output": output, "format": "png", "end": 0.5}));
        assert_eq!(written, names(&["card_00000.png", "card_00001.png", "card_00002.png", "card_00003.png", "card_00004.png"]));
        assert_listed(&listed, &written, &["card_RenderLog.txt"]);
        assert!(!listed.contains("card.png"), "the plain name is never written");
    }

    #[test]
    fn a_sequence_numbers_from_the_first_frame_of_its_span() {
        let mut rig = rig(64, 48, 1.0);
        let output = rig.path("late_[###].png");
        let (listed, written) = rig.export(&json!({"output": output, "format": "png", "start": 0.3, "end": 0.5}));
        assert_eq!(written, names(&["late_003.png", "late_004.png"]));
        assert_listed(&listed, &written, &["late_RenderLog.txt"]);
    }

    #[test]
    fn a_segmented_sequence_is_a_folder_of_files_per_segment_each_with_its_log() {
        let mut rig = rig(64, 48, 1.0);
        rig.pref("export.segmentSequences", json!(true));
        rig.pref("export.segmentSequenceFiles", json!(2));
        let output = rig.path("seg.png");
        let (listed, written) = rig.export_logged(json!({"comp": 1, "output": output, "format": "png", "end": 0.5}));
        assert_eq!(
            written,
            names(&[
                "seg_001/seg_00000.png",
                "seg_001/seg_00001.png",
                "seg_001/seg_RenderLog.txt",
                "seg_002/seg_00002.png",
                "seg_002/seg_00003.png",
                "seg_002/seg_RenderLog.txt",
                "seg_003/seg_00004.png",
                "seg_003/seg_RenderLog.txt",
            ])
        );
        assert_listed(&listed, &written, &[]);
    }

    #[test]
    fn a_segmented_movie_is_a_numbered_file_per_segment() {
        // Uncompressed 320 by 240 is 230,400 bytes a frame, so one megabyte holds four of the ten frames.
        let mut rig = rig(320, 240, 1.0);
        rig.pref("export.segmentMovies", json!(true));
        rig.pref("export.segmentMovieMb", json!(1));
        let output = rig.path("m.mov");
        let (listed, written) = rig.export_logged(json!({"comp": 1, "output": output, "format": "uncompressed"}));
        assert_eq!(written, names(&["m_001.mov", "m_001_RenderLog.txt", "m_002.mov", "m_002_RenderLog.txt", "m_003.mov", "m_003_RenderLog.txt"]));
        assert_listed(&listed, &written, &[]);
    }

    #[test]
    fn the_bit_depth_suffix_follows_the_format_and_the_modules_own_depth() {
        let mut rig = rig(64, 48, 1.0);
        rig.pref("export.appendBitsToName", json!(true));
        let (movie, png, half, float) = (rig.path("a.mp4"), rig.path("p.png"), rig.path("h.exr"), rig.path("f.exr"));
        let mut all = BTreeSet::new();
        for item in [
            json!({"comp": 1, "output": movie, "format": "h264", "end": 0.1}),
            json!({"comp": 1, "output": png, "format": "png", "end": 0.1}),
            json!({"comp": 1, "output": half, "format": "exr", "exrBitDepth": "half", "end": 0.1}),
            json!({"comp": 1, "output": float, "format": "exr", "exrBitDepth": "float", "end": 0.1}),
        ] {
            let before = rig.written();
            let (listed, written) = rig.export_logged(item);
            let new: BTreeSet<String> = written.difference(&before).cloned().collect();
            assert_listed(&listed, &new, &[]);
            all.extend(new);
        }
        assert_eq!(
            all,
            names(&[
                "a_8bpc.mp4",
                "a_8bpc_RenderLog.txt",
                "p_8bpc_00000.png",
                "p_8bpc_RenderLog.txt",
                "h_16bpc_00000.exr",
                "h_16bpc_RenderLog.txt",
                "f_32bpc_00000.exr",
                "f_32bpc_RenderLog.txt",
            ])
        );
    }

    #[test]
    fn a_queue_lists_each_queued_item_and_each_output_module_but_never_an_unticked_item() {
        let mut rig = rig(64, 48, 1.0);
        let (first, second, extra, unticked) = (rig.path("first.mp4"), rig.path("second.png"), rig.path("extra.png"), rig.path("unticked.png"));
        rig.exec("renderQueue.add", json!({"comp": 1, "format": "h264", "output": first, "end": 0.2}));
        rig.exec("renderQueue.add", json!({"comp": 1, "format": "png", "output": second, "end": 0.2, "log": "plusSettings"}));
        rig.exec("render.addOutputModule", json!({"item": 2, "format": "png", "output": extra}));
        rig.exec("renderQueue.add", json!({"comp": 1, "format": "png", "output": unticked, "end": 0.2}));
        rig.exec("renderQueue.setRender", json!({"index": 3, "render": false}));
        let (listed, written) = rig.export(&json!({"queue": true}));
        assert_eq!(
            written,
            names(&["first.mp4", "second_00000.png", "second_00001.png", "extra_00000.png", "extra_00001.png", "second_RenderLog.txt", "extra_RenderLog.txt"])
        );
        assert_listed(&listed, &written, &["first_RenderLog.txt"]);
    }

    #[test]
    fn an_item_the_render_refuses_lists_nothing() {
        let mut rig = rig(64, 48, 1.0);
        let output = rig.path("x.mp4");
        rig.exec("renderQueue.add", json!({"comp": 1, "format": "h264", "output": output}));
        let mut calls = 0;
        let session = rig.backend.session().expect("a session");
        // The commands never leave an item with no output, so the project is edited as a document would be.
        let project = std::sync::Arc::make_mut(&mut session.project);
        for item in &mut project.render_queue {
            item.output.output.clear();
        }
        list_queued(session, &mut |_| {
            calls += 1;
            Ok(())
        })
        .expect("nothing to list is no refusal");
        assert_eq!(calls, 0, "an item with no output is refused by the render, which writes none of it");
    }

    #[test]
    fn a_long_sequence_is_listed_in_chunks_until_the_list_refuses() {
        // A render this long is never started: the lister refuses its second chunk, which is how the
        // journal answers a list it cannot hold, and the listing stops there without the rest.
        let mut rig = rig(16, 16, 100_000.0);
        let output = rig.path("long.png");
        rig.exec("renderQueue.add", json!({"comp": 1, "format": "png", "output": output}));
        let mut sizes = Vec::new();
        let session = rig.backend.session().expect("a session");
        let refused = list_queued(session, &mut |paths| {
            sizes.push(paths.len());
            if sizes.len() == 2 { Err(CraftError::io("the journal is full")) } else { Ok(()) }
        });
        assert_eq!(refused, Err(CraftError::io("the journal is full")));
        assert_eq!(sizes, [CHUNK, CHUNK]);
    }
}
