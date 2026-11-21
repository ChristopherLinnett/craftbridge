//! A release's own figures: the runs the owner measured of a release on the acceptance phones, kept in one
//! file, and the one rule `conformance/effects/baseline.toml` holds them to (plan §9, §29). `cargo xtask
//! release-figures add <file> <device id> <commit> <run output>` takes the one line a measuring test printed
//! out of what its runner printed and adds it to the file, and `cargo xtask release-figures judge <file>
//! <commit>` judges the file, so the file is written and read by one definition, the release's notes quote
//! the verdict, and no script writes, reads or weighs a figure itself.
//!
//! The file is what the measuring tests print and nothing else, one run to a line, under the commit they
//! measured:
//!
//! ```text
//! commit <the full commit the release tags>
//! <device id> <line> <figure>=<value> ... <repeats word>=<v>,<v>,...
//! ```
//!
//! A run's line is read whole against the definition: its figures stand in the order of the file's
//! `[[figure]]` tables for the device's platform, each value in decimal with its figure's `decimals`, then
//! as many warm repeats as the running example makes, each with `repeats_decimals`. So a line a test did not
//! print, or that a rename of a figure left behind, is refused rather than read as a measurement. Every run
//! prints its own repeats, so no two runs print one line: a line standing twice is one run counted twice,
//! and is refused too. A file measured at another commit is no measurement of this release, and says so.
//!
//! The rule is the file's own `[comparison]`: a `flagged` figure on a platform its `unheld_on` does not name
//! is flagged when the median of at least `runs` runs is more than `flag_slower_than_percent` above the
//! record's median and the fastest of them is slower than the record's slowest run. Every other figure is
//! reported beside the record's, a figure on an `unheld_on` platform included.
//!
//! The judge prints one `device` line for each phone of the record, `device<TAB>id<TAB>platform<TAB>runs
//! measured`, then one `figure` line for each figure the phone's runs carry, `figure<TAB>id<TAB>name<TAB>
//! median<TAB>fastest<TAB>record's median<TAB>record's slowest<TAB>percent<TAB>verdict` (the percent is
//! `n/a` over a record of nothing), the verdict one of `flagged`, `within`, `unheld` (reported on a platform
//! that is never flagged), `uncompared` (fewer runs than the comparison asks for) and `reported` (a figure no
//! release is held to). A file measured at another commit prints the one line `another<TAB>commit`.

use crate::baseline::{Definition, Figure, Record, Repeats};
use std::path::Path;
use std::process::ExitCode;

/// How a figure's runs stand against the record's, in the word a note prints.
const FLAGGED: &str = "flagged";
const WITHIN: &str = "within";
const UNHELD: &str = "unheld";
const UNCOMPARED: &str = "uncompared";
const REPORTED: &str = "reported";

/// The word a measurement's first line names the commit it measured with.
const MEASURED: &str = "commit";

/// The median of a figure's runs, the middle one or the mean of the middle two: how the record's own
/// medians are taken.
fn median(values: &[f64]) -> Option<f64> {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    if sorted.len() % 2 == 1 { sorted.get(middle).copied() } else { Some((sorted.get(middle.checked_sub(1)?)? + sorted.get(middle)?) / 2.0) }
}

/// One run, as its line printed it: the device it ran on and each figure's value, in the order of the file's
/// figures for that device's platform.
struct Run<'a> {
    record: &'a Record,
    values: Vec<(&'a Figure, f64)>,
}

/// A value as a measuring test prints it, `decimals` places in decimal and a minus its only sign, and
/// nothing for any other spelling of it (`2800` or `2800.00` for `2800.0`, `2.8e3`, `+2800.0`), which no run
/// printed.
fn printed(word: &str, decimals: usize) -> Option<f64> {
    word.parse::<f64>().ok().filter(|value| value.is_finite() && format!("{value:.decimals$}") == word)
}

/// A run's line, read whole: every word of it is one the definition names, in its order and with its
/// decimals, so a line that is short, long, out of order or printed otherwise is refused and never read as
/// a run of fewer figures.
fn run_of<'a>(definition: &'a Definition, repeats: &Repeats, line: &str) -> Result<Run<'a>, String> {
    let mut words = line.split(' ');
    let id = words.next().unwrap_or_default();
    let record = definition
        .records
        .iter()
        .find(|record| record.id == id)
        .ok_or_else(|| format!("`{line}` names the device `{id}`, which no record of {} holds", crate::baseline::BASELINE))?;
    let refused = |what: &str| format!("the run `{line}` of {id} {what}");
    if words.next() != Some(definition.line.as_str()) {
        return Err(refused(&format!("does not begin `{} {}`, the line a run prints", id, definition.line)));
    }
    let mut values = Vec::new();
    for figure in definition.figures.iter().filter(|figure| figure.platforms.contains(&record.platform)) {
        let word = words.next().unwrap_or_default();
        let expected = || {
            let (name, decimals) = (&figure.name, figure.decimals);
            refused(&format!("carries `{word}` where `{name}=<v>` stands next, with {decimals} decimals, the figures standing in the file's order"))
        };
        let value = word.strip_prefix(figure.name.as_str()).and_then(|rest| rest.strip_prefix('=')).and_then(|number| printed(number, figure.decimals));
        let value = value.ok_or_else(expected)?;
        values.push((figure, value));
    }
    let word = words.next().unwrap_or_default();
    let held = word.strip_prefix(repeats.word.as_str()).and_then(|rest| rest.strip_prefix('=')).is_some_and(|list| {
        let each: Vec<&str> = if list.is_empty() { Vec::new() } else { list.split(',').collect() };
        each.len() == repeats.count && each.iter().all(|repeat| printed(repeat, repeats.decimals).is_some())
    });
    if !held {
        return Err(refused(&format!(
            "carries `{word}` where the running example's {} warm repeats stand after its figures, `{}=<v>,<v>,...` with {} decimals",
            repeats.count, repeats.word, repeats.decimals
        )));
    }
    if words.next().is_some() {
        return Err(refused("carries a word after its warm repeats"));
    }
    Ok(Run { record, values })
}

