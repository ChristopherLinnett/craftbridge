//! The structure of a timing run: how many runs timed the frame, which layers were timed, at what depth,
//! and which effects on each, and for playback how many frames each pass timed and whether the layer
//! cache reported. The command line prints its benchmark as text, without a JSON form at the pin, and the
//! plug-in answers its timings as JSON, so each is read down to the same structure and the measurements,
//! which differ by machine and run, are left out.

use serde_json::Value;

/// One timed layer: its depth, its name and the ids of its effects in order.
pub(crate) type Row = (usize, String, Vec<String>);

/// What a timing run timed, apart from how long anything took.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Structure {
    /// How many runs timed the frame.
    pub(crate) runs: Option<u64>,
    pub(crate) layers: Vec<Row>,
    /// The playback timed, when it was asked for.
    pub(crate) play: Option<Play>,
}

/// The playback a timing run timed: the frames without the layer cache and with it, and whether the
/// cache's own counts came with them.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Play {
    pub(crate) no_cache: Option<u64>,
    pub(crate) layer_cache: Option<u64>,
    pub(crate) cache: bool,
}

/// The count a heading starts with, `<n> <unit>`.
fn count(text: &str, unit: &str) -> Option<u64> {
    let mut words = text.split_whitespace();
    let n = words.next()?.parse().ok()?;
    (words.next() == Some(unit)).then_some(n)
}

/// The structure of the command line's benchmark text: `bench (no cache): <n> runs …` heads the frame's
/// rows, a layer row is indented two spaces and two for each depth and reads `<name> process <figures>`,
/// an effect row under it starts `fx <id>`, and playback adds `bench-play (no cache): <n> frames …`,
/// `bench-play (layer cache): <n> frames …` and the cache's `layer cache: …` line.
pub(crate) fn from_text(text: &str) -> Structure {
    let mut structure = Structure::default();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(effect) = trimmed.strip_prefix("fx ") {
            if let (Some(row), Some(id)) = (structure.layers.last_mut(), effect.split_whitespace().next()) {
                row.2.push(id.to_string());
            }
        } else if let Some(heading) = trimmed.strip_prefix("bench (no cache): ") {
            structure.runs = count(heading, "runs");
        } else if let Some(heading) = trimmed.strip_prefix("bench-play (no cache): ") {
            structure.play.get_or_insert_default().no_cache = count(heading, "frames");
        } else if let Some(heading) = trimmed.strip_prefix("bench-play (layer cache): ") {
            structure.play.get_or_insert_default().layer_cache = count(heading, "frames");
        } else if let Some((name, _)) = trimmed.split_once(" process ") {
            let indent = line.len() - trimmed.len();
            structure.layers.push((indent.saturating_sub(2) / 2, name.trim_end().to_string(), Vec::new()));
        } else if trimmed.starts_with("layer cache: ") {
            structure.play.get_or_insert_default().cache = true;
        }
    }
    structure
}

/// The structure of the plug-in's `timings` answer: its runs, the layers it timed for the frame, and its
/// playback.
pub(crate) fn from_json(timings: &Value) -> Structure {
    let layers = timings.pointer("/frame/layers").and_then(Value::as_array);
    let layers = layers
        .into_iter()
        .flatten()
        .map(|layer| {
            let effects = layer
                .get("effects")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|e| e.get("id").and_then(Value::as_str).map(str::to_string))
                .collect();
            let depth = layer.get("depth").and_then(Value::as_u64).and_then(|depth| usize::try_from(depth).ok()).unwrap_or(0);
            (depth, layer.get("layer").and_then(Value::as_str).unwrap_or("").to_string(), effects)
        })
        .collect();
    let play = timings.get("play").map(|play| Play {
        no_cache: play.pointer("/noCache/frames").and_then(Value::as_u64),
        layer_cache: play.pointer("/layerCache/frames").and_then(Value::as_u64),
        cache: play.get("cache").is_some(),
    });
    Structure { runs: timings.get("runs").and_then(Value::as_u64), layers, play }
}

/// A count as a failure names it.
fn shown(count: Option<u64>) -> String {
    count.map_or_else(|| "none".to_string(), |count| count.to_string())
}

/// A playback as a failure names it.
fn played(play: Option<&Play>) -> String {
    match play {
        None => "no playback".to_string(),
        Some(play) => format!(
            "{} frames without the cache and {} with it, {}",
            shown(play.no_cache),
            shown(play.layer_cache),
            if play.cache { "with the cache's counts" } else { "without the cache's counts" }
        ),
    }
}

