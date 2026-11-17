//! Step 3 of the update loop (plan §24): which adaptations an upstream revision change touches.
//!
//! `tools/adaptations.toml` lists, for each adaptation, the upstream items it relies on, each item with
//! the places it occupies at the revision the pin carries: one or more files, each with its lines or
//! whole. An item may be a description rather than a name, so it is never looked up by name: this module
//! reads the list as TOML, asks git for the difference between the old and the new revision of every
//! listed file, and names every adaptation with an item whose lines the difference touches, or whose file
//! is listed whole and differs, with that difference for a person to recheck. A change above an item's
//! lines moves them without altering the item, so the item is listed apart with the lines it moved to,
//! for the list to be refreshed with the recheck. An adaptation upstream made needless stays in the list,
//! marked `retired` with the upstream commit whose interface replaced it and with its items removed, so
//! its id is never reused, and the report lists retired entries apart from the live ones.
//!
//! What git is asked is behind [`Revisions`], so the tests give it a scratch repository's revisions and
//! the update gives it the machine's caches of the official repositories.

use std::path::Path;

/// The list of adaptations, from the repository root.
pub(crate) const LIST: &str = "tools/adaptations.toml";

/// Which upstream repository a listed file belongs to. The Craft libraries are read at the revision an
/// app's own crates build them at: EffectCraft and FilmCraft each lock a revision of their own, so a file
/// of the libraries has no one revision until the app that relies on it is named.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Repository {
    Effectcraft,
    Filmcraft,
    /// The Craft libraries as EffectCraft's crates build them.
    EffectcraftLibraries,
    /// The Craft libraries as FilmCraft's crates build them.
    FilmcraftLibraries,
}

impl Repository {
    /// The app whose build decides the revision: the repository itself, or the app whose libraries these
    /// are.
    pub(crate) fn app(self) -> Self {
        match self {
            Self::Effectcraft | Self::EffectcraftLibraries => Self::Effectcraft,
            Self::Filmcraft | Self::FilmcraftLibraries => Self::Filmcraft,
        }
    }

    /// The Craft libraries as this repository's app builds them.
    pub(crate) fn libraries(self) -> Self {
        match self.app() {
            Self::Filmcraft => Self::FilmcraftLibraries,
            _ => Self::EffectcraftLibraries,
        }
    }

    pub(crate) fn is_libraries(self) -> bool {
        self != self.app()
    }

    /// The repository as the layer table spells it, host and all: the one place a repository is spelt.
    pub(crate) fn spelt(self) -> Option<&'static str> {
        if self.is_libraries() {
            return Some(crate::layers::CRAFT_LIBRARY_REPOSITORY);
        }
        let noun = if self == Self::Filmcraft { "film" } else { "effects" };
        crate::layers::app_by_noun(noun).map(|app| app.repository)
    }

    /// The repository's `owner/name`, as the list writes it.
    fn path(self) -> Option<&'static str> {
        self.spelt().and_then(|spelt| spelt.split_once('/')).map(|(_, path)| path)
    }

    /// The repository an entry's `repository` names, or EffectCraft's when it names none. An entry is an
    /// app's: the libraries are named on a place, where the entry's app says at which revision.
    fn named(name: Option<&str>) -> Option<Self> {
        let Some(name) = name else { return Some(Self::Effectcraft) };
        [Self::Effectcraft, Self::Filmcraft].into_iter().find(|repository| repository.path() == Some(name))
    }

    /// A file of this repository as a message names it: a library's file is told from the app's own,
    /// whose folders it can share names with.
    pub(crate) fn shown(self, file: &str) -> String {
        match self.path().filter(|_| self.is_libraries()) {
            Some(path) => format!("{path}:{file}"),
            None => file.to_string(),
        }
    }
}

/// Lines of one place, as the list writes them and as numbers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub(crate) text: String,
    first: u64,
    last: u64,
}

impl Span {
    /// The span written back as the list writes one: a single line, or `first-last`.
    fn written(first: u64, last: u64) -> String {
        if first == last { first.to_string() } else { format!("{first}-{last}") }
    }
}

/// One place an item occupies: a file, and its lines there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Location {
    /// The entry's own repository, or the Craft libraries as the entry's app builds them.
    pub(crate) repository: Repository,
    pub(crate) file: String,
    /// Empty: the whole file.
    pub(crate) lines: Vec<Span>,
}

/// One item an adaptation relies on, at every place it occupies: one claim over many sites stays one
/// item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Relies {
    pub(crate) item: String,
    pub(crate) at: Vec<Location>,
}

/// One adaptation of the list: live, or retired with the upstream commit whose interface replaced it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Adaptation {
    pub(crate) id: String,
    /// The upstream commit whose interface replaced this adaptation, on a retired entry; `None` on a
    /// live one. A retired entry is read for its id and this commit and nothing else, so the fields
    /// below hold what no reading of a retired entry ever touches: no crate, no places.
    pub(crate) retired: Option<String>,
    pub(crate) crate_name: String,
    pub(crate) repository: Repository,
    pub(crate) relies: Vec<Relies>,
}

/// A retired entry of the list. Nothing of it can change with a revision, since it lists no items, so
/// the update's report lists retired entries apart from the live ones.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Retired {
    pub(crate) id: String,
    /// The upstream commit whose interface replaced the adaptation.
    pub(crate) replaced_by: String,
}

/// What the update asks of git about two revisions of one repository.
pub(crate) trait Revisions {
    /// Does the old revision hold `file`?
    fn holds(&self, repository: Repository, file: &str) -> Result<bool, String>;
    /// `git diff -U0` of `file` from the old to the new revision; empty when it did not change.
    fn diff(&self, repository: Repository, file: &str) -> Result<String, String>;
}

fn span(text: &str) -> Result<Span, String> {
    let number = |part: &str| part.trim().parse::<u64>().ok().filter(|number| *number > 0);
    let (first, last) = match text.split_once('-') {
        Some((first, last)) => (number(first), number(last)),
        None => (number(text), number(text)),
    };
    match (first, last) {
        (Some(first), Some(last)) if first <= last => Ok(Span { text: text.to_string(), first, last }),
        _ => Err(format!("`{text}` is no line or `first-last` range of lines")),
    }
}