/// A commit named in full, as git prints it: 40 lower-case hex digits.
fn full_commit(commit: &str) -> bool {
    commit.len() == 40 && commit.chars().all(|digit| digit.is_ascii_digit() || ('a'..='f').contains(&digit))
}

/// A measurement's file read whole: the commit its first line says it measured, and every run under it,
/// each line once.
fn measurement<'d, 's>(definition: &'d Definition, source: &'s str) -> Result<(&'s str, Vec<Run<'d>>), String> {
    let repeats = definition.repeats.as_ref().map_err(|reason| format!("{reason}, so a run's line cannot be read whole"))?;
    let mut lines = source.lines();
    let first = lines.next().unwrap_or_default();
    let measured = first
        .strip_prefix(MEASURED)
        .and_then(|rest| rest.strip_prefix(' '))
        .filter(|commit| full_commit(commit))
        .ok_or_else(|| format!("the first line is `{first}`, not `{MEASURED} <the full commit the release tags>`: a measurement says what it measured"))?;
    let mut read: Vec<&str> = Vec::new();
    let mut runs = Vec::new();
    for line in lines {
        if read.contains(&line) {
            return Err(format!(
                "the run `{line}` stands twice: every run prints its own warm repeats, so no two runs print one line, and one line twice is one run counted as two"
            ));
        }
        runs.push(run_of(definition, repeats, line)?);
        read.push(line);
    }
    Ok((measured, runs))
}

/// A value in half units of its figure's last decimal, or nothing for one that is not a whole number of
/// them. A run prints each value with its figure's decimals and a median is one run's value or the mean of
/// two, the record's as a release's, so every value the rule weighs is a whole number of half units: the
/// rule is weighed in those whole numbers, so no rounding of a fraction decides a figure at the allowance's
/// very edge. Past 2^53 a float holds no whole number exactly, so a value that large is none either.
fn half_units(value: f64, decimals: usize) -> Option<i128> {
    let scaled = value * 2.0 * 10f64.powi(i32::try_from(decimals).ok()?);
    let whole = scaled.round();
    ((scaled - whole).abs() < 1e-6 && whole.abs() < 9_007_199_254_740_992.0).then_some(whole as i128)
}

/// A value in half units, printed exactly: with its figure's decimals, and one more where it is a half, so a
/// median at the allowance and one a decimal past it never read alike in a note.
fn said(half_units: i128, decimals: usize) -> String {
    let places = if half_units % 2 == 0 { decimals } else { decimals + 1 };
    format!("{:.places$}", half_units as f64 / 2.0 / 10f64.powi(decimals as i32))
}

/// Whether `median` is more than the allowance above `record`, both in half units: weighed exactly, with
/// the allowance as the notes print it, in whole units of its own last decimal (`12.5` is 125 tenths).
fn over_the_allowance(median: i128, record: i128, allowance: f64) -> Option<bool> {
    let written = crate::baseline::amount(allowance);
    let places = written.split_once('.').map_or(0, |(_, fraction)| fraction.len());
    let scale = 10i128.checked_pow(u32::try_from(places).ok()?)?;
    let allowed = half_units(allowance, places)?.checked_div(2)?;
    Some(median.checked_sub(record)?.checked_mul(100)?.checked_mul(scale)? > record.checked_mul(allowed)?)
}

