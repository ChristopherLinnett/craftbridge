//! The per-device baseline: `cargo xtask baseline` prints what `conformance/effects/baseline.toml` records
//! for each acceptance phone, as a release's notes read it — one `allowance` line, then one `device` line
//! per record, each field a tab away from the next, the last but one the record's figures as a run's line
//! prints them and the last the conditions a release's runs are compared with it under.
//!
//! The file is read with the `toml` crate, as every TOML of this repository is (`crate::pins::toml_file`),
//! and no script reads it by text itself: which figures there are, the decimals each is printed with, and
//! which platform measures which are the file's to say, and a text reader would know one spelling of each
//! and misread every other. The Swift real-app test and the Kotlin instrumented check measure against this
//! same file (`ios/sdk/Tests/CraftBridgeEffectsTests/Baseline.swift`,
//! `android/sdk/effects/src/androidTest/kotlin/io/github/christopherlinnett/craftbridge/effects/BaselineOnDevice.kt`);
//! this is the one reader of the records for what only reports them, which is what a release's notes do
//! (plan §9, §23, §29), and of the rule a release's own figures are judged by and the line their runs print
//! (`crate::release_figures`): the figures' `flagged`, `unheld_on` and condition, the `[comparison]`'s runs, the
//! allowance and the warm repeats are read here once, each record's conditions among them. A record a note could not fill is refused here, naming
//! the device and the figure, rather than printed as nothing.
//!
//! What the file may hold is the Swift reader's to say: the gate's Swift suite reads the repository's own
//! file through it (`BaselineTests`), refusing a table or a key it does not read, at the top, in a figure
//! and in a record, and a record that is not complete. So a key is added or renamed in the change that
//! teaches that reader, and this one does not keep a second list of the file's keys, which every such
//! change would have to find and grow too or stop the next release's notes: it requires every key whose
//! words reach the notes — so a renamed one is refused, never read as absent — and leaves every other key
//! to that reader. In a record that is every word it does not print; a table there is a figure's spread,
//! which the notes print, so one no `[[figure]]` names, or that the record's platform does not measure, is
//! refused rather than left out of the notes.

use std::path::Path;
use std::process::ExitCode;

/// The one file the baseline is written in: the figures a run measures, the line it prints, and the record
/// of each acceptance phone.
pub(crate) const BASELINE: &str = "conformance/effects/baseline.toml";

/// The platforms a record, or a figure, can name, as the file spells them.
const PLATFORMS: [&str; 2] = ["ios", "android"];

/// The most decimals a figure may print: a figure asking for more prints noise, and a runaway number there
/// is a mistake to refuse rather than a line of digits to carry into a note.
const MOST_DECIMALS: i64 = 12;

/// What a figure that is a run's condition asks of it, as the file writes it: the value the record's own runs
/// read (`record`), or a bound the file states once for every record (`at_most`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Condition {
    Record,
    AtMost(f64),
}

/// One figure: the name its record's table carries, the decimals its value is printed with, the platforms
/// that measure it, whether a release is held to it (`flagged`), the platforms whose runs of it are
/// reported and never flagged (`unheld_on`) and, for a figure that is a condition, what the condition asks.
pub(crate) struct Figure {
    pub(crate) name: String,
    pub(crate) decimals: usize,
    pub(crate) platforms: Vec<String>,
    pub(crate) flagged: bool,
    pub(crate) unheld_on: Vec<String>,
    pub(crate) condition: Option<Condition>,
}

/// A condition as one record holds it: the value its runs read, or the bound every record has.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Rule {
    Exactly(f64),
    AtMost(f64),
}

/// One condition of a record: the figure that shows it run by run, the decimals that figure prints, and what
/// a run's value of it must be for the run to be compared with the record.
pub(crate) struct Required {
    pub(crate) figure: String,
    pub(crate) decimals: usize,
    pub(crate) rule: Rule,
}

impl Required {
    /// Whether a run that read `value` of the figure was made in the condition. Both sides are weighed as
    /// whole half units of the figure's last decimal, so no rounding of a fraction decides a run at the bound.
    pub(crate) fn met_by(&self, value: f64) -> bool {
        let (Some(read), Some(asked)) = (half_units(value, self.decimals), half_units(self.bound(), self.decimals)) else {
            return false;
        };
        match self.rule {
            Rule::Exactly(_) => read == asked,
            Rule::AtMost(_) => read <= asked,
        }
    }

    fn bound(&self) -> f64 {
        match self.rule {
            Rule::Exactly(value) | Rule::AtMost(value) => value,
        }
    }

    /// The rule in the words a note prints after the figure's name: `exactly 2880`, `at most 33.0`.
    pub(crate) fn asks(&self) -> String {
        let value = format!("{:.decimals$}", self.bound(), decimals = self.decimals);
        match self.rule {
            Rule::Exactly(_) => format!("exactly {value}"),
            Rule::AtMost(_) => format!("at most {value}"),
        }
    }

    /// The condition with its figure: `cpu_cap_mhz exactly 2880`, `battery_c at most 33.0`.
    pub(crate) fn says(&self) -> String {
        format!("{} {}", self.figure, self.asks())
    }
}

/// A value in half units of its figure's last decimal, or nothing for one that is not a whole number of
/// them. A run prints each value with its figure's decimals and a median is one run's value or the mean of
/// two, the record's as a release's, so every value the rule weighs is a whole number of half units: the
/// rule is weighed in those whole numbers, so no rounding of a fraction decides a figure at the allowance's
/// very edge. Past 2^53 a float holds no whole number exactly, so a value that large is none either.
pub(crate) fn half_units(value: f64, decimals: usize) -> Option<i128> {
    let scaled = value * 2.0 * 10f64.powi(i32::try_from(decimals).ok()?);
    let whole = scaled.round();
    ((scaled - whole).abs() < 1e-6 && whole.abs() < 9_007_199_254_740_992.0).then_some(whole as i128)
}