fn text_of<'a>(table: &'a toml::Value, key: &str, what: &str) -> Result<&'a str, String> {
    table.get(key).and_then(toml::Value::as_str).ok_or_else(|| format!("{what} names no `{key}` string"))
}

/// The keys an item and each of its places may hold. Any other, a misspelt `lines` say, would leave a
/// place read as the whole file, compared on terms nobody wrote.
const ITEM_KEYS: [&str; 2] = ["item", "at"];
const PLACE_KEYS: [&str; 3] = ["file", "lines", "repository"];

/// The table `value` is, holding only `keys`: what an item or a place is written as.
fn table_of<'a>(value: &'a toml::Value, keys: &[&str], form: &str, what: &str) -> Result<&'a toml::Table, String> {
    // An item written as one sentence, its file and lines in its prose, cannot be compared: it is refused
    // by name rather than read as relying on nothing.
    let table = value.as_table().ok_or_else(|| format!("{what}: {value} is no {form} table: write the item and each of its places apart"))?;
    match table.keys().find(|key| !keys.contains(&key.as_str())) {
        Some(key) => Err(format!("{what}: {value} holds `{key}`, which is none of {}: write the form {form}", keys.join(", "))),
        None => Ok(table),
    }
}

/// The lines of one place: none for the whole file, else each a line or a `first-last` range.
fn lines_of(place: &toml::Table, what: &str) -> Result<Vec<Span>, String> {
    match place.get("lines") {
        None => Ok(Vec::new()),
        Some(toml::Value::Array(lines)) if lines.is_empty() => {
            Err(format!("{what}: `lines` lists no line: leave the key out for a place that is the whole file"))
        }
        Some(toml::Value::Array(lines)) => lines
            .iter()
            .map(|line| {
                line.as_str()
                    .ok_or_else(|| format!("{what}: a line entry is no string"))
                    .and_then(|line| span(line).map_err(|error| format!("{what}: {error}")))
            })
            .collect(),
        Some(_) => Err(format!("{what}: `lines` is no list of strings")),
    }
}

/// The repository a place's file is read in: the entry's own, unless the place names the Craft libraries,
/// which it reads as the entry's app builds them. No other repository may stand on a place, since an
/// adaptation relies on its own app and the libraries that app builds.
fn place_repository(place: &toml::Table, entry: Repository, what: &str) -> Result<Repository, String> {
    let Some(named) = place.get("repository") else { return Ok(entry) };
    let libraries = entry.libraries();
    match (named.as_str(), libraries.path()) {
        (Some(name), Some(path)) if name == path => Ok(libraries),
        _ => Err(format!(
            "{what}: a place's `repository` may only be the Craft libraries, {}, read as the entry's app builds them",
            libraries.path().unwrap_or_default()
        )),
    }
}

/// One item of an adaptation, with every place it occupies, each file of each repository once.
fn relies_of(depend: &toml::Value, what: &str, entry: Repository) -> Result<Relies, String> {
    const FORM: &str = "{ item, at = [{ file, lines }] }";
    let table = table_of(depend, &ITEM_KEYS, FORM, what)?;
    let item = table.get("item").and_then(toml::Value::as_str).ok_or_else(|| format!("{what}: {depend} names no `item` string"))?;
    let what = format!("{what}: {item}");
    let places =
        table.get("at").and_then(toml::Value::as_array).filter(|at| !at.is_empty()).ok_or_else(|| format!("{what}: `at` lists no place it occupies"))?;
    let mut at: Vec<Location> = Vec::new();
    for place in places {
        let place = table_of(place, &PLACE_KEYS, "{ file, lines }", &what)?;
        let file = place.get("file").and_then(toml::Value::as_str).ok_or_else(|| format!("{what}: a place names no `file` string"))?;
        let repository = place_repository(place, entry, &format!("{what}: {file}"))?;
        let shown = repository.shown(file);
        if at.iter().any(|known| known.repository == repository && known.file == file) {
            return Err(format!("{what}: {shown} is listed twice: list its lines in one place"));
        }
        at.push(Location { repository, file: file.to_string(), lines: lines_of(place, &format!("{what}: {shown}"))? });
    }
    Ok(Relies { item: item.to_string(), at })
}

/// The number of an adaptation's id, `AD<n>` spelt the one way: `AD01` or `AD+1` would be a second spelling
/// of an id the order already holds.
fn id_number(id: &str) -> Option<u64> {
    let number: u64 = id.strip_prefix("AD")?.parse().ok()?;
    (number > 0 && id == format!("AD{number}")).then_some(number)
}

/// An entry as a message names it: its place in the list, and the task that lands it when it says.
fn entry_named(position: usize, entry: &toml::Value) -> String {
    match entry.get("implements").and_then(toml::Value::as_str) {
        Some(task) => format!("{position} ({task})"),
        None => position.to_string(),
    }
}

/// The line each `[[adaptation]]` entry of a list's text starts on, found by the TOML parser's own spans.
fn entry_lines(text: &str) -> Vec<usize> {
    let line_of = |offset: usize| text.get(..offset).map_or(1, |before| before.matches('\n').count().saturating_add(1));
    let table = toml::de::DeTable::parse(text).ok();
    let entries = table.as_ref().and_then(|table| table.get_ref().get("adaptation")).and_then(|entries| entries.get_ref().as_array());
    entries.map(|entries| entries.iter().map(|entry| line_of(entry.span().start)).collect()).unwrap_or_default()
}