/// One figure's line for one phone: the runs' median and fastest beside the record's, the change of the
/// median over the record's, and what the rule makes of it.
fn figure_line(definition: &Definition, record: &Record, figure: &Figure, runs: &[f64]) -> Result<String, String> {
    let spread = record
        .spreads
        .iter()
        .find(|spread| spread.name == figure.name)
        .ok_or_else(|| format!("{}: the record holds no `{}` to set the release's runs beside", record.id, figure.name))?;
    let middle = median(runs).ok_or_else(|| format!("{}: `{}` has no run to take a median of", record.id, figure.name))?;
    let fastest = runs.iter().copied().fold(f64::INFINITY, f64::min);
    let units = |value: f64, what: &str| {
        half_units(value, figure.decimals).ok_or_else(|| {
            format!("{}: `{}`'s {what}, {value}, is no value a run prints with {} decimals, nor a median of such", record.id, figure.name, figure.decimals)
        })
    };
    let (median_units, fastest_units) = (units(middle, "median of the release's runs")?, units(fastest, "fastest run")?);
    let (record_units, slowest_units) = (units(spread.median, "record's median")?, units(spread.max, "record's slowest run")?);
    let over = over_the_allowance(median_units, record_units, definition.allowance)
        .ok_or_else(|| format!("{}: `{}` cannot be weighed against an allowance of {} %", record.id, figure.name, definition.allowance))?;
    // A record of nothing has no percent to be over: the same nothing is no change, and anything else has no
    // percentage to say.
    let percent = if record_units != 0 {
        // A change under the printed decimal is no change, whichever side of the record it fell on.
        format!("{:+.1}", (median_units - record_units) as f64 * 100.0 / record_units as f64).replace("-0.0", "+0.0")
    } else if median_units == 0 {
        "+0.0".to_string()
    } else {
        "n/a".to_string()
    };
    let verdict = if !figure.flagged {
        REPORTED
    } else if figure.unheld_on.contains(&record.platform) {
        UNHELD
    } else if (runs.len() as i64) < definition.comparison_runs {
        UNCOMPARED
    } else if over && fastest_units > slowest_units {
        FLAGGED
    } else {
        WITHIN
    };
    let [median, fastest, record_median, slowest] = [median_units, fastest_units, record_units, slowest_units].map(|units| said(units, figure.decimals));
    Ok(format!("figure\t{}\t{}\t{median}\t{fastest}\t{record_median}\t{slowest}\t{percent}\t{verdict}", record.id, figure.name))
}

/// What the owner's file says of the release at `commit`, as the lines above.
fn judge(definition: &Definition, source: &str, commit: &str) -> Result<Vec<String>, String> {
    let (measured, runs) = measurement(definition, source)?;
    if measured != commit {
        return Ok(vec![format!("another\t{measured}")]);
    }
    let mut judged = Vec::new();
    for record in &definition.records {
        let held: Vec<&Run> = runs.iter().filter(|run| std::ptr::eq(run.record, record)).collect();
        judged.push(format!("device\t{}\t{}\t{}", record.id, record.platform, held.len()));
        if held.is_empty() {
            continue;
        }
        for figure in definition.figures.iter().filter(|figure| figure.platforms.contains(&record.platform)) {
            let values: Vec<f64> = held.iter().flat_map(|run| run.values.iter().filter(|(of, _)| std::ptr::eq(*of, figure)).map(|(_, value)| *value)).collect();
            judged.push(figure_line(definition, record, figure, &values)?);
        }
    }
    Ok(judged)
}

/// The one line a run printed, out of all its runner printed: a line that begins with the definition's own
/// word, the console's carriage return taken off.
fn printed_line<'a>(definition: &Definition, output: &'a str) -> Result<&'a str, String> {
    let opening = format!("{} ", definition.line);
    let lines: Vec<&str> = output.lines().map(|line| line.trim_end_matches('\r')).filter(|line| line.starts_with(&opening)).collect();
    match lines.as_slice() {
        [line] => Ok(line),
        [] => Err(format!("holds no line beginning `{}`: the run printed no figures, so there is no run to add", definition.line)),
        _ => Err(format!("holds {} lines beginning `{}`: a run prints one, so this is no output of one run", lines.len(), definition.line)),
    }
}

/// The measurement `existing` holds (none when there is no file yet), with `line`, which a run printed on the
/// device `id` at `commit`, added under it, read back whole; and the runs of `id` it then holds.
fn added(definition: &Definition, existing: Option<&str>, id: &str, commit: &str, line: &str) -> Result<(String, usize), String> {
    if !full_commit(commit) {
        return Err(format!("`{commit}` is no full commit: a measurement names the 40 hex digits of the commit the release tags"));
    }
    let mut text = match existing {
        Some(text) => {
            let (measured, _) = measurement(definition, text)?;
            if measured != commit {
                return Err(format!(
                    "measures the commit {measured}: a file is the measurement of one commit, so the runs of {commit} start a file of their own once it is moved away"
                ));
            }
            text.to_string()
        }
        None => format!("{MEASURED} {commit}\n"),
    };
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&format!("{id} {line}\n"));
    let of_device = measurement(definition, &text)?.1.iter().filter(|run| run.record.id == id).count();
    Ok((text, of_device))
}

/// `cargo xtask release-figures judge <file> <commit>`.
pub(crate) fn run_judge(root: &Path, file: &str, commit: &str) -> ExitCode {
    let judged = crate::baseline::definition(root).and_then(|definition| {
        let source = crate::scan::read_text(Path::new(file))?;
        judge(&definition, &source, commit).map_err(|reason| format!("{file}: {reason}"))
    });
    crate::report::print_lines(judged, "release-figures judge")
}