/// One record's spread of a figure: the median, the fastest and the slowest run, which a release is held to,
/// and the median as a run prints it.
pub(crate) struct Spread {
    pub(crate) name: String,
    pub(crate) median: f64,
    pub(crate) min: f64,
    pub(crate) max: f64,
    pub(crate) word: String,
}

/// One acceptance phone's record, as far as a release reads it.
pub(crate) struct Record {
    pub(crate) id: String,
    pub(crate) platform: String,
    pub(crate) device: String,
    pub(crate) os: String,
    pub(crate) recorded: String,
    pub(crate) runs: i64,
    pub(crate) spreads: Vec<Spread>,
    /// What a release's run must read of each condition to be compared with this record.
    pub(crate) conditions: Vec<Required>,
}

/// The warm preview repeats a run's line ends with: the word they stand under, the decimals each is printed
/// with (`repeats_decimals`) and how many a run makes (`[running_example].preview_repeats`).
pub(crate) struct Repeats {
    pub(crate) word: String,
    pub(crate) decimals: usize,
    pub(crate) count: usize,
}

/// What `baseline.toml` defines, read once: the line a run prints and the repeats it ends with, the
/// allowance in percent and the runs a release is compared on (`[comparison]`), the figures in the order a
/// run prints them, and the record of each phone. The repeats are held as read or as why they cannot be:
/// only a release's own runs carry them, so only their reader refuses a file for them, and the records'
/// notes, which print none, leave them to the Swift reader as the module's comment says.
pub(crate) struct Definition {
    pub(crate) line: String,
    pub(crate) repeats: Result<Repeats, String>,
    pub(crate) allowance: f64,
    pub(crate) comparison_runs: i64,
    pub(crate) figures: Vec<Figure>,
    pub(crate) records: Vec<Record>,
}

/// The figures the file defines, in the order it defines them: the order a run's line prints them in.
fn figures(document: &toml::Table) -> Result<Vec<Figure>, String> {
    let listed = document
        .get("figure")
        .and_then(toml::Value::as_array)
        .filter(|listed| !listed.is_empty())
        .ok_or_else(|| format!("{BASELINE}: defines no [[figure]], so no run and no record has a figure to name"))?;
    let mut figures: Vec<Figure> = Vec::with_capacity(listed.len());
    for (position, entry) in listed.iter().enumerate() {
        let table = entry.as_table().ok_or_else(|| format!("{BASELINE}: figure {} is no table", position + 1))?;
        let name = text(table, "name", &format!("figure {}", position + 1))?;
        if figures.iter().any(|figure| figure.name == name) {
            return Err(format!("{BASELINE}: two figures are named `{name}`, so a record would say one and mean the other"));
        }
        let decimals = decimals(table, "decimals", &name)?;
        // A figure naming no platform is measured on every platform, which is how the file says "both"; a
        // platform spelt otherwise would leave the figure out of every record read for it.
        let measured_on = match table.get("platforms") {
            Some(listed) => platforms(listed, &format!("`{name}`'s `platforms`"))?,
            None => PLATFORMS.iter().map(|platform| (*platform).to_string()).collect(),
        };
        let flagged = table
            .get("flagged")
            .and_then(toml::Value::as_bool)
            .ok_or_else(|| format!("{BASELINE}: `{name}` says no `flagged`, whether a release is held to it"))?;
        let unheld_on = match table.get("unheld_on") {
            Some(listed) => platforms(listed, &format!("`{name}`'s `unheld_on`"))?,
            None => Vec::new(),
        };
        let condition = condition_of(table, &name, flagged, decimals)?;
        figures.push(Figure { name, decimals, platforms: measured_on, flagged, unheld_on, condition });
    }
    Ok(figures)
}

/// What a figure says of being a run's condition: `condition = "record"` asks a run to read what the record's
/// runs read, `condition = "at_most"` with a `condition_bound` asks it to read no more than the bound, and a
/// figure with neither is a cost or a plain report. A condition is never a cost, which `flagged` says of
/// the figures a release is held to.
fn condition_of(table: &toml::Table, name: &str, flagged: bool, decimals: usize) -> Result<Option<Condition>, String> {
    let bound = table.get("condition_bound");
    let Some(kind) = table.get("condition") else {
        return match bound {
            Some(_) => Err(format!("{BASELINE}: the figure `{name}` holds a `condition_bound` and no `condition`")),
            None => Ok(None),
        };
    };
    if flagged {
        return Err(format!("{BASELINE}: the figure `{name}` is a condition and flagged, which only a cost is"));
    }
    match (kind.as_str(), bound) {
        (Some("record"), None) => Ok(Some(Condition::Record)),
        (Some("record"), Some(_)) => Err(format!("{BASELINE}: the figure `{name}` is `record` and holds a `condition_bound`, which it has no use for")),
        (Some("at_most"), _) => {
            let bound = number(table, "condition_bound", name)
                .map_err(|_| format!("{BASELINE}: the figure `{name}` is `at_most` and names no number `condition_bound`"))?;
            // The bound is weighed against one run's value, a whole number of the figure's last decimal: a half of
            // one is a median's, which a note would print rounded to a bound the rule does not apply.
            if half_units(bound, decimals).is_none_or(|units| units % 2 != 0) {
                return Err(format!("{BASELINE}: the figure `{name}`'s `condition_bound` {bound} has more decimals than its {decimals}, which no run prints"));
            }
            Ok(Some(Condition::AtMost(bound)))
        }
        (other, _) => Err(format!(
            "{BASELINE}: the figure `{name}`'s `condition` is {}, which is neither record nor at_most",
            other.map_or_else(|| "not text".to_string(), |word| format!("`{word}`"))
        )),
    }
}