/// One entry of the list, once its id is known to be greater than every id `numbers` holds, which takes it.
fn adaptation_of(position: usize, entry: &toml::Value, numbers: &mut Vec<(u64, String)>) -> Result<Adaptation, String> {
    let id = text_of(entry, "id", &format!("{LIST}: an adaptation"))?;
    let what = format!("{LIST}: {id}");
    let named = entry_named(position.saturating_add(1), entry);
    let number = id_number(id).ok_or_else(|| format!("{what}: is no id of the form AD1, AD2 and on"))?;
    if let Some((_, earlier)) = numbers.iter().find(|(known, _)| *known == number) {
        return Err(format!("{LIST}: entries {earlier} and {named} both have id {id}: the later one takes an id greater than the last entry's"));
    }
    if let Some((last, earlier)) = numbers.last().filter(|(last, _)| number < *last) {
        return Err(format!("{what}: entry {named} comes after entry {earlier}, whose id AD{last} is greater"));
    }
    numbers.push((number, named));
    // A retired adaptation keeps its entry so its id is never taken again. The id and its place in
    // the order are checked above with every other entry's; of the rest of a retired entry the
    // reader reads only the commit that replaced it, written as its id so that it names one commit
    // for good, and refuses `depends` whatever it holds: an item left in any form reads as one the
    // adaptation still relies on.
    if let Some(retired) = entry.get("retired") {
        let commit = retired.as_str().filter(|commit| crate::upstream::is_commit_id(commit)).ok_or_else(|| {
            format!("{what}: is retired without its upstream commit: `retired` is {retired}, no commit id in full or abbreviated as git writes one")
        })?;
        if entry.get("depends").is_some() {
            return Err(format!("{what}: is retired but keeps `depends`: a retired entry holds no items, only the commit {commit} that replaced it"));
        }
        return Ok(Adaptation {
            id: id.to_string(),
            retired: Some(commit.to_string()),
            crate_name: String::new(),
            repository: Repository::Effectcraft,
            relies: Vec::new(),
        });
    }
    let named = match entry.get("repository") {
        None => None,
        Some(value) => Some(value.as_str().ok_or_else(|| format!("{what}: `repository` is no string"))?),
    };
    let repository =
        Repository::named(named).ok_or_else(|| format!("{what}: relies on {}, a repository the update does not read", named.unwrap_or_default()))?;
    let depends = entry
        .get("depends")
        .and_then(toml::Value::as_array)
        .filter(|depends| !depends.is_empty())
        .ok_or_else(|| format!("{what}: lists nothing it relies on"))?;
    let relies = depends.iter().map(|depend| relies_of(depend, &what, repository)).collect::<Result<Vec<_>, _>>()?;
    Ok(Adaptation { id: id.to_string(), retired: None, crate_name: text_of(entry, "crate", &what)?.to_string(), repository, relies })
}

/// The adaptations of a list's text. Each id is greater than the last entry's, and no id is taken twice: two
/// changes that each add an adaptation take the same next id, and the gate passes each alone, so only the
/// reader refuses the second once both have merged. A gap is allowed. A retired entry's id holds its place
/// in the order as every other id does, so it is never reused; of the rest of a retired entry the reader
/// checks that it names its commit and holds no items, and reads nothing else. A refusal of an entry names
/// the line the entry starts on, for a paste or a merge that left a table empty or an entry twice.
pub(crate) fn parse(text: &str) -> Result<Vec<Adaptation>, String> {
    let document: toml::Value = toml::from_str(text).map_err(|error| format!("{LIST}: is not valid TOML: {error}"))?;
    let entries = document.get("adaptation").and_then(toml::Value::as_array).ok_or_else(|| format!("{LIST}: lists no [[adaptation]]"))?;
    let lines = entry_lines(text);
    let mut numbers: Vec<(u64, String)> = Vec::new();
    entries
        .iter()
        .enumerate()
        .map(|(position, entry)| {
            adaptation_of(position, entry, &mut numbers).map_err(|error| match lines.get(position) {
                Some(line) => format!("{error} (the entry starting at line {line})"),
                None => error,
            })
        })
        .collect()
}

/// The adaptations of the repository's list.
pub(crate) fn read(root: &Path) -> Result<Vec<Adaptation>, String> {
    parse(&crate::scan::read_text(&root.join(LIST))?)
}

/// One hunk of `git diff -U0`: where it starts and how many lines it replaces in the old revision, and
/// how many it puts in their place.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Hunk {
    old_start: u64,
    old_count: u64,
    new_count: u64,
    text: String,
}

impl Hunk {
    /// Does this hunk change a line of `span`? An insertion lies after line `old_start`, so it touches
    /// the span only when it lands between two of the span's lines.
    fn touches(&self, span: &Span) -> bool {
        if self.old_count == 0 {
            span.first <= self.old_start && self.old_start < span.last
        } else {
            self.old_start <= span.last && self.old_start.saturating_add(self.old_count) > span.first
        }
    }

    /// Does this hunk lie wholly above `span`, moving its lines without touching them?
    fn precedes(&self, span: &Span) -> bool {
        if self.old_count == 0 { self.old_start < span.first } else { self.old_start.saturating_add(self.old_count) <= span.first }
    }
}

/// `start[,count]`, one side of a hunk header: a count left out is one line.
fn hunk_side(side: &str) -> Option<(u64, u64)> {
    let (start, count) = side.split_once(',').unwrap_or((side, "1"));
    Some((start.parse().ok()?, count.parse().ok()?))
}

/// The hunks of a `git diff -U0`, by the `@@ -start[,count] +start[,count] @@` headers git writes.
fn hunks(diff: &str) -> Result<Vec<Hunk>, String> {
    let mut found: Vec<Hunk> = Vec::new();
    for line in diff.lines() {
        if let Some(header) = line.strip_prefix("@@ -") {
            let mut sides = header.split_whitespace();
            let old = sides.next().and_then(hunk_side);
            let new = sides.next().and_then(|side| side.strip_prefix('+')).and_then(hunk_side);
            let (Some((old_start, old_count)), Some((_, new_count))) = (old, new) else {
                return Err(format!("git wrote a hunk header this cannot read: {line}"));
            };
            found.push(Hunk { old_start, old_count, new_count, text: String::new() });
        }
        if let Some(hunk) = found.last_mut() {
            hunk.text.push_str(line);
            hunk.text.push('\n');
        }
    }
    Ok(found)
}

/// One place of an item of an adaptation that the revision change touched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Touched {
    pub(crate) item: String,
    pub(crate) file: String,
    /// Where in the file: `lines 104-269`, or `the whole file`.
    pub(crate) place: String,
    /// The hunks that touch it, as git wrote them.
    pub(crate) diff: String,
}

/// An adaptation to recheck, with what touched it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Recheck {
    pub(crate) id: String,
    pub(crate) crate_name: String,
    pub(crate) touched: Vec<Touched>,
}

/// An item whose lines the revision change moved without touching them: the list is refreshed to say
/// where they now are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Moved {
    pub(crate) id: String,
    pub(crate) item: String,
    pub(crate) file: String,
    /// The spans that moved, as the list writes them, and the same spans at the new revision.
    pub(crate) from: String,
    pub(crate) to: String,
}