/// `cargo xtask release-figures add <file> <device id> <commit> <run output>`.
pub(crate) fn run_add(root: &Path, file: &str, id: &str, commit: &str, output: &str) -> ExitCode {
    let said = crate::baseline::definition(root).and_then(|definition| {
        let printed = crate::scan::read_text(Path::new(output))?;
        let line = printed_line(&definition, &printed).map_err(|reason| format!("{output}: {reason}"))?;
        let path = Path::new(file);
        let existing = if path.exists() { Some(crate::scan::read_text(path)?) } else { None };
        let (text, runs) = added(&definition, existing.as_deref(), id, commit, line).map_err(|reason| format!("{file}: {reason}"))?;
        if let Some(folder) = path.parent().filter(|folder| !folder.as_os_str().is_empty()) {
            std::fs::create_dir_all(folder).map_err(|error| format!("could not create {}: {error}", folder.display()))?;
        }
        std::fs::write(path, text).map_err(|error| format!("{file}: {error}"))?;
        Ok(vec![format!("{file}: {id} holds {runs} run(s) of {commit}, of the {} a comparison rests on", definition.comparison_runs)])
    });
    crate::report::print_lines(said, "release-figures add")
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    /// Two phones and the shapes of figure the rule tells apart: held on both, held on the iPhone alone, never
    /// held (`thermal_state`); three runs a comparison, two warm repeats a run. The iPhone's first path is
    /// recorded at a median of 123.5, whose allowance, 135.85, is a median of two runs.
    const BASELINE: &str = "line = \"craftbridge-baseline\"\nrepeats_word = \"preview_repeats_ms\"\nrepeats_decimals = 2\nflag_slower_than_percent = 10\n[comparison]\nruns = 3\n\
[running_example]\npreview_repeats = 2\n\
[[figure]]\nname = \"full_export_ms\"\ndecimals = 1\nflagged = true\n\
[[figure]]\nname = \"first_path_ms\"\ndecimals = 1\nflagged = true\nunheld_on = [\"android\"]\n\
[[figure]]\nname = \"thermal_state\"\ndecimals = 0\nflagged = false\nplatforms = [\"ios\"]\n\
[[device]]\nid = \"iphone\"\nplatform = \"ios\"\ndevice = \"iPhone\"\nos = \"iOS\"\nrecorded = \"2026-11-19\"\nruns = 3\n\
[device.full_export_ms]\nmedian = 2800\nmin = 2790\nmax = 2850\nvalues = [2790, 2800, 2850]\n\
[device.first_path_ms]\nmedian = 123.5\nmin = 121.3\nmax = 127.2\nvalues = [121.3, 123.5, 127.2]\n\
[device.thermal_state]\nmedian = 0\nmin = 0\nmax = 0\nvalues = [0, 0, 0]\n\
[[device]]\nid = \"xiaomi\"\nplatform = \"android\"\ndevice = \"Xiaomi\"\nos = \"Android\"\nrecorded = \"2026-11-20\"\nruns = 3\n\
[device.full_export_ms]\nmedian = 1750\nmin = 1740\nmax = 1760\nvalues = [1740, 1750, 1760]\n\
[device.first_path_ms]\nmedian = 60\nmin = 59\nmax = 131\nvalues = [59, 60, 131]\n";

    fn definition_of(baseline: &str) -> Definition {
        let scratch = crate::scratch::Scratch::new("release-figures");
        scratch.write(crate::baseline::BASELINE, baseline);
        crate::baseline::definition(&scratch.0).unwrap()
    }

    fn definition() -> Definition {
        definition_of(BASELINE)
    }

    /// The `run`th run of the iPhone, as it prints it: every run's own warm repeats tell it from every other.
    fn iphone(run: usize, export: &str, path: &str) -> String {
        format!("iphone craftbridge-baseline full_export_ms={export} first_path_ms={path} thermal_state=0 preview_repeats_ms=17.{:02},17.60\n", 10 + run)
    }

    fn xiaomi(run: usize, export: &str, path: &str) -> String {
        format!("xiaomi craftbridge-baseline full_export_ms={export} first_path_ms={path} preview_repeats_ms=20.{:02},20.10\n", 10 + run)
    }

    /// A file of `commit` holding one iPhone run for each of `iphone_exports` and one Xiaomi run for each of
    /// `xiaomi_exports`, the iPhone's first path at 123.5 and the Xiaomi's at 400.0.
    fn file(commit: &str, iphone_exports: &[&str], xiaomi_exports: &[&str]) -> String {
        let mut text = format!("commit {commit}\n");
        for (run, export) in iphone_exports.iter().enumerate() {
            text.push_str(&iphone(run, export, "123.5"));
        }
        for (run, export) in xiaomi_exports.iter().enumerate() {
            text.push_str(&xiaomi(run, export, "400.0"));
        }
        text
    }

    fn judged(text: &str) -> Result<Vec<String>, String> {
        judge(&definition(), text, COMMIT)
    }

    /// The verdict `lines` give `device`'s figure `name`.
    fn verdict<'a>(lines: &'a [String], device: &str, name: &str) -> Option<&'a str> {
        lines.iter().find(|line| line.starts_with(&format!("figure\t{device}\t{name}\t"))).and_then(|line| line.rsplit('\t').next())
    }

    #[test]
    fn a_run_like_the_record_is_within_it() {
        let lines = judged(&file(COMMIT, &["2790.0", "2800.0", "2850.0"], &["1740.0", "1750.0", "1760.0"])).unwrap();
        assert_eq!(
            lines,
            [
                "device\tiphone\tios\t3",
                "figure\tiphone\tfull_export_ms\t2800.0\t2790.0\t2800.0\t2850.0\t+0.0\twithin",
                "figure\tiphone\tfirst_path_ms\t123.5\t123.5\t123.5\t127.2\t+0.0\twithin",
                "figure\tiphone\tthermal_state\t0\t0\t0\t0\t+0.0\treported",
                "device\txiaomi\tandroid\t3",
                "figure\txiaomi\tfull_export_ms\t1750.0\t1740.0\t1750.0\t1760.0\t+0.0\twithin",
                "figure\txiaomi\tfirst_path_ms\t400.0\t400.0\t60.0\t131.0\t+566.7\tunheld",
            ],
            "every figure beside the record's; a figure on an unheld platform reports and never flags, however far it is"
        );
    }

    #[test]
    fn a_held_figure_twelve_percent_slower_is_flagged_and_an_unheld_one_is_not() {
        // 12 % over the iPhone's 2800 and the Xiaomi's 1750 medians, on every run of each.
        let lines = judged(&file(COMMIT, &["3136.0"; 3], &["1960.0"; 3])).unwrap();
        assert!(lines.contains(&"figure\tiphone\tfull_export_ms\t3136.0\t3136.0\t2800.0\t2850.0\t+12.0\tflagged".to_string()), "{lines:?}");
        assert!(lines.contains(&"figure\txiaomi\tfull_export_ms\t1960.0\t1960.0\t1750.0\t1760.0\t+12.0\tflagged".to_string()), "{lines:?}");
        assert_eq!(verdict(&lines, "xiaomi", "first_path_ms"), Some(UNHELD), "the Xiaomi's first path is 567 % over and not flagged: {lines:?}");
        assert_eq!(lines.iter().filter(|line| line.ends_with("\tflagged")).count(), 2);
    }

    #[test]
    fn a_figure_no_release_is_held_to_is_reported_however_far_it_moves() {
        let lines = judged(&file(COMMIT, &["2800.0"; 3], &["1750.0"; 3]).replace("thermal_state=0", "thermal_state=3")).unwrap();
        assert!(lines.contains(&"figure\tiphone\tthermal_state\t3\t3\t0\t0\tn/a\treported".to_string()), "{lines:?}");
    }

    #[test]
    fn a_median_over_the_allowance_is_not_flagged_while_its_fastest_run_is_inside_the_records_range() {
        // The median is 12 % over, but one run sits at 2840, inside the record's slowest, 2850: noise, not a slower build.
        let lines = judged(&file(COMMIT, &["2840.0", "3136.0", "3136.0"], &["1750.0"; 3])).unwrap();
        assert!(lines.iter().any(|line| line.starts_with("figure\tiphone\tfull_export_ms\t3136.0\t2840.0") && line.ends_with("\twithin")), "{lines:?}");
        // The fastest run exactly at the record's slowest is inside its range too, and a hair past it is not.
        let at = judged(&file(COMMIT, &["2850.0", "3136.0", "3136.0"], &["1750.0"; 3])).unwrap();
        assert_eq!(verdict(&at, "iphone", "full_export_ms"), Some(WITHIN), "{at:?}");
        let past = judged(&file(COMMIT, &["2850.1", "3136.0", "3136.0"], &["1750.0"; 3])).unwrap();
        assert_eq!(verdict(&past, "iphone", "full_export_ms"), Some(FLAGGED), "{past:?}");
    }

    #[test]
    fn a_fastest_run_past_the_records_slowest_with_a_median_inside_the_allowance_is_not_flagged() {
        // 9 % over, every run past the record's slowest: slower, but within what the file allows.
        let lines = judged(&file(COMMIT, &["3052.0"; 3], &["1750.0"; 3])).unwrap();
        assert_eq!(verdict(&lines, "iphone", "full_export_ms"), Some(WITHIN), "{lines:?}");
    }

    #[test]
    fn exactly_the_allowance_is_not_flagged_and_a_hair_either_side_is_judged_by_its_side() {
        for (export, judged_as) in [("3079.9", WITHIN), ("3080.0", WITHIN), ("3080.1", FLAGGED)] {
            let lines = judged(&file(COMMIT, &[export; 3], &["1750.0"; 3])).unwrap();
            assert_eq!(verdict(&lines, "iphone", "full_export_ms"), Some(judged_as), "{export} over the record's 2800: {lines:?}");
        }
    }

    #[test]
    fn a_median_of_two_runs_exactly_at_the_allowance_is_weighed_exactly() {
        // 10 % over the record's 123.5 is 135.85, the mean of 135.8 and 135.9, which a float's own sum puts a
        // hair over; 135.9, 10.04 % over, is past it.
        let at = ["135.8", "135.8", "135.9", "135.9"];
        let past = ["135.8", "135.9", "135.9", "135.9"];
        for (runs, judged_as, median) in [(at, WITHIN, "135.85"), (past, FLAGGED, "135.9")] {
            let mut text = format!("commit {COMMIT}\n");
            for (run, path) in runs.iter().enumerate() {
                text.push_str(&iphone(run, "2800.0", path));
            }
            let lines = judged(&text).unwrap();
            assert_eq!(verdict(&lines, "iphone", "first_path_ms"), Some(judged_as), "{runs:?}: {lines:?}");
            // Each median is said exactly, so the two never read alike beside verdicts that differ.
            let said = format!("figure\tiphone\tfirst_path_ms\t{median}\t135.8\t123.5\t127.2\t+10.0\t{judged_as}");
            assert!(lines.contains(&said), "{said:?} in {lines:?}");
        }
    }

    #[test]
    fn an_allowance_with_decimals_is_weighed_as_the_file_writes_it() {
        // 10.1 % over 2800 is 3082.8 exactly, which 10.1 as a float puts a hair under.
        let definition = definition_of(&BASELINE.replace("flag_slower_than_percent = 10\n", "flag_slower_than_percent = 10.1\n"));
        for (export, judged_as) in [("3082.8", WITHIN), ("3082.9", FLAGGED)] {
            let lines = judge(&definition, &file(COMMIT, &[export; 3], &["1750.0"; 3]), COMMIT).unwrap();
            assert_eq!(verdict(&lines, "iphone", "full_export_ms"), Some(judged_as), "{export}: {lines:?}");
        }
    }

    #[test]
    fn the_runs_a_comparison_asks_for_are_the_fewest_compared() {
        // The test's comparison asks for three runs: two are reported and not compared, three and four are.
        for (runs, judged_as) in [(2, UNCOMPARED), (3, FLAGGED), (4, FLAGGED)] {
            let lines = judged(&file(COMMIT, &vec!["3500.0"; runs], &["1750.0"; 3])).unwrap();
            assert_eq!(lines.first().map(String::as_str), Some(format!("device\tiphone\tios\t{runs}").as_str()));
            assert_eq!(verdict(&lines, "iphone", "full_export_ms"), Some(judged_as), "{runs} runs: {lines:?}");
            // A figure no release is held to, and one on an unheld platform, read the same however few runs stand.
            assert_eq!(verdict(&lines, "iphone", "thermal_state"), Some(REPORTED), "{lines:?}");
        }
        let few = judged(&file(COMMIT, &["3500.0"; 2], &["1960.0"; 2])).unwrap();
        assert_eq!(verdict(&few, "xiaomi", "first_path_ms"), Some(UNHELD), "{few:?}");
    }

    #[test]
    fn a_phone_with_no_run_is_listed_as_measured_on_none() {
        let lines = judged(&file(COMMIT, &["3500.0"; 3], &[])).unwrap();
        assert!(lines.contains(&"device\txiaomi\tandroid\t0".to_string()), "{lines:?}");
        assert!(!lines.iter().any(|line| line.starts_with("figure\txiaomi")), "{lines:?}");
        assert_eq!(judged(&format!("commit {COMMIT}\n")).unwrap(), ["device\tiphone\tios\t0", "device\txiaomi\tandroid\t0"], "a file of no run");
    }

    #[test]
    fn a_file_measured_at_another_commit_is_no_measurement_of_this_release() {
        let other = "fedcba9876543210fedcba9876543210fedcba98";
        assert_eq!(judged(&file(other, &["2800.0"; 3], &["1750.0"; 3])).unwrap(), [format!("another\t{other}")]);
    }

    #[test]
    fn a_change_too_small_to_print_is_no_change_either_way() {
        let lines = judged(&file(COMMIT, &["2799.9"; 3], &["1750.0"; 3])).unwrap();
        assert!(lines.contains(&"figure\tiphone\tfull_export_ms\t2799.9\t2799.9\t2800.0\t2850.0\t+0.0\twithin".to_string()), "{lines:?}");
    }

    #[test]
    fn a_medians_even_count_takes_the_middle_two() {
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), Some(2.5));
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&[]), None);
    }

    #[test]
    fn a_record_whose_median_no_run_could_print_is_refused_rather_than_rounded() {
        let definition = definition_of(&BASELINE.replace("median = 123.5\n", "median = 123.52\n"));
        let error = judge(&definition, &file(COMMIT, &["2800.0"; 3], &[]), COMMIT).unwrap_err();
        assert!(error.contains("iphone: `first_path_ms`'s record's median, 123.52, is no value a run prints with 1 decimals"), "{error}");
    }

    #[test]
    fn a_file_that_is_no_run_of_the_definition_is_refused() {
        let good = file(COMMIT, &["2800.0"; 3], &["1750.0"; 3]);
        assert!(judged(&good).is_ok(), "{good}");
        let headless = good.replacen(&format!("commit {COMMIT}\n"), "", 1);
        assert!(judged(&headless).unwrap_err().contains("not `commit <the full commit the release tags>`"), "a measurement says what it measured");
        for short in ["commit 0123456", &format!("commit {}", COMMIT.to_uppercase()), &format!("commit  {COMMIT}"), &format!("commits {COMMIT}")] {
            assert!(judged(&good.replacen(&format!("commit {COMMIT}"), short, 1)).unwrap_err().contains("the full commit"), "{short}");
        }
        assert!(judged("").unwrap_err().contains("the first line is ``"), "an empty file says what it lacks");
        for (from, to, refusal) in [
            ("xiaomi craftbridge", "pixel craftbridge", "names the device `pixel`, which no record of conformance/effects/baseline.toml holds"),
            ("iphone craftbridge-baseline full_export_ms=2800.0", "iphone other-line full_export_ms=2800.0", "does not begin `iphone craftbridge-baseline`"),
            ("full_export_ms=2800.0", "full_export=2800.0", "carries `full_export=2800.0` where `full_export_ms=<v>` stands next, with 1 decimals"),
            ("first_path_ms=123.5", "first_path_ms=fast", "carries `first_path_ms=fast` where `first_path_ms=<v>` stands next"),
            ("first_path_ms=123.5", "first_path_ms=NaN", "carries `first_path_ms=NaN`"),
            ("first_path_ms=123.5", "first_path_ms=inf", "carries `first_path_ms=inf`"),
            // Each spelling of a figure's value that its decimals do not print.
            ("full_export_ms=2800.0", "full_export_ms=2800", "carries `full_export_ms=2800` where"),
            ("full_export_ms=2800.0", "full_export_ms=2800.00", "carries `full_export_ms=2800.00` where"),
            ("full_export_ms=2800.0", "full_export_ms=2.8e3", "carries `full_export_ms=2.8e3` where"),
            ("full_export_ms=2800.0", "full_export_ms=+2800.0", "carries `full_export_ms=+2800.0` where"),
            ("full_export_ms=2800.0", "full_export_ms= 2800.0", "carries `full_export_ms=` where"),
            ("thermal_state=0", "thermal_state=0.0", "carries `thermal_state=0.0` where `thermal_state=<v>` stands next, with 0 decimals"),
            (" thermal_state=0", "", "carries `preview_repeats_ms=17.10,17.60` where `thermal_state=<v>` stands next"),
            // The repeats, short of the running example's, past it, or printed with other decimals.
            (
                " preview_repeats_ms=17.10,17.60",
                "",
                "carries `` where the running example's 2 warm repeats stand after its figures, `preview_repeats_ms=<v>,<v>,...` with 2 decimals",
            ),
            ("preview_repeats_ms=17.10,17.60", "preview_repeats_ms=17.10", "carries `preview_repeats_ms=17.10` where the running example's 2 warm repeats"),
            ("preview_repeats_ms=17.10,17.60", "preview_repeats_ms=17.10,17.60,17.70", "carries `preview_repeats_ms=17.10,17.60,17.70` where"),
            ("preview_repeats_ms=17.10,17.60", "preview_repeats_ms=17.10,17.6", "carries `preview_repeats_ms=17.10,17.6` where"),
            ("preview_repeats_ms=17.10,17.60", "preview_repeats_ms=17.10,x", "carries `preview_repeats_ms=17.10,x` where"),
            ("preview_repeats_ms=17.10,17.60", "warm_ms=17.10,17.60", "carries `warm_ms=17.10,17.60` where"),
            ("preview_repeats_ms=17.10,17.60", "preview_repeats_ms=17.10,17.60 extra", "carries a word after its warm repeats"),
        ] {
            assert!(good.contains(from), "{from}");
            let error = judged(&good.replacen(from, to, 1)).unwrap_err();
            assert!(error.contains(refusal), "`{from}` as `{to}` is refused as {refusal:?}: {error}");
        }
        let blank = format!("{good}\n");
        assert!(judged(&blank).unwrap_err().contains("names the device ``"), "a blank line is no run");
    }

    #[test]
    fn a_value_below_zero_is_read_as_a_run_prints_it() {
        // A condition can read below zero (a battery out in the cold), and a run prints it with its minus.
        let lines = judged(&file(COMMIT, &["2800.0"; 3], &["1750.0"; 3]).replace("thermal_state=0", "thermal_state=-1")).unwrap();
        assert!(lines.contains(&"figure\tiphone\tthermal_state\t-1\t-1\t0\t0\tn/a\treported".to_string()), "{lines:?}");
    }

    #[test]
    fn one_run_written_twice_is_refused_rather_than_counted_twice() {
        let once = file(COMMIT, &["3136.0", "3136.0"], &["1750.0"; 3]);
        let twice = format!("{once}{}", iphone(0, "3136.0", "123.5"));
        let error = judged(&twice).unwrap_err();
        assert!(error.contains("stands twice: every run prints its own warm repeats"), "two runs and one of them again are no three runs: {error}");
    }

    #[test]
    fn a_definition_with_no_repeats_word_cannot_read_a_run_whole() {
        let definition = definition_of(&BASELINE.replace("repeats_word = \"preview_repeats_ms\"\n", ""));
        let error = judge(&definition, &file(COMMIT, &["2800.0"; 3], &["1750.0"; 3]), COMMIT).unwrap_err();
        assert!(error.contains("the baseline names no text `repeats_word`, so a run's line cannot be read whole"), "{error}");
        for (from, refusal) in [
            ("repeats_decimals = 2\n", "preview_repeats_ms has no whole `repeats_decimals`"),
            ("preview_repeats = 2\n", "the running example has no whole `preview_repeats`"),
        ] {
            let definition = definition_of(&BASELINE.replace(from, ""));
            let error = judge(&definition, &file(COMMIT, &["2800.0"; 3], &["1750.0"; 3]), COMMIT).unwrap_err();
            assert!(error.contains(refusal), "{from}: {error}");
        }
    }

    #[test]
    fn the_repositorys_own_record_judges_its_own_runs_within_it() {
        let definition = crate::baseline::definition(&crate::repo_root()).unwrap();
        let repeats = definition.repeats.as_ref().unwrap();
        let mut text = format!("commit {COMMIT}\n");
        for record in &definition.records {
            for run in 0..definition.comparison_runs {
                let figures: Vec<String> = record
                    .spreads
                    .iter()
                    .map(|spread| {
                        let decimals = definition.figures.iter().find(|figure| figure.name == spread.name).unwrap().decimals;
                        format!("{}={:.decimals$}", spread.name, spread.max)
                    })
                    .collect();
                let warm: Vec<String> =
                    (0..repeats.count).map(|repeat| format!("{:.decimals$}", (run * 100) as f64 + repeat as f64, decimals = repeats.decimals)).collect();
                text.push_str(&format!("{} {} {} {}={}\n", record.id, definition.line, figures.join(" "), repeats.word, warm.join(",")));
            }
        }
        let lines = judge(&definition, &text, COMMIT).unwrap();
        assert!(!lines.iter().any(|line| line.ends_with("\tflagged")), "runs at the record's own slowest are never flagged against it: {lines:?}");
        assert!(lines.iter().any(|line| line.ends_with("\twithin")), "{lines:?}");
    }

    /// What an iPhone run's runner printed around its line: its own lines, ending as a terminal's do, and the
    /// line itself with a carriage return more, as a console's own one is left behind by a terminal's.
    fn iphone_output(run: usize, export: &str) -> String {
        let printed = iphone(run, export, "123.5");
        let line = printed.trim_end().strip_prefix("iphone ").unwrap();
        format!("test theRunningExample ... started\r\n{line}\r\r\ntest theRunningExample ... ok\r\n")
    }

    #[test]
    fn a_run_added_from_its_runners_output_is_read_back_by_the_judge() {
        let definition = definition();
        let (first, runs) = added(&definition, None, "iphone", COMMIT, printed_line(&definition, &iphone_output(0, "3136.0")).unwrap()).unwrap();
        assert_eq!(first, format!("commit {COMMIT}\n{}", iphone(0, "3136.0", "123.5")), "a new file starts at its commit");
        assert_eq!(runs, 1);
        let mut text = first;
        for run in 1..3 {
            (text, _) = added(&definition, Some(&text), "iphone", COMMIT, printed_line(&definition, &iphone_output(run, "3136.0")).unwrap()).unwrap();
        }
        let xiaomi_output = format!("PASS  baseline: the running example\n{}OK (1 checks)\n", xiaomi(0, "1750.0", "400.0").strip_prefix("xiaomi ").unwrap());
        let (text, runs) = added(&definition, Some(text.trim_end()), "xiaomi", COMMIT, printed_line(&definition, &xiaomi_output).unwrap()).unwrap();
        assert_eq!(runs, 1, "the runs of the device added, not of the file");
        let lines = judge(&definition, &text, COMMIT).unwrap();
        assert!(lines.contains(&"device\tiphone\tios\t3".to_string()) && lines.contains(&"device\txiaomi\tandroid\t1".to_string()), "{lines:?}");
        assert_eq!(verdict(&lines, "iphone", "full_export_ms"), Some(FLAGGED), "{lines:?}");
    }

    #[test]
    fn a_run_that_cannot_stand_in_the_file_is_not_added() {
        let definition = definition();
        let output = iphone_output(0, "2800.0");
        let line = printed_line(&definition, &output).unwrap();
        let (text, _) = added(&definition, None, "iphone", COMMIT, line).unwrap();
        for (error, refusal) in [
            (added(&definition, Some(&text), "iphone", COMMIT, line).unwrap_err(), "stands twice"),
            (added(&definition, None, "pixel", COMMIT, line).unwrap_err(), "names the device `pixel`"),
            (added(&definition, None, "xiaomi", COMMIT, line).unwrap_err(), "the run `xiaomi craftbridge-baseline"),
            (
                added(&definition, None, "iphone", COMMIT, &line.replace("first_path_ms=123.5", "first_path_ms=123.50")).unwrap_err(),
                "carries `first_path_ms=123.50` where `first_path_ms=<v>` stands next, with 1 decimals",
            ),
            (
                added(&definition, None, "iphone", COMMIT, &line.replace(",17.60", ",17.6")).unwrap_err(),
                "carries `preview_repeats_ms=17.10,17.6` where the running example's 2 warm repeats stand after its figures",
            ),
            (added(&definition, None, "iphone", "0123456", line).unwrap_err(), "`0123456` is no full commit"),
            (
                added(&definition, Some(&text), "iphone", "fedcba9876543210fedcba9876543210fedcba98", line).unwrap_err(),
                "measures the commit 0123456789abcdef0123456789abcdef01234567: a file is the measurement of one commit",
            ),
            (added(&definition, Some("commit 0123456\n"), "iphone", COMMIT, line).unwrap_err(), "the first line is `commit 0123456`"),
            (printed_line(&definition, "test theRunningExample ... FAILED\n").unwrap_err(), "holds no line beginning `craftbridge-baseline`"),
            (printed_line(&definition, &format!("{output}{output}")).unwrap_err(), "holds 2 lines beginning `craftbridge-baseline`"),
        ] {
            assert!(error.contains(refusal), "refused as {refusal:?}: {error}");
        }
        // A line that only mentions the word, or holds it further in, is no run's line.
        assert!(printed_line(&definition, &format!("craftbridge-baselines\n  craftbridge-baseline x\n{output}")).is_ok());
    }
}