/// What a record's runs of a condition figure say of the condition: for `record` the one value they all read,
/// for `at_most` the bound, which every one of them must meet.
fn required_of(figure: &Figure, condition: Condition, spread: &Spread, id: &str) -> Result<Required, String> {
    let rule = match condition {
        Condition::Record => {
            if half_units(spread.min, figure.decimals) != half_units(spread.max, figure.decimals) {
                return Err(format!(
                    "{BASELINE}: {id}: `{}` ran from {} to {}, and a record's runs read one value of it, which a release is compared on",
                    figure.name, spread.min, spread.max
                ));
            }
            Rule::Exactly(spread.median)
        }
        Condition::AtMost(bound) => Rule::AtMost(bound),
    };
    let required = Required { figure: figure.name.clone(), decimals: figure.decimals, rule };
    if !required.met_by(spread.min) || !required.met_by(spread.max) {
        return Err(format!("{BASELINE}: {id}: `{}` ran from {} to {}, outside its own condition ({})", figure.name, spread.min, spread.max, required.asks()));
    }
    Ok(required)
}

/// The decimals the values a line names `named` are printed with, a figure's or the warm repeats', held to
/// what a line can carry.
fn decimals(table: &toml::Table, key: &str, named: &str) -> Result<usize, String> {
    let decimals = whole(table, key, named)?;
    if !(0..=MOST_DECIMALS).contains(&decimals) {
        return Err(format!("{BASELINE}: `{named}` prints {decimals} decimals, outside the 0 to {MOST_DECIMALS} a figure is printed with"));
    }
    Ok(decimals as usize)
}

/// One record's spread of a figure: its median and its slowest run, which a release's own runs are held
/// to, and its median printed with the figure's own decimals, which is the word a run prints for it and the
/// word a note quotes. The record's min and every run's value are read too, because a table holding only a
/// median is a figure someone typed rather than measured.
fn spread_of(record: &toml::Table, figure: &Figure, runs: i64, id: &str) -> Result<Spread, String> {
    let spread = record.get(&figure.name).and_then(toml::Value::as_table).ok_or_else(|| {
        format!(
            "{BASELINE}: {id} records no `{}`, the figure the file says its platform measures: the notes can quote no figure that is not there",
            figure.name
        )
    })?;
    let median = number(spread, "median", &format!("{id}: `{}`", figure.name))?;
    let low = number(spread, "min", &format!("{id}: `{}`", figure.name))?;
    let high = number(spread, "max", &format!("{id}: `{}`", figure.name))?;
    let values = spread
        .get("values")
        .and_then(toml::Value::as_array)
        .filter(|values| !values.is_empty())
        .ok_or_else(|| format!("{BASELINE}: {id}: `{}` holds no values, so its median is no measurement", figure.name))?;
    let held: Vec<f64> = values
        .iter()
        .map(|value| as_number(value).ok_or_else(|| format!("{BASELINE}: {id}: `{}` holds a value that is no number: {value}", figure.name)))
        .collect::<Result<_, _>>()?;
    if held.len() != runs as usize {
        return Err(format!("{BASELINE}: {id}: `{}` holds {} values for {runs} runs", figure.name, held.len()));
    }
    if !(low <= median && median <= high) {
        return Err(format!("{BASELINE}: {id}: `{}` says a median of {median} outside its own range {low} to {high}", figure.name));
    }
    Ok(Spread { name: figure.name.clone(), median, min: low, max: high, word: format!("{}={:.decimals$}", figure.name, median, decimals = figure.decimals) })
}

/// The warm repeats a run's line ends with, as the file defines them.
fn repeats(document: &toml::Table) -> Result<Repeats, String> {
    let word = text(document, "repeats_word", "the baseline")?;
    let decimals = decimals(document, "repeats_decimals", &word)?;
    let count = whole(table(document, "running_example")?, "preview_repeats", "the running example")?;
    let count = usize::try_from(count).map_err(|_| format!("{BASELINE}: the running example makes {count} warm previews, which no run makes"))?;
    Ok(Repeats { word, decimals, count })
}

/// The whole file, read and held to what a release's notes and its own figures read of it.
pub(crate) fn definition(root: &Path) -> Result<Definition, String> {
    let file = crate::pins::toml_file(root, BASELINE)?;
    let document = file.as_table().ok_or_else(|| format!("{BASELINE}: is no TOML table"))?;
    let line = text(document, "line", "the baseline")?;
    let repeats = repeats(document);
    let allowance = number(document, "flag_slower_than_percent", "the baseline")?;
    let comparison = table(document, "comparison")?;
    let comparison_runs = whole(comparison, "runs", "the comparison a release is held to")?;
    if comparison_runs < 1 {
        return Err(format!("{BASELINE}: a release is compared on {comparison_runs} runs, which compares nothing"));
    }
    let figures = figures(document)?;
    let records = document.get("device").and_then(toml::Value::as_array).into_iter().flatten();
    let records: Vec<&toml::Value> = records.collect();
    if records.is_empty() {
        return Err(format!("{BASELINE}: records no device, so a release's notes would carry no baseline"));
    }
    let mut read: Vec<Record> = Vec::with_capacity(records.len());
    for (position, entry) in records.iter().enumerate() {
        let named = format!("device {}", position + 1);
        let record = entry.as_table().ok_or_else(|| format!("{BASELINE}: {named} is no table"))?;
        // The id leads the record's line, so it is held to what every field of the line is held to: a
        // record with none would be quoted under a name the file never gave it, and one spanning a field
        // would move every field after it.
        let id = text(record, "id", &named)?;
        if read.iter().any(|earlier| earlier.id == id) {
            return Err(format!("{BASELINE}: {id}: a second record carries this id, so a note could name either"));
        }
        let platform = text(record, "platform", &id)?;
        if !PLATFORMS.contains(&platform.as_str()) {
            return Err(format!("{BASELINE}: {id}: the platform `{platform}` is neither ios nor android, so no figure of it can be read"));
        }
        let runs = whole(record, "runs", &id)?;
        if runs < comparison_runs {
            return Err(format!("{BASELINE}: {id}: {runs} runs, fewer than the {comparison_runs} a baseline rests on"));
        }
        let device = text(record, "device", &id)?;
        let os = text(record, "os", &id)?;
        let recorded = text(record, "recorded", &id)?;
        // A table in a record is a figure's spread; every other word of it the notes do not print is the Swift
        // reader's to hold (the module's comment says why).
        for key in record.iter().filter(|(_, value)| value.is_table()).map(|(key, _)| key) {
            match figures.iter().find(|figure| figure.name == *key) {
                // A figure the record's own platform does not measure was recorded beside the wrong phone.
                Some(figure) if !figure.platforms.contains(&platform) => {
                    return Err(format!("{BASELINE}: {id}: records `{}`, which no run on {platform} measures", figure.name));
                }
                None => {
                    return Err(format!(
                        "{BASELINE}: {id}: records `{key}`, which no [[figure]] of {BASELINE} defines: the notes print no figure they cannot name"
                    ));
                }
                _ => {}
            }
        }
        let mut spreads = Vec::new();
        for figure in figures.iter().filter(|figure| figure.platforms.contains(&platform)) {
            spreads.push(spread_of(record, figure, runs, &id)?);
        }
        // A record that names no condition would compare a release on every run, the throttled ones too.
        let mut conditions = Vec::new();
        for figure in figures.iter().filter(|figure| figure.platforms.contains(&platform)) {
            let (Some(condition), Some(spread)) = (figure.condition, spreads.iter().find(|spread| spread.name == figure.name)) else { continue };
            conditions.push(required_of(figure, condition, spread, &id)?);
        }
        if conditions.is_empty() {
            return Err(format!(
                "{BASELINE}: {id}: no figure of {platform} is a condition, so a release's runs made in other conditions would be compared with it"
            ));
        }
        read.push(Record { id, platform, device, os, recorded, runs, spreads, conditions });
    }
    Ok(Definition { line, repeats, allowance, comparison_runs, figures, records: read })
}