/// What comparing two revisions against the list found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Comparison {
    pub(crate) recheck: Vec<Recheck>,
    pub(crate) moved: Vec<Moved>,
    /// The list's retired entries, in the list's order, which no comparison can touch.
    pub(crate) retired: Vec<Retired>,
}

/// Where `span` lies at the new revision, given every hunk of its file that does not touch it: each
/// hunk above it moves it by the lines it adds less the lines it removes.
fn moved_span(span: &Span, changes: &[Hunk]) -> Result<(u64, u64), String> {
    let shift: i128 = changes.iter().filter(|hunk| hunk.precedes(span)).map(|hunk| i128::from(hunk.new_count) - i128::from(hunk.old_count)).sum();
    let at = |line: u64| u64::try_from(i128::from(line) + shift).ok().filter(|line| *line > 0);
    match (at(span.first), at(span.last)) {
        (Some(first), Some(last)) => Ok((first, last)),
        _ => Err(format!("lines {} move out of the file by the hunks git wrote", span.text)),
    }
}

/// The files the old revision does not hold, as `(adaptation id, file)`: a list that names a file that is
/// not there cannot be compared against anything.
pub(crate) fn missing(list: &[Adaptation], revisions: &dyn Revisions) -> Result<Vec<(String, String)>, String> {
    let mut absent = Vec::new();
    for adaptation in list {
        for location in adaptation.relies.iter().flat_map(|relies| &relies.at) {
            let shown = location.repository.shown(&location.file);
            if !revisions.holds(location.repository, &location.file)? && !absent.contains(&(adaptation.id.clone(), shown.clone())) {
                absent.push((adaptation.id.clone(), shown));
            }
        }
    }
    Ok(absent)
}