/// The failures of the plug-in's structure against the command line's; none when the same runs, layers,
/// effects and playback were timed. A frame read from nothing is a failure of its own: a command line
/// whose text changed would otherwise match an answer that timed nothing.
pub(crate) fn failures(oracle: &Structure, plugin: &Structure) -> Vec<String> {
    if oracle.runs.is_none() || oracle.layers.is_empty() {
        return vec!["the command line's benchmark has no runs heading or no layer rows: its text may have changed".to_string()];
    }
    let mut failures = Vec::new();
    if oracle.runs != plugin.runs {
        failures.push(format!("the command line timed the frame over {} runs, the plug-in over {}", shown(oracle.runs), shown(plugin.runs)));
    }
    if oracle.layers != plugin.layers {
        let at = oracle.layers.iter().zip(&plugin.layers).position(|(a, b)| a != b).unwrap_or_else(|| oracle.layers.len().min(plugin.layers.len()));
        failures.push(format!(
            "the layers and effects timed differ from the command line's at row {at}: the command line's {} rows against the plug-in's {}",
            oracle.layers.len(),
            plugin.layers.len()
        ));
    }
    if oracle.play != plugin.play {
        failures.push(format!(
            "the playback timed differs: the command line's {} against the plug-in's {}",
            played(oracle.play.as_ref()),
            played(plugin.play.as_ref())
        ));
    }
    failures
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TEXT: &str = "bench (no cache): 2 runs at scale 1: wall min 1.0 ms\n  Lower Third process 1.2 ms\n    fx ec.blur 0.3 ms\n    fx ec.glow 0.1 ms\n    Inner process 0.4 ms\n  Back process 0.5 ms\nbench-play (no cache): 3 frames at scale 1: wall mean 1.0 ms\nbench-play (layer cache): 3 frames at scale 1: wall mean 1.0 ms\n  layer cache: 3 hits, 7 misses, 7 entries, 78.4 MB\n";

    fn answer() -> Value {
        json!({"runs": 2, "frame": {"layers": [
            {"layer": "Lower Third", "depth": 0, "ms": 9.0, "effects": [{"id": "ec.blur", "ms": 5}, {"id": "ec.glow"}]},
            {"layer": "Inner", "depth": 1},
            {"layer": "Back", "depth": 0, "effects": []}]},
            "play": {"noCache": {"frames": 3, "wallMeanMs": 4.0}, "layerCache": {"frames": 3}, "cache": {"hits": 1}}})
    }

    #[test]
    fn the_text_reads_as_its_runs_its_layers_by_depth_with_their_effects_and_its_playback() {
        let structure = from_text(TEXT);
        assert_eq!(structure.runs, Some(2));
        assert_eq!(
            structure.layers,
            [
                (0, "Lower Third".to_string(), vec!["ec.blur".to_string(), "ec.glow".to_string()]),
                (1, "Inner".to_string(), vec![]),
                (0, "Back".to_string(), vec![])
            ]
        );
        assert_eq!(structure.play, Some(Play { no_cache: Some(3), layer_cache: Some(3), cache: true }));
        assert_eq!(from_text("bench (no cache): 2 runs\n  A process 1 ms\n").play, None, "no playback heading, no playback");
        assert_eq!(from_text("bench (no cache): two runs\n").runs, None, "a heading whose count is no number counts nothing");
    }

    #[test]
    fn the_answer_reads_as_the_same_structure_and_no_measurement_counts() {
        assert_eq!(failures(&from_text(TEXT), &from_json(&answer())), Vec::<String>::new());
    }

    #[test]
    fn a_layer_an_effect_or_a_depth_that_differs_is_named_and_empty_text_is_a_failure() {
        let oracle = from_text(TEXT);
        let other = from_json(&json!({"runs": 2, "frame": {"layers": [{"layer": "Lower Third", "depth": 0, "effects": [{"id": "ec.blur"}]}]}}));
        assert!(failures(&oracle, &other).iter().any(|f| f.contains("at row 0")));
        let mut deeper = oracle.clone();
        if let Some(row) = deeper.layers.get_mut(1) {
            row.0 = 2;
        }
        assert!(failures(&oracle, &deeper).iter().any(|f| f.contains("at row 1")));
        let mut fewer = oracle.clone();
        fewer.layers.truncate(2);
        assert!(failures(&oracle, &fewer).iter().any(|f| f.contains("3 rows against the plug-in's 2")));
        assert!(failures(&Structure::default(), &Structure::default()).iter().any(|f| f.contains("no runs heading or no layer rows")));
        let unheaded = Structure { runs: None, ..oracle.clone() };
        assert!(failures(&unheaded, &oracle).iter().any(|f| f.contains("no runs heading")));
        assert_eq!(from_json(&json!({})), Structure::default());
    }

    #[test]
    fn the_runs_and_the_playback_are_held_to_the_command_lines() {
        let oracle = from_text(TEXT);
        let mut answer = answer();
        answer["runs"] = json!(3);
        assert_eq!(failures(&oracle, &from_json(&answer)), ["the command line timed the frame over 2 runs, the plug-in over 3"]);
        let mut no_play = self::answer();
        if let Some(entries) = no_play.as_object_mut() {
            entries.remove("play");
        }
        assert_eq!(
            failures(&oracle, &from_json(&no_play)),
            [
                "the playback timed differs: the command line's 3 frames without the cache and 3 with it, with the cache's counts against the plug-in's no playback"
            ]
        );
        let mut fewer_frames = self::answer();
        fewer_frames["play"]["layerCache"]["frames"] = json!(2);
        assert!(failures(&oracle, &from_json(&fewer_frames)).iter().any(|f| f.contains("3 with it") && f.contains("2 with it")));
        let mut no_cache = self::answer();
        if let Some(entries) = no_cache["play"].as_object_mut() {
            entries.remove("cache");
        }
        assert!(failures(&oracle, &from_json(&no_cache)).iter().any(|f| f.ends_with("without the cache's counts")));
    }
}