/// What a release's notes read of the baseline: the allowance a release is held to, then one line per
/// device record — its id, its platform, what it is, the system it ran, the day it was recorded, the runs
/// it rests on, its figures as a run on that phone prints them, and the conditions a release's runs are
/// compared with it under.
fn baseline(root: &Path) -> Result<Vec<String>, String> {
    let definition = definition(root)?;
    let mut lines = vec![format!("allowance\t{}\t{}", amount(definition.allowance), definition.comparison_runs)];
    for record in &definition.records {
        let words: Vec<&str> = std::iter::once(definition.line.as_str()).chain(record.spreads.iter().map(|spread| spread.word.as_str())).collect();
        lines.push(format!(
            "device\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            record.id,
            record.platform,
            record.device,
            record.os,
            record.recorded,
            record.runs,
            words.join(" "),
            record.conditions.iter().map(Required::says).collect::<Vec<_>>().join(", ")
        ));
    }
    Ok(lines)
}

/// The platforms a figure's list names, each one the file knows a record for.
fn platforms(listed: &toml::Value, what: &str) -> Result<Vec<String>, String> {
    let items = listed.as_array().filter(|items| !items.is_empty()).ok_or_else(|| format!("{BASELINE}: {what} is no list of platforms"))?;
    items
        .iter()
        .map(|item| {
            let platform = item.as_str().unwrap_or_default();
            if PLATFORMS.contains(&platform) {
                Ok(platform.to_string())
            } else {
                Err(format!("{BASELINE}: {what} names `{platform}`, which is neither ios nor android"))
            }
        })
        .collect()
}

/// One text field of the file or a record, named for whoever asked: a record's line is read by a script at
/// the tabs, so a field that is empty, or spans a line, is refused rather than printed as nothing.
fn text(table: &toml::Table, key: &str, what: &str) -> Result<String, String> {
    match table.get(key).and_then(toml::Value::as_str) {
        Some(text) if text.chars().any(char::is_alphanumeric) && !text.contains(['\t', '\n', '\r']) => Ok(text.to_string()),
        Some(_) => Err(format!("{BASELINE}: {what} has a `{key}` that is empty or spans a line, which no record line carries")),
        None => Err(format!("{BASELINE}: {what} names no text `{key}`")),
    }
}

/// One table of the file, by name.
fn table<'a>(document: &'a toml::Table, key: &str) -> Result<&'a toml::Table, String> {
    document.get(key).and_then(toml::Value::as_table).ok_or_else(|| format!("{BASELINE}: holds no table `{key}`"))
}

/// One whole number of the file or a record: TOML writes a count as an integer, and nothing here rounds a
/// figure to one.
fn whole(table: &toml::Table, key: &str, what: &str) -> Result<i64, String> {
    table.get(key).and_then(toml::Value::as_integer).ok_or_else(|| format!("{BASELINE}: {what} has no whole `{key}`"))
}

/// One measured number: TOML writes a whole median (`peak_memory_bytes`) as an integer and a spreading one
/// as a float, and both are the figure.
fn number(table: &toml::Table, key: &str, what: &str) -> Result<f64, String> {
    table.get(key).and_then(as_number).ok_or_else(|| format!("{BASELINE}: {what} has no number `{key}`"))
}

/// A measured number wherever the file writes one, a key's or one of a spread's values: the one read of an
/// integer or a float, so no reader of a figure takes a number the others refuse.
fn as_number(value: &toml::Value) -> Option<f64> {
    value.as_float().or_else(|| value.as_integer().map(|whole| whole as f64))
}

/// A number as the file wrote it: 10 stays 10 and 12.5 stays 12.5, so a note says the allowance the file
/// gives and no reading of the reader's own.
pub(crate) fn amount(value: f64) -> String {
    if value.fract() == 0.0 { format!("{}", value as i64) } else { format!("{value}") }
}