/// Compare the two revisions against every live adaptation of the list. A retired entry lists no items
/// for a revision to touch, so it is named apart, as retired, instead of compared.
pub(crate) fn compare(list: &[Adaptation], revisions: &dyn Revisions) -> Result<Comparison, String> {
    let mut comparison = Comparison::default();
    for adaptation in list {
        if let Some(replaced_by) = &adaptation.retired {
            comparison.retired.push(Retired { id: adaptation.id.clone(), replaced_by: replaced_by.clone() });
            continue;
        }
        let mut touched = Vec::new();
        for (relies, location) in adaptation.relies.iter().flat_map(|relies| relies.at.iter().map(move |location| (relies, location))) {
            let diff = revisions.diff(location.repository, &location.file)?;
            let shown = location.repository.shown(&location.file);
            let what = |error: String| format!("{}: {shown}: {error}", adaptation.id);
            let changes = hunks(&diff).map_err(what)?;
            if changes.is_empty() {
                continue;
            }
            let (place, selected): (String, Vec<&Hunk>) = if location.lines.is_empty() {
                ("the whole file".to_string(), changes.iter().collect())
            } else {
                let selected = changes.iter().filter(|hunk| location.lines.iter().any(|span| hunk.touches(span))).collect();
                (format!("lines {}", location.lines.iter().map(|span| span.text.as_str()).collect::<Vec<_>>().join(", ")), selected)
            };
            if !selected.is_empty() {
                touched.push(Touched { item: relies.item.clone(), file: shown.clone(), place, diff: selected.iter().map(|hunk| hunk.text.as_str()).collect() });
            }
            // A span the change left alone may still have moved, whether or not another span of the
            // place was touched: the recheck rewrites the touched ones, and nothing else would tell it
            // where the rest went.
            let (mut from, mut now) = (Vec::new(), Vec::new());
            for span in location.lines.iter().filter(|span| !changes.iter().any(|hunk| hunk.touches(span))) {
                let (first, last) = moved_span(span, &changes).map_err(what)?;
                if (first, last) != (span.first, span.last) {
                    from.push(span.text.as_str());
                    now.push(Span::written(first, last));
                }
            }
            if !now.is_empty() {
                comparison.moved.push(Moved {
                    id: adaptation.id.clone(),
                    item: relies.item.clone(),
                    file: shown.clone(),
                    from: from.join(", "),
                    to: now.join(", "),
                });
            }
        }
        if !touched.is_empty() {
            comparison.recheck.push(Recheck { id: adaptation.id.clone(), crate_name: adaptation.crate_name.clone(), touched });
        }
    }
    Ok(comparison)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST_TEXT: &str = r#"
[[adaptation]]
id = "AD1"
crate = "craftbridge-effects"
depends = [
  { item = "Session", at = [{ file = "crates/engine/src/lib.rs", lines = ["10-20"] }] },
  { item = "probe", at = [{ file = "crates/media/src/probe.rs", lines = ["5", "30-31"] }] },
]

[[adaptation]]
id = "AD2"
crate = "craftbridge-effects"
depends = [
  { item = "the render queue commands", at = [{ file = "crates/engine/src/cmd/render_queue.rs" }] },
  { item = "every sentence that ends `not available in this build`", at = [
    { file = "crates/engine/src/cmd/effect.rs", lines = ["39", "91"] },
    { file = "crates/engine/src/cmd/proxy.rs", lines = ["44"] },
  ] },
]

[[adaptation]]
id = "AD3"
crate = "craftbridge-media"
repository = "storytold/filmcraft"
depends = [
  { item = "register", at = [{ file = "crates/codecs/src/lib.rs", lines = ["99"] }] },
]
"#;

    /// Revisions that answer from a table of diffs, as git would.
    struct Fixed(Vec<(Repository, &'static str, &'static str)>);

    impl Revisions for Fixed {
        fn holds(&self, _: Repository, _: &str) -> Result<bool, String> {
            Ok(true)
        }
        fn diff(&self, repository: Repository, file: &str) -> Result<String, String> {
            Ok(self.0.iter().find(|(known, name, _)| *known == repository && *name == file).map(|(.., diff)| diff.to_string()).unwrap_or_default())
        }
    }

    #[test]
    fn the_list_is_read_as_toml_with_every_item_and_its_lines() {
        let list = parse(LIST_TEXT).unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list.first().map(|ad| ad.relies.len()), Some(2));
        assert_eq!(list.get(2).map(|ad| ad.repository), Some(Repository::Filmcraft));
        let place = |ad: usize, item: usize| {
            list.get(ad).and_then(|ad| ad.relies.get(item)).map(|relies| relies.at.iter().map(|at| (at.file.as_str(), at.lines.len())).collect::<Vec<_>>())
        };
        assert_eq!(place(1, 0), Some(vec![("crates/engine/src/cmd/render_queue.rs", 0)]), "a place without lines is the whole file");
        assert_eq!(
            place(1, 1),
            Some(vec![("crates/engine/src/cmd/effect.rs", 2), ("crates/engine/src/cmd/proxy.rs", 1)]),
            "one claim over several files is one item"
        );
        assert!(parse(&LIST_TEXT.replace("\"10-20\"", "\"20-10\"")).is_err_and(|error| error.contains("20-10")));
        assert!(parse(&LIST_TEXT.replace("storytold/filmcraft", "someone/else")).is_err_and(|error| error.contains("someone/else")));
        assert!(parse(&LIST_TEXT.replace("storytold/filmcraft", "filmcraft")).is_err(), "a repository is named by owner and name");
        assert_eq!(
            parse(&LIST_TEXT.replace("repository = \"storytold/filmcraft\"", "repository = \"storytold/effectcraft\"")).unwrap().get(2).map(|ad| ad.repository),
            Some(Repository::Effectcraft)
        );
        assert!(parse("[[adaptation]]\nid = \"AD1\"\ncrate = \"c\"\n").is_err_and(|error| error.contains("relies on")));
    }

    #[test]
    fn an_item_written_as_one_sentence_is_refused_by_name() {
        let probe = "{ item = \"probe\", at = [{ file = \"crates/media/src/probe.rs\", lines = [\"5\", \"30-31\"] }] }";
        let old_form = LIST_TEXT.replace(probe, "\"probe (crates/media/src/probe.rs:5, :30-31)\"");
        assert!(parse(&old_form).is_err_and(|error| error.contains("AD1") && error.contains("probe (crates/media/src/probe.rs:5, :30-31)")));
        let one_file = LIST_TEXT.replace(probe, "{ item = \"probe\", file = \"crates/media/src/probe.rs\", lines = [\"5\"] }");
        assert!(
            parse(&one_file).is_err_and(|error| error.contains("AD1") && error.contains("`file`") && error.contains("at = [")),
            "a place is written in `at`"
        );
        let misspelt = LIST_TEXT.replace("lines = [\"99\"]", "line = [\"99\"]");
        assert!(parse(&misspelt).is_err_and(|error| error.contains("AD3") && error.contains("`line`")), "a misspelt key is not read as the whole file");
        let empty = LIST_TEXT.replace("lines = [\"99\"]", "lines = []");
        assert!(parse(&empty).is_err_and(|error| error.contains("AD3") && error.contains("register")), "no lines is not the whole file");
        let nowhere = LIST_TEXT.replace("at = [{ file = \"crates/codecs/src/lib.rs\", lines = [\"99\"] }]", "at = []");
        assert!(parse(&nowhere).is_err_and(|error| error.contains("register") && error.contains("no place")), "an item is somewhere");
        let twice = LIST_TEXT.replace("crates/engine/src/cmd/proxy.rs", "crates/engine/src/cmd/effect.rs");
        assert!(parse(&twice).is_err_and(|error| error.contains("AD2") && error.contains("twice")), "a file is one place of an item");
    }

    /// An entry pasted inside the one before it, as a rebase can leave the file: AD3's header lands in
    /// AD2's `depends` list. The list cannot be read, and the refusal says where.
    #[test]
    fn a_list_that_does_not_parse_is_refused_naming_its_line() {
        let close_ad2 = "  ] },\n]\n\n[[adaptation]]\nid = \"AD3\"";
        assert_eq!(LIST_TEXT.matches(close_ad2).count(), 1, "the fixture closes AD2's list just before AD3");
        let broken = LIST_TEXT.replace(close_ad2, "  ] },\n\n[[adaptation]]\nid = \"AD3\"");
        // TOML reads the header inside a list as a list in it, so the first line it cannot read is AD3's id.
        let failing = broken.lines().position(|line| line == "id = \"AD3\"").map(|at| at + 1).unwrap();
        let refused = parse(&broken).unwrap_err();
        assert!(refused.starts_with(LIST) && refused.contains(&format!("line {failing},")), "the file and the line TOML stops at: {refused}");
    }

    /// A header left twice, as a merge that keeps both sides can: the list is valid TOML with an empty
    /// entry, so the refusal names the line the entry starts on, not one TOML stops at.
    #[test]
    fn an_entry_that_parses_but_is_refused_names_the_line_it_starts_on() {
        let twice = LIST_TEXT.replace("[[adaptation]]\nid = \"AD3\"", "[[adaptation]]\n[[adaptation]]\nid = \"AD3\"");
        // The headers are AD1's, AD2's, the empty entry's and AD3's.
        let headers = twice.lines().enumerate().filter(|(_, line)| *line == "[[adaptation]]").map(|(at, _)| at + 1).collect::<Vec<_>>();
        let empty = headers.get(2).copied().unwrap();
        let refused = parse(&twice).unwrap_err();
        assert!(refused.starts_with(LIST) && refused.ends_with(&format!("line {empty})")), "the empty entry's own line: {refused}");
        let misspelt = LIST_TEXT.replace("lines = [\"99\"]", "line = [\"99\"]");
        // AD3's header is the line above its id, and a line number counts from 1.
        let ad3 = misspelt.lines().position(|line| line == "id = \"AD3\"").unwrap();
        let refused = parse(&misspelt).unwrap_err();
        assert!(refused.ends_with(&format!("line {ad3})")), "the entry that holds the misspelling: {refused}");
    }

    #[test]
    fn the_real_list_parses_with_one_entry_for_each_adaptation() {
        assert!(!read(&crate::repo_root()).unwrap().is_empty(), "the reader holds each id to the order, once");
    }

    #[test]
    fn an_id_is_greater_than_the_last_entry_s_and_names_one_adaptation() {
        let list = |ids: &[(&str, &str)]| -> String { ids.iter().map(|(id, task)| live(id, task)).collect() };
        assert!(parse(&list(&[("AD1", "FX-1a"), ("AD2", "FX-1b"), ("AD3", "MED-1a")])).is_ok());
        let twice = parse(&list(&[("AD1", "FX-1a"), ("AD2", "CORE-2a"), ("AD2", "FX-1b")]));
        assert!(
            twice.as_ref().is_err_and(|error| error.contains("AD2") && error.contains("2 (CORE-2a)") && error.contains("3 (FX-1b)")),
            "both named: {twice:?}"
        );
        let again = parse(&list(&[("AD1", "FX-1a"), ("AD2", "FX-1b"), ("AD1", "CORE-2a")]));
        assert!(again.as_ref().is_err_and(|error| error.contains("1 (FX-1a)") && error.contains("3 (CORE-2a)")), "an earlier id taken again: {again:?}");
        assert!(parse(&list(&[("AD1", "FX-1a"), ("AD3", "FX-1b"), ("AD9", "MED-1a")])).is_ok(), "a gap is allowed");
        assert!(parse(&list(&[("AD4", "FX-1a")])).is_ok(), "the first entry need not be AD1");
        let backwards = parse(&list(&[("AD3", "FX-1a"), ("AD1", "FX-1b")]));
        assert!(
            backwards.as_ref().is_err_and(|error| error.contains("AD1") && error.contains("1 (FX-1a)") && error.contains("AD3")),
            "a decrease: {backwards:?}"
        );
        for odd in ["AD01", "AD+1", "AD0", "ad1", "AD", "A1", "AD1a"] {
            assert!(parse(&list(&[(odd, "FX-1a")])).is_err_and(|error| error.contains(odd)), "{odd} is no id the order reads");
        }
    }

    /// A retired entry: its id, and the upstream commit whose interface replaced it, and nothing else.
    fn retired(id: &str, commit: &str) -> String {
        format!("[[adaptation]]\nid = \"{id}\"\nretired = \"{commit}\"\n")
    }

    /// A live entry, with one item.
    fn live(id: &str, task: &str) -> String {
        format!("[[adaptation]]\nid = \"{id}\"\ncrate = \"c\"\nimplements = \"{task}\"\ndepends = [{{ item = \"i\", at = [{{ file = \"f.rs\" }}] }}]\n")
    }

    #[test]
    fn a_retired_entry_keeps_its_id_in_the_order_so_no_entry_after_it_can_take_it_again() {
        let list = parse(&format!("{}{}{}", live("AD1", "FX-1a"), retired("AD2", "0d1e2f3a4b5c"), live("AD3", "FX-1b"))).unwrap();
        assert_eq!(list.len(), 3, "the retired entry is one of the list's entries");
        let second = list.get(1).unwrap();
        assert_eq!(second.retired.as_deref(), Some("0d1e2f3a4b5c"), "the commit whose interface replaced it");
        assert!(second.relies.is_empty() && second.crate_name.is_empty(), "the reader reads no items and no crate of a retired entry");
        let taken = parse(&format!("{}{}{}", live("AD1", "FX-1a"), live("AD9", "FX-1b"), retired("AD9", "0d1e2f3a4b5c")));
        assert!(
            taken.as_ref().is_err_and(|error| error.contains("entries 2 (FX-1b) and 3 both have id AD9")),
            "a retired entry takes an id a live entry already holds, naming both: {taken:?}"
        );
        let taken = parse(&format!("{}{}{}", live("AD1", "FX-1a"), retired("AD9", "0d1e2f3a4b5c"), live("AD9", "FX-1c")));
        assert!(
            taken.as_ref().is_err_and(|error| error.contains("entries 2 and 3 (FX-1c) both have id AD9")),
            "a new entry takes the retired entry's highest id, naming both: {taken:?}"
        );
        assert!(
            parse(&format!("{}{}", retired("AD9", "0d1e2f3a4b5c"), live("AD5", "FX-1b"))).is_err_and(|error| error.contains("AD9") && error.contains("AD5")),
            "the order rule covers a retired entry's id too"
        );
    }

    #[test]
    fn a_retired_entry_holds_no_items_and_names_the_upstream_commit_that_replaced_it() {
        let keeps = "tools/adaptations.toml: AD2: is retired but keeps `depends`: a retired entry holds no items, only the commit 0d1e2f3a4b5c that replaced it (the entry starting at line 1)";
        for depends in [
            "depends = [{ item = \"i\", at = [{ file = \"f.rs\" }] }]\n",
            "depends = { item = \"i\", at = [{ file = \"f.rs\" }] }\n",
            "depends = \"i, in f.rs\"\n",
            "depends = []\n",
            "[[adaptation.depends]]\nitem = \"i\"\nat = [{ file = \"f.rs\" }]\n",
        ] {
            let with_items = retired("AD2", "0d1e2f3a4b5c") + depends;
            assert_eq!(parse(&with_items), Err(keeps.to_string()), "an item left in any form, or the key left empty: {with_items}");
        }
        let no_commit = |shown: &str| {
            format!(
                "tools/adaptations.toml: AD2: is retired without its upstream commit: `retired` is {shown}, no commit id in full or abbreviated as git writes one (the entry starting at line 1)"
            )
        };
        for commit in ["", "yes", "main", "v0.24.1", "main~3", "0D1E2F3A", " 0d1e2f3a", "0d1e2f3g", &"0".repeat(41)] {
            assert_eq!(parse(&retired("AD2", commit)), Err(no_commit(&format!("\"{commit}\""))), "a commit is named by its id: {commit:?}");
        }
        for (written, shown) in [("3", "3"), ("true", "true"), ("[\"0d1e2f3a\"]", "[\"0d1e2f3a\"]")] {
            let not_a_string = retired("AD2", "0d1e2f3a4b5c").replace("\"0d1e2f3a4b5c\"", written);
            assert_eq!(parse(&not_a_string), Err(no_commit(shown)), "a commit is written as a string: {not_a_string}");
        }
        for commit in ["1037", "0d1e2f3a4b5c", "103760584a60074c9aaeef686d2d05d89f08155e"] {
            let list = parse(&retired("AD1", commit));
            assert!(
                list.as_ref().is_ok_and(|list| list.first().and_then(|ad| ad.retired.as_deref()) == Some(commit)),
                "the retired form needs no crate, no implements and no items: {list:?}"
            );
        }
        let kept = retired("AD2", "0d1e2f3a4b5c")
            + "crate = \"craftbridge-effects\"\nfile = \"src/gone.rs\"\nimplements = \"FX-1a\"\nrepository = \"storytold/nowhere\"\n"
            + "summary = \"what the adaptation did while it lived\"\nreplaces = \"what it stood in for (crates/host/src/lib.rs:96-147)\"\n";
        assert!(parse(&kept).is_ok_and(|list| list.first().is_some_and(|ad| ad.retired.is_some())), "the reader reads nothing else of a retired entry");
    }

    #[test]
    fn a_revision_that_touches_an_item_s_lines_names_that_adaptation_with_the_diff() {
        let list = parse(LIST_TEXT).unwrap();
        let revisions = Fixed(vec![(Repository::Effectcraft, "crates/engine/src/lib.rs", "@@ -15,2 +15,3 @@ pub struct Session\n-old\n+new\n+more\n")]);
        let found = compare(&list, &revisions).unwrap();
        assert_eq!(found.recheck.len(), 1);
        let recheck = found.recheck.first().unwrap();
        assert_eq!(recheck.id, "AD1");
        assert_eq!(recheck.touched.first().map(|touched| (touched.item.as_str(), touched.place.as_str())), Some(("Session", "lines 10-20")));
        assert!(recheck.touched.first().is_some_and(|touched| touched.diff.contains("+new")), "the diff comes with the name");
        assert!(found.moved.is_empty());
    }

    #[test]
    fn a_change_above_an_item_s_lines_moves_them_and_one_below_leaves_them() {
        let list = parse(LIST_TEXT).unwrap();
        let moved = |diff: &'static str| compare(&list, &Fixed(vec![(Repository::Effectcraft, "crates/media/src/probe.rs", diff)])).unwrap();
        let found = moved("@@ -2 +2,3 @@\n-x\n+y\n+z\n+w\n@@ -10,4 +12,0 @@\n-a\n-b\n-c\n-d\n");
        assert!(found.recheck.is_empty(), "{found:?}");
        let expected = Moved {
            id: "AD1".to_string(),
            item: "probe".to_string(),
            file: "crates/media/src/probe.rs".to_string(),
            from: "5, 30-31".to_string(),
            to: "7, 28-29".to_string(),
        };
        assert_eq!(found.moved, [expected], "two lines added above line 5, four removed between it and 30");
        assert_eq!(moved("@@ -4,0 +5 @@\n+new\n").moved.first().map(|moved| moved.to.as_str()), Some("6, 31-32"), "an insertion just above line 5");
        assert_eq!(moved("@@ -40 +40,2 @@\n-x\n+y\n+z\n"), Comparison::default(), "a change below every listed line moves none of them");
        assert_eq!(moved("@@ -1 +1 @@\n-x\n+y\n"), Comparison::default(), "a change above them that keeps the line count moves none of them");
    }

    #[test]
    fn a_span_the_change_left_alone_is_located_even_when_another_span_of_its_place_was_touched() {
        let list = parse(LIST_TEXT).unwrap();
        let revisions = Fixed(vec![(Repository::Effectcraft, "crates/media/src/probe.rs", "@@ -5 +5 @@\n-a\n+b\n@@ -10,0 +11,2 @@\n+x\n+y\n")]);
        let found = compare(&list, &revisions).unwrap();
        assert_eq!(found.recheck.iter().map(|recheck| recheck.id.as_str()).collect::<Vec<_>>(), ["AD1"], "line 5 was changed");
        assert_eq!(
            found.moved.iter().map(|moved| (moved.item.as_str(), moved.from.as_str(), moved.to.as_str())).collect::<Vec<_>>(),
            [("probe", "30-31", "32-33")],
            "lines 30-31 were not touched, and two lines were added above them"
        );
        let shifted = Fixed(vec![(Repository::Effectcraft, "crates/media/src/probe.rs", "@@ -2,0 +3,2 @@\n+x\n+y\n@@ -30 +32 @@\n-a\n+b\n")]);
        let found = compare(&list, &shifted).unwrap();
        assert_eq!(found.recheck.iter().map(|recheck| recheck.id.as_str()).collect::<Vec<_>>(), ["AD1"], "line 30 was changed");
        assert_eq!(
            found.moved.iter().map(|moved| (moved.item.as_str(), moved.from.as_str(), moved.to.as_str())).collect::<Vec<_>>(),
            [("probe", "5", "7")],
            "30-31, touched, is the recheck's to rewrite, though the lines added above moved it too"
        );
    }

    #[test]
    fn the_edges_of_a_hunk_decide_whether_it_touches_the_lines() {
        let item = span("10-20").unwrap();
        let hunk = |old_start, old_count| Hunk { old_start, old_count, new_count: 1, text: String::new() };
        assert!(hunk(20, 1).touches(&item) && hunk(9, 2).touches(&item), "a changed line at either end of the range");
        assert!(!hunk(21, 1).touches(&item) && !hunk(8, 2).touches(&item), "a changed line just outside it");
        assert!(hunk(12, 0).touches(&item), "an insertion between two lines of the range");
        assert!(!hunk(9, 0).touches(&item) && !hunk(20, 0).touches(&item), "an insertion before its first line or after its last");
        assert!(hunk(8, 2).precedes(&item) && hunk(9, 0).precedes(&item), "a hunk that ends where the range starts lies above it");
        assert!(
            !hunk(9, 2).precedes(&item) && !hunk(20, 0).precedes(&item) && !hunk(21, 1).precedes(&item),
            "one that reaches into it or lies below it does not"
        );
        assert!(hunk(15, u64::MAX).touches(&item) && !hunk(u64::MAX, 2).precedes(&item), "a hostile count or start saturates rather than overflows");
        assert!(
            hunks("@@ -3,2 +3 @@\n-a\n-b\n+c\n")
                .is_ok_and(|found| found == [Hunk { old_start: 3, old_count: 2, new_count: 1, text: "@@ -3,2 +3 @@\n-a\n-b\n+c\n".to_string() }])
        );
        assert!(hunks("@@ -x +3 @@\n").is_err() && hunks("@@ -3 @@\n").is_err(), "a header this cannot read is an error, not a hunk of nothing");
    }

    #[test]
    fn a_file_listed_whole_is_touched_by_any_change_and_another_repository_is_read_apart() {
        let list = parse(LIST_TEXT).unwrap();
        let revisions = Fixed(vec![
            (Repository::Effectcraft, "crates/engine/src/cmd/render_queue.rs", "@@ -400 +400 @@\n-a\n+b\n"),
            (Repository::Filmcraft, "crates/codecs/src/lib.rs", "@@ -99 +99 @@\n-a\n+b\n"),
            (Repository::Effectcraft, "crates/codecs/src/lib.rs", "@@ -1 +1 @@\n-a\n+b\n"),
        ]);
        let found = compare(&list, &revisions).unwrap();
        assert_eq!(found.recheck.iter().map(|recheck| recheck.id.as_str()).collect::<Vec<_>>(), ["AD2", "AD3"]);
        let whole = found.recheck.first().and_then(|recheck| recheck.touched.first()).map(|touched| touched.place.as_str());
        assert_eq!(whole, Some("the whole file"), "a place without lines is named as the whole file, not as lines");
    }

    #[test]
    fn one_place_of_an_item_over_several_files_names_it_and_another_moves_alone() {
        let list = parse(LIST_TEXT).unwrap();
        let revisions = Fixed(vec![
            (Repository::Effectcraft, "crates/engine/src/cmd/proxy.rs", "@@ -44 +44 @@\n-a\n+b\n"),
            (Repository::Effectcraft, "crates/engine/src/cmd/effect.rs", "@@ -10,0 +11 @@\n+c\n"),
        ]);
        let found = compare(&list, &revisions).unwrap();
        let touched: Vec<(&str, &str, &str)> = found
            .recheck
            .iter()
            .flat_map(|recheck| recheck.touched.iter().map(|touched| (recheck.id.as_str(), touched.file.as_str(), touched.place.as_str())))
            .collect();
        assert_eq!(touched, [("AD2", "crates/engine/src/cmd/proxy.rs", "lines 44")], "the site the change touched, under its one item");
        assert_eq!(
            found.moved.iter().map(|moved| (moved.file.as_str(), moved.to.as_str())).collect::<Vec<_>>(),
            [("crates/engine/src/cmd/effect.rs", "40, 92")]
        );
    }

    /// A file of the Craft libraries is named on its place, and read as the entry's own app builds them.
    #[test]
    fn a_place_in_the_craft_libraries_is_read_as_the_entry_s_app_builds_them() {
        let in_libraries = "{ repository = \"storytold/craft-libs\", file = \"crates/media/src/isobmff/codec.rs\", lines = [\"167\"] }";
        let film = LIST_TEXT.replace(
            "at = [{ file = \"crates/codecs/src/lib.rs\", lines = [\"99\"] }]",
            &format!("at = [{{ file = \"crates/codecs/src/lib.rs\", lines = [\"99\"] }}, {in_libraries}]"),
        );
        let effects = LIST_TEXT.replace("at = [{ file = \"crates/engine/src/lib.rs\", lines = [\"10-20\"] }]", &format!("at = [{in_libraries}]"));
        let read = |text: &str, id: usize, item: usize| {
            parse(text).unwrap().get(id).and_then(|ad| ad.relies.get(item)).map(|relies| relies.at.iter().map(|at| at.repository).collect::<Vec<_>>())
        };
        assert_eq!(
            read(&film, 2, 0),
            Some(vec![Repository::Filmcraft, Repository::FilmcraftLibraries]),
            "FilmCraft's entry reads them as FilmCraft builds them"
        );
        assert_eq!(read(&effects, 0, 0), Some(vec![Repository::EffectcraftLibraries]), "EffectCraft's entry reads them as EffectCraft builds them");
        let elsewhere = LIST_TEXT.replace(
            "at = [{ file = \"crates/engine/src/lib.rs\", lines = [\"10-20\"] }]",
            "at = [{ repository = \"storytold/filmcraft\", file = \"crates/codecs/src/lib.rs\" }]",
        );
        assert!(parse(&elsewhere).is_err_and(|error| error.contains("AD1") && error.contains("Craft libraries")), "no other repository stands on a place");
        let both = film.replace(in_libraries, &format!("{{ file = \"crates/media/src/isobmff/codec.rs\" }}, {in_libraries}"));
        assert!(parse(&both).is_ok(), "one path in the app and in the libraries is two files, as EffectCraft's own crates/media is");
        let twice = film.replace(in_libraries, &format!("{in_libraries}, {in_libraries}"));
        assert!(parse(&twice).is_err_and(|error| error.contains("storytold/craft-libs:crates/media/src/isobmff/codec.rs") && error.contains("twice")));
        let found =
            compare(&parse(&film).unwrap(), &Fixed(vec![(Repository::FilmcraftLibraries, "crates/media/src/isobmff/codec.rs", "@@ -167 +167 @@\n-a\n+b\n")]))
                .unwrap();
        let touched: Vec<(&str, &str)> =
            found.recheck.iter().flat_map(|recheck| recheck.touched.iter().map(|touched| (recheck.id.as_str(), touched.file.as_str()))).collect();
        assert_eq!(touched, [("AD3", "storytold/craft-libs:crates/media/src/isobmff/codec.rs")], "named with the repository it lives in");
        let unchanged =
            compare(&parse(&film).unwrap(), &Fixed(vec![(Repository::Filmcraft, "crates/media/src/isobmff/codec.rs", "@@ -167 +167 @@\n-a\n+b\n")])).unwrap();
        assert!(unchanged.recheck.is_empty(), "a file of the same path in the app is another file: {unchanged:?}");
    }

    #[test]
    fn a_listed_file_the_old_revision_lacks_is_named() {
        struct Lacking;
        impl Revisions for Lacking {
            fn holds(&self, _: Repository, file: &str) -> Result<bool, String> {
                Ok(file != "crates/media/src/probe.rs" && file != "crates/engine/src/cmd/proxy.rs")
            }
            fn diff(&self, _: Repository, _: &str) -> Result<String, String> {
                Ok(String::new())
            }
        }
        let list = parse(LIST_TEXT).unwrap();
        let absent = [("AD1".to_string(), "crates/media/src/probe.rs".to_string()), ("AD2".to_string(), "crates/engine/src/cmd/proxy.rs".to_string())];
        assert_eq!(missing(&list, &Lacking).unwrap(), absent, "every place of every item is looked for");
    }
}