/// `cargo xtask baseline`: the record of each acceptance phone, as a release's notes read it.
pub(crate) fn run(root: &Path) -> ExitCode {
    crate::report::print_lines(baseline(root), "baseline")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two figures, one of them measured on iOS alone, and a record of each platform, three runs each.
    const RECORD: &str = "line = \"craftbridge-baseline\"\nflag_slower_than_percent = 10\n[comparison]\nruns = 3\n[[figure]]\nname = \"first_preview_ms\"\ndecimals = 1\nflagged = true\n[[figure]]\nname = \"thermal_state\"\ndecimals = 0\nflagged = false\nplatforms = [\"ios\"]\ncondition = \"at_most\"\ncondition_bound = 1\n[[figure]]\nname = \"cpu_cap_mhz\"\ndecimals = 0\nflagged = false\nplatforms = [\"android\"]\ncondition = \"record\"\n[[device]]\nid = \"iphone-12-pro\"\nplatform = \"ios\"\ndevice = \"iPhone 12 Pro (iPhone13,3)\"\nos = \"iOS 26.6.1\"\nrecorded = \"2026-11-19\"\nruns = 3\n[device.first_preview_ms]\nmedian = 3894.05\nmin = 3792.7\nmax = 3967.8\nvalues = [3967.8, 3894.6, 3792.7]\n[device.thermal_state]\nmedian = 0\nmin = 0\nmax = 0\nvalues = [0, 0, 0]\n[[device]]\nid = \"xiaomi-17-pro-max\"\nplatform = \"android\"\ndevice = \"Xiaomi 17 Pro Max (2509FPN0BC)\"\nos = \"Android 17 (API 37)\"\nrecorded = \"2026-11-20\"\nruns = 3\n[device.first_preview_ms]\nmedian = 1200\nmin = 1100\nmax = 1300\nvalues = [1300, 1200, 1100]\n[device.cpu_cap_mhz]\nmedian = 2880\nmin = 2880\nmax = 2880\nvalues = [2880, 2880, 2880]\n";

    fn scratch(body: &str, tag: &str) -> crate::scratch::Scratch {
        let scratch = crate::scratch::Scratch::new(tag);
        scratch.write(BASELINE, body);
        scratch
    }

    /// What a scratch tree's baseline prints, or why it refused.
    fn read(body: &str, tag: &str) -> Result<Vec<String>, String> {
        baseline(&scratch(body, tag).0)
    }

    /// The lines `RECORD` reads as.
    const READ: [&str; 3] = [
        "allowance\t10\t3",
        "device\tiphone-12-pro\tios\tiPhone 12 Pro (iPhone13,3)\tiOS 26.6.1\t2026-11-19\t3\tcraftbridge-baseline first_preview_ms=3894.1 thermal_state=0\tthermal_state at most 1",
        "device\txiaomi-17-pro-max\tandroid\tXiaomi 17 Pro Max (2509FPN0BC)\tAndroid 17 (API 37)\t2026-11-20\t3\tcraftbridge-baseline first_preview_ms=1200.0 cpu_cap_mhz=2880\tcpu_cap_mhz exactly 2880",
    ];

    #[test]
    fn the_record_reads_as_a_runs_line_of_its_figures() {
        assert_eq!(
            read(RECORD, "baseline-record").unwrap(),
            [
                "allowance\t10\t3",
                "device\tiphone-12-pro\tios\tiPhone 12 Pro (iPhone13,3)\tiOS 26.6.1\t2026-11-19\t3\tcraftbridge-baseline first_preview_ms=3894.1 thermal_state=0\tthermal_state at most 1",
                "device\txiaomi-17-pro-max\tandroid\tXiaomi 17 Pro Max (2509FPN0BC)\tAndroid 17 (API 37)\t2026-11-20\t3\tcraftbridge-baseline first_preview_ms=1200.0 cpu_cap_mhz=2880\tcpu_cap_mhz exactly 2880",
            ],
            "one line per record, each figure at its own decimals — the Xiaomi's whole median takes the decimal its figure asks for — and no figure a platform does not measure"
        );
        assert_eq!(read(RECORD, "baseline-record-again").unwrap(), READ, "the lines `READ` names are the ones the record reads as");
    }

    #[test]
    fn the_repositorys_own_record_is_read_whole() {
        let lines = baseline(&crate::repo_root()).unwrap_or_else(|reason| panic!("the record a release's notes quote cannot be read: {reason}"));
        let (allowance, devices) = lines.split_first().unwrap();
        assert!(allowance.starts_with("allowance\t"), "{allowance}");
        assert!(!devices.is_empty(), "the repository's record holds no phone");
        for device in devices {
            assert!(device.starts_with("device\t") && device.split('\t').count() == 9, "a device line of nine fields: {device}");
        }
    }

    #[test]
    fn the_repositorys_own_record_names_a_condition_for_each_phone() {
        let definition = definition(&crate::repo_root()).unwrap();
        let said =
            |id: &str| definition.records.iter().find(|record| record.id == id).map(|record| record.conditions.iter().map(Required::says).collect::<Vec<_>>());
        assert_eq!(said("iphone-12-pro"), Some(vec!["thermal_state at most 1".to_string()]), "no serious thermal state");
        assert_eq!(
            said("xiaomi-17-pro-max"),
            Some(vec!["cpu_cap_mhz exactly 2880".to_string(), "battery_c at most 33.0".to_string()]),
            "the cap the record's runs read, and a battery that is not hot"
        );
    }

    /// The keys that make a figure a condition, spelt as the file spells them, and the refusal each way of
    /// writing them wrongly is given.
    #[test]
    fn a_condition_spelt_or_bounded_wrongly_is_refused() {
        let iphone = "condition = \"at_most\"\ncondition_bound = 1\n";
        let android = "condition = \"record\"\n";
        for (from, to, refusal) in [
            (iphone, "condition = \"at_most\"\n", "the figure `thermal_state` is `at_most` and names no number `condition_bound`"),
            (iphone, "condition = \"at_most\"\ncondition_bound = \"1\"\n", "the figure `thermal_state` is `at_most` and names no number `condition_bound`"),
            (iphone, "condition_bound = 1\n", "the figure `thermal_state` holds a `condition_bound` and no `condition`"),
            (
                iphone,
                "condition = \"atmost\"\ncondition_bound = 1\n",
                "the figure `thermal_state`'s `condition` is `atmost`, which is neither record nor at_most",
            ),
            (iphone, "condition = 1\ncondition_bound = 1\n", "the figure `thermal_state`'s `condition` is not text, which is neither record nor at_most"),
            (iphone, "condition = \"at_most\"\ncondition_bound = 1.25\n", "the figure `thermal_state`'s `condition_bound` 1.25 has more decimals than its 0"),
            // A half of the last decimal is a median's, never a value one run prints, so it is no bound either.
            (iphone, "condition = \"at_most\"\ncondition_bound = 1.5\n", "the figure `thermal_state`'s `condition_bound` 1.5 has more decimals than its 0"),
            (
                android,
                "condition = \"record\"\ncondition_bound = 2880\n",
                "the figure `cpu_cap_mhz` is `record` and holds a `condition_bound`, which it has no use for",
            ),
            (android, "condition = \"Record\"\n", "the figure `cpu_cap_mhz`'s `condition` is `Record`, which is neither record nor at_most"),
        ] {
            assert_eq!(RECORD.matches(from).count(), 1, "{from}");
            let error = read(&RECORD.replace(from, to), "baseline-condition-spelt").unwrap_err();
            assert!(error.contains(refusal), "`{to}` is refused as {refusal:?}: {error}");
        }
        let flagged = RECORD.replace("flagged = false\nplatforms = [\"ios\"]", "flagged = true\nplatforms = [\"ios\"]");
        assert!(
            read(&flagged, "baseline-condition-flagged").unwrap_err().contains("the figure `thermal_state` is a condition and flagged, which only a cost is"),
            "a condition is never a cost a release is flagged on"
        );
        assert!(
            read(&RECORD.replace("condition_bound = 1\n", "condition_bound = 1.0\n"), "baseline-condition-float").is_ok(),
            "a bound written as a float is the same bound"
        );
    }

    /// A bound is a whole number of the figure's last decimal, written as an integer or as a float.
    #[test]
    fn a_bound_is_read_as_a_whole_number_of_the_figures_decimals() {
        let battery = RECORD.replace("name = \"cpu_cap_mhz\"\ndecimals = 0", "name = \"battery_c\"\ndecimals = 1");
        let battery = battery
            .replace("condition = \"record\"\n", "condition = \"at_most\"\ncondition_bound = 33\n")
            .replace("[device.cpu_cap_mhz]", "[device.battery_c]");
        let battery = battery.replace(
            "median = 2880\nmin = 2880\nmax = 2880\nvalues = [2880, 2880, 2880]",
            "median = 31.2\nmin = 30.9\nmax = 31.2\nvalues = [30.9, 31.2, 31.2]",
        );
        for (bound, said) in [("33", "33.0"), ("33.0", "33.0"), ("34.5", "34.5")] {
            let lines = read(&battery.replace("condition_bound = 33\n", &format!("condition_bound = {bound}\n")), "baseline-bound-read").unwrap();
            assert!(lines.iter().any(|line| line.ends_with(&format!("battery_c at most {said}"))), "{bound}: {lines:?}");
        }
        for bound in ["33.05", "33.25", "-0.05"] {
            let error = read(&battery.replace("condition_bound = 33\n", &format!("condition_bound = {bound}\n")), "baseline-bound-refused").unwrap_err();
            assert!(error.contains("`condition_bound`") && error.contains("has more decimals than its 1"), "{bound}: {error}");
        }
        let thermal = RECORD.replace("condition_bound = 1\n", "condition_bound = -1\n");
        assert!(read(&thermal, "baseline-bound-negative").unwrap_err().contains("outside its own condition"), "a negative bound the record's own runs break");
    }

    #[test]
    fn a_record_that_names_no_condition_or_breaks_its_own_is_refused() {
        // A platform none of whose figures is a condition would compare a release's throttled runs too.
        let none = RECORD.replace("condition = \"record\"\n", "");
        assert!(
            read(&none, "baseline-no-condition")
                .unwrap_err()
                .contains("xiaomi-17-pro-max: no figure of android is a condition, so a release's runs made in other conditions"),
            "a record with no condition is refused"
        );
        let ios_none = RECORD.replace("condition = \"at_most\"\ncondition_bound = 1\n", "");
        assert!(read(&ios_none, "baseline-no-condition-ios").unwrap_err().contains("iphone-12-pro: no figure of ios is a condition"));
        // A record's runs that read different values of a `record` condition leave nothing to compare a release with.
        let spread = RECORD.replace(
            "median = 2880\nmin = 2880\nmax = 2880\nvalues = [2880, 2880, 2880]",
            "median = 2880\nmin = 2438\nmax = 2880\nvalues = [2880, 2438, 2880]",
        );
        assert!(
            read(&spread, "baseline-condition-spread")
                .unwrap_err()
                .contains("xiaomi-17-pro-max: `cpu_cap_mhz` ran from 2438 to 2880, and a record's runs read one value of it"),
            "a record made across two caps names no cap"
        );
        // A record whose own runs break an `at_most` is a record made outside the condition it is compared under.
        let hot = RECORD.replace("median = 0\nmin = 0\nmax = 0\nvalues = [0, 0, 0]", "median = 0\nmin = 0\nmax = 2\nvalues = [0, 0, 2]");
        assert!(
            read(&hot, "baseline-condition-record-hot")
                .unwrap_err()
                .contains("iphone-12-pro: `thermal_state` ran from 0 to 2, outside its own condition (at most 1)"),
            "a record made at a serious thermal state breaks the rule it holds a release to"
        );
        // The record that meets its conditions at the bound is the record the rule is for.
        assert!(read(&RECORD.replace("max = 0\nvalues = [0, 0, 0]", "max = 1\nvalues = [0, 0, 1]"), "baseline-condition-at-bound").is_ok());
    }

    #[test]
    fn a_key_the_notes_do_not_read_is_left_to_the_files_own_reader() {
        let beside = RECORD
            .replace("line = \"craftbridge-baseline\"\n", "line = \"craftbridge-baseline\"\nrepeats_word = \"preview_repeats_ms\"\nrepeats_decimals = 2\n")
            .replace(
                "decimals = 1\nflagged = true\n",
                "decimals = 1\nflagged = true\nunheld_on = [\"android\"]\nwhat = \"milliseconds to the first preview\"\n",
            )
            .replace("recorded = \"2026-11-20\"\n", "recorded = \"2026-11-20\"\nfirmware = \"HyperOS 3\"\n");
        assert_eq!(
            read(&beside, "baseline-keys-beside").unwrap(),
            READ,
            "a top-level key, a figure's key and a record's word the notes print nothing of read as the record without them"
        );
    }

    #[test]
    fn a_key_the_notes_read_is_refused_renamed() {
        for (from, to, refusal) in [
            ("line = ", "line_word = ", "the baseline names no text `line`"),
            ("flag_slower_than_percent = ", "flag_slower_percent = ", "the baseline has no number `flag_slower_than_percent`"),
            ("[comparison]\nruns = 3", "[comparison]\nrun_count = 3", "the comparison a release is held to has no whole `runs`"),
            ("decimals = 1\n", "places = 1\n", "first_preview_ms has no whole `decimals`"),
            ("platforms = [\"ios\"]", "platform = [\"ios\"]", "xiaomi-17-pro-max records no `thermal_state`"),
            ("os = \"iOS 26.6.1\"", "system = \"iOS 26.6.1\"", "iphone-12-pro names no text `os`"),
            ("recorded = \"2026-11-20\"", "recorded_on = \"2026-11-20\"", "xiaomi-17-pro-max names no text `recorded`"),
        ] {
            assert_eq!(RECORD.matches(from).count(), 1, "{from}");
            let error = read(&RECORD.replace(from, to), "baseline-renamed").unwrap_err();
            assert!(error.contains(refusal), "`{from}` renamed `{to}` is refused as {refusal:?}: {error}");
        }
    }

    #[test]
    fn a_record_named_by_no_id_or_an_id_across_its_fields_is_refused() {
        let no_id = RECORD.replace("id = \"xiaomi-17-pro-max\"\n", "");
        assert!(read(&no_id, "baseline-no-id").unwrap_err().contains("device 2 names no text `id`"), "a record with no id is quoted under none");
        for id in ["\"xiaomi\\t17\"", "\"\""] {
            let error = read(&RECORD.replace("\"xiaomi-17-pro-max\"", id), "baseline-id-spanning").unwrap_err();
            assert!(error.contains("device 2 has a `id` that is empty or spans a line"), "the id {id}: {error}");
        }
    }

    #[test]
    fn a_figure_that_says_whether_a_release_is_held_to_it_and_where_it_is_not_is_read_whole() {
        let no_flag = RECORD.replace("decimals = 1\nflagged = true\n", "decimals = 1\n");
        assert!(
            read(&no_flag, "baseline-no-flagged").unwrap_err().contains("`first_preview_ms` says no `flagged`, whether a release is held to it"),
            "a figure a release may or may not be held to is refused until the file says which"
        );
        let held = RECORD.replace("decimals = 1\nflagged = true\n", "decimals = 1\nflagged = true\nunheld_on = [\"android\"]\n");
        let definition = {
            let scratch = scratch(&held, "baseline-unheld");
            definition(&scratch.0).unwrap()
        };
        let first = definition.figures.iter().find(|figure| figure.name == "first_preview_ms").unwrap();
        assert!(first.flagged && first.unheld_on == ["android"], "the figure's own words reach the rule that reads them");
        let misspelt = RECORD.replace("decimals = 1\nflagged = true\n", "decimals = 1\nflagged = true\nunheld_on = [\"andriod\"]\n");
        assert!(
            read(&misspelt, "baseline-unheld-misspelt")
                .unwrap_err()
                .contains("`first_preview_ms`'s `unheld_on` names `andriod`, which is neither ios nor android"),
            "a platform spelt otherwise would hold a figure it should not"
        );
    }

    #[test]
    fn a_figure_printed_past_the_decimals_a_figure_takes_is_refused() {
        for decimals in ["13", "-1"] {
            let error = read(&RECORD.replace("decimals = 1\n", &format!("decimals = {decimals}\n")), "baseline-decimals").unwrap_err();
            assert!(error.contains(&format!("`first_preview_ms` prints {decimals} decimals, outside the 0 to 12")), "{error}");
        }
        assert!(read(&RECORD.replace("decimals = 1\n", "decimals = 12\n"), "baseline-decimals-most").is_ok(), "twelve is the most a figure prints");
    }

    #[test]
    fn two_figures_of_one_name_are_refused() {
        let twice = RECORD.replace("name = \"thermal_state\"", "name = \"first_preview_ms\"");
        assert!(
            read(&twice, "baseline-figure-twice").unwrap_err().contains("two figures are named `first_preview_ms`"),
            "a record would say one and mean the other"
        );
    }

    #[test]
    fn a_figures_platforms_are_a_list_of_ios_or_android() {
        for (platforms, refusal) in [
            ("[\"iOS\"]", "`thermal_state`'s `platforms` names `iOS`, which is neither ios nor android"),
            ("\"ios\"", "`thermal_state`'s `platforms` is no list of platforms"),
            ("[]", "`thermal_state`'s `platforms` is no list of platforms"),
        ] {
            let error = read(&RECORD.replace("platforms = [\"ios\"]", &format!("platforms = {platforms}")), "baseline-platforms").unwrap_err();
            assert!(error.contains(refusal), "{platforms}: {error}");
        }
    }

    #[test]
    fn a_spread_of_no_values_or_of_a_value_that_is_no_number_is_refused() {
        let none = RECORD.replace("values = [1300, 1200, 1100]", "values = []");
        assert!(read(&none, "baseline-no-values").unwrap_err().contains("xiaomi-17-pro-max: `first_preview_ms` holds no values"), "a median of no runs");
        let word = RECORD.replace("values = [1300, 1200, 1100]", "values = [1300, \"1200\", 1100]");
        assert!(
            read(&word, "baseline-value-word").unwrap_err().contains("xiaomi-17-pro-max: `first_preview_ms` holds a value that is no number"),
            "a run that measured a word"
        );
    }

    #[test]
    fn a_comparison_of_no_run_is_refused() {
        let none = RECORD.replace("[comparison]\nruns = 3", "[comparison]\nruns = 0");
        assert!(read(&none, "baseline-no-comparison").unwrap_err().contains("a release is compared on 0 runs, which compares nothing"));
    }

    #[test]
    fn a_figure_or_a_record_that_is_no_table_is_refused() {
        let head = RECORD.split("[[figure]]").next().unwrap();
        assert!(read(&format!("figure = [1]\n{head}"), "baseline-figure-no-table").unwrap_err().contains("figure 1 is no table"));
        let figures = RECORD.split("[[device]]").next().unwrap();
        assert!(read(&format!("device = [\"x\"]\n{figures}"), "baseline-device-no-table").unwrap_err().contains("device 1 is no table"));
    }

    #[test]
    fn the_record_a_note_could_not_fill_is_refused_naming_the_device_and_figure() {
        let no_median = RECORD.replace("median = 3894.05\n", "");
        assert!(
            read(&no_median, "baseline-no-median").unwrap_err().contains("iphone-12-pro: `first_preview_ms` has no number `median`"),
            "a figure with no median is refused, not printed as nothing"
        );
        let string_median = RECORD.replace("median = 3894.05", "median = \"slow\"");
        assert!(read(&string_median, "baseline-string-median").unwrap_err().contains("no number `median`"), "a median that is a word is refused");
        let no_table = RECORD.replace("[device.thermal_state]", "[device.not_a_figure_of_ios]");
        assert!(read(&no_table, "baseline-no-table").unwrap_err().contains("no [[figure]]"), "a record naming a figure no [[figure]] defines is refused");
        let fewer_values = RECORD.replace("values = [1300, 1200, 1100]", "values = [1300, 1200]");
        assert!(
            read(&fewer_values, "baseline-fewer-values").unwrap_err().contains("2 values for 3 runs"),
            "values that do not answer the record's runs are refused"
        );
        let out_of_its_range = RECORD.replace("min = 1100", "min = 1250");
        assert!(
            read(&out_of_its_range, "baseline-median-out-of-range").unwrap_err().contains("outside its own range"),
            "a median its own record contradicts is refused"
        );
    }

    #[test]
    fn a_file_the_parser_cannot_read_is_refused_as_the_file_it_is() {
        let broken = RECORD.replace("id = \"xiaomi-17-pro-max\"", "id = xiaomi-17-pro-max");
        assert!(read(&broken, "baseline-not-toml").unwrap_err().contains("is not valid TOML"), "a file no TOML reader can read is refused as the file it is");
    }

    #[test]
    fn a_record_rests_on_the_runs_a_baseline_rests_on() {
        let shallow = RECORD
            .replace("os = \"Android 17 (API 37)\"\nrecorded = \"2026-11-20\"\nruns = 3", "os = \"Android 17 (API 37)\"\nrecorded = \"2026-11-20\"\nruns = 2");
        assert!(
            read(&shallow, "baseline-shallow").unwrap_err().contains("2 runs, fewer than the 3 a baseline rests on"),
            "a record of fewer runs than the file's own comparison is no record"
        );
    }

    #[test]
    fn one_id_names_one_record() {
        let twice = RECORD.replace("id = \"xiaomi-17-pro-max\"", "id = \"iphone-12-pro\"");
        assert!(
            read(&twice, "baseline-id-twice").unwrap_err().contains("a second record carries this id, so a note could name either"),
            "two records under one id: a note that could name either names neither"
        );
    }

    #[test]
    fn a_record_of_no_platform_we_read_is_refused() {
        let misspelt = RECORD.replace("platform = \"android\"", "platform = \"androids\"");
        assert!(
            read(&misspelt, "baseline-platform-misspelt")
                .unwrap_err()
                .contains("the platform `androids` is neither ios nor android, so no figure of it can be read"),
            "figures are read through a platform, so a platform no figure stands on records nothing"
        );
    }

    #[test]
    fn a_figure_asked_of_the_wrong_phone_is_refused() {
        // The Xiaomi is the last record, so a table written after it belongs to it: iOS's own figure,
        // recorded beside a phone that measures nothing of it.
        let misplaced = format!("{RECORD}[device.thermal_state]\nmedian = 0\nmin = 0\nmax = 0\nvalues = [0, 0, 0]\n");
        let error = read(&misplaced, "baseline-misplaced-figure").unwrap_err();
        assert!(
            error.contains("xiaomi-17-pro-max: records `thermal_state`, which no run on android measures"),
            "a record carrying a figure its platform does not measure is refused: {error}"
        );
    }

    #[test]
    fn a_file_that_holds_no_record_or_no_figure_is_refused() {
        let no_device = RECORD.split("[[device]]").next().unwrap();
        assert!(read(no_device, "baseline-no-device").unwrap_err().contains("records no device"), "a baseline of no phone says nothing about any release");
        let no_figure = RECORD.split("[[device]]").next().unwrap().split("[[figure]]").next().unwrap();
        assert!(read(no_figure, "baseline-no-figure").unwrap_err().contains("defines no [[figure]]"), "a baseline of no figure has nothing to name");
    }

    #[test]
    fn a_baseline_of_no_file_is_refused_as_a_missing_file() {
        let scratch = crate::scratch::Scratch::new("baseline-no-file");
        assert!(baseline(&scratch.0).unwrap_err().contains(BASELINE), "the refusal names the file the release's notes could not read");
    }

    #[test]
    fn a_field_spanning_a_line_is_refused() {
        let tabbed = RECORD.replace("device = \"iPhone 12 Pro (iPhone13,3)\"", "device = \"iPhone 12 Pro\\t(iPhone13,3)\"");
        assert!(
            read(&tabbed, "baseline-tabbed-field").unwrap_err().contains("which no record line carries"),
            "a device named across the fields of its own line is refused"
        );
    }
}
