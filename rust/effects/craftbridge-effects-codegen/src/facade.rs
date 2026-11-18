//! What every facade emitter shares: the surface's commands grouped into namespaces, and the names and
//! documentation a typed call is given.
//!
//! A facade is one typed method for each command an app can call, in one type for each namespace. A
//! command the surface marks refused under confinement or withheld by the plugin has none
//! ([`Command::callable`]): every call of it is answered with that refusal, so a method could only fail.
//! Everything that does not depend on the language is decided here once, so the Swift and the Kotlin
//! facade name, group, order and describe the same things alike: the namespace of a command is its id
//! without the last segment, the method is the last segment, a key is a parameter of the same name, the
//! keys keep the order of the `params` text, and every sentence of the documentation is written here. Only
//! how a language spells a name it reserves, how a type is written and how a comment is laid out is the
//! emitter's, through [`Dialect`].
//!
//! Names come out as plain identifiers in lower or upper camel case that start with a letter. Two things
//! that would take one name are an error naming both, never a silent pick: a facade that compiled with one
//! of two commands missing would be a wrong API, not a failed build.

use crate::model::Command;
use crate::types::KeyType;
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};

/// The parameter every method ends with: keys the type rules did not type, passed through as they are.
pub(crate) const EXTRA: &str = "extra";

/// What documents [`EXTRA`], a method's reply and its failure, the same for every method.
pub(crate) const EXTRA_DOC: &str = "Keys the typed parameters do not cover, passed through as they are; the engine checks every key.";
pub(crate) const RETURNS_DOC: &str = "The engine's reply.";
pub(crate) const THROWS_DOC: &str = "Whatever the engine refuses the call with.";

/// What documents the word an enum key's value carries, and the way to make the value of any word, the
/// same for every enum key: a value is its word, so a word the engine adds later is a value all the same.
pub(crate) const WORD_DOC: &str = "The word the engine takes for this value.";
pub(crate) const FROM_WORD_DOC: &str = "The value of a word, one listed here or one the engine added since; the engine checks it.";

/// What a namespace's type is named: its path in upper camel case and this, `LayerMaskCommands`.
const TYPE_SUFFIX: &str = "Commands";

/// `text` between the quotes of a string literal: a backslash and a double quote always take a
/// backslash, every letter in `extra` takes one as well, and every control letter goes through
/// `control`, which spells it the way its language reads it. The registry writes the text between the
/// quotes — a command id, a key, an enum word — none of which may end the literal early.
pub(crate) fn string_literal(text: &str, extra: &str, control: impl Fn(char) -> String) -> String {
    text.chars()
        .map(|letter| match letter {
            '\\' => "\\\\".to_string(),
            '"' => "\\\"".to_string(),
            letter if extra.contains(letter) => format!("\\{letter}"),
            letter if letter.is_control() => control(letter),
            letter => letter.to_string(),
        })
        .collect()
}

/// The first lines of every facade file: the `@generated` notice the gate's `generated` stage knows a
/// generated file by, naming the subcommand that wrote it (`command`, which each emitter's module owns as
/// the one name `run` answers it to), then where to make a change instead.
pub(crate) fn header(command: &str) -> String {
    format!("{}// Change the generator, which reads EffectCraft's command registry at the pinned revision.\n", crate::generated_notice(command))
}

/// The member every namespace's type of either facade keeps its engine by. A namespace below another is
/// handed out by a property of its parent's type, so one named this would be a second declaration of the
/// name there: [`crate::swift::check`] and [`crate::kotlin::check`] both refuse it.
pub(crate) const ENGINE: &str = "engine";

/// The one thing that differs by language in naming: how a name that the language reserves is written.
pub(crate) trait Dialect {
    /// `name` as an identifier of the language.
    fn escape(&self, name: &str) -> String;
}

/// One command namespace: the type that holds its methods.
pub(crate) struct Namespace {
    /// The namespace as the ids spell it, e.g. `layer.mask`.
    pub(crate) path: String,
    /// The type that holds its methods, in upper camel case: `LayerMaskCommands`.
    pub(crate) type_name: String,
    /// What the namespace is called where its parent holds it, in lower camel case: `mask`.
    pub(crate) name: String,
    /// The `type_name` of the namespace that holds this one, `None` for a namespace the engine itself holds.
    pub(crate) parent: Option<String>,
    /// Its documentation, one sentence.
    pub(crate) doc: String,
    pub(crate) methods: Vec<Method>,
    /// The enums of its methods' keys.
    pub(crate) enums: Vec<EnumType>,
}

/// One command.
pub(crate) struct Method {
    /// The command id.
    pub(crate) id: String,
    /// The method's name, in lower camel case: `motionBlur`.
    pub(crate) name: String,
    /// The first sentence of its documentation: the registry's label.
    pub(crate) summary: String,
    /// The rest of its documentation: the registry's `params` text, on one line.
    pub(crate) detail: String,
    pub(crate) params: Vec<Param>,
}

/// One key of a command.
pub(crate) struct Param {
    /// The key as the engine spells it.
    pub(crate) key: String,
    /// The parameter's name, in lower camel case.
    pub(crate) name: String,
    pub(crate) required: bool,
    pub(crate) key_type: KeyType,
    /// The name of the key's enum type, when the key is an enum.
    pub(crate) enum_name: Option<String>,
    /// Its documentation: the key and its piece of the `params` text, on one line.
    pub(crate) doc: String,
}

/// The type of an enum key: a name for its values.
pub(crate) struct EnumType {
    pub(crate) name: String,
    /// The command whose key it types.
    pub(crate) id: String,
    /// The key it types.
    pub(crate) key: String,
    /// Its documentation, one sentence.
    pub(crate) doc: String,
    pub(crate) cases: Vec<EnumCase>,
}

/// One value of an enum key.
pub(crate) struct EnumCase {
    /// The case's name, in lower camel case.
    pub(crate) name: String,
    /// The value as the engine spells it.
    pub(crate) value: String,
    /// Its documentation, one sentence.
    pub(crate) doc: String,
}

/// Group the commands of `commands` an app can call into namespaces, every namespace and its methods in
/// the order of their ids; a command no app can call is no method (module docs). A namespace holds the
/// namespaces below it too, so a namespace with commands of its own and one without are both here, and
/// one left with no method of its own or below it is none.
pub(crate) fn plan(commands: &[Command]) -> Result<Vec<Namespace>, String> {
    let mut by_path: BTreeMap<Vec<&str>, Namespace> = BTreeMap::new();
    for command in commands.iter().filter(|command| command.callable()) {
        let segments: Vec<&str> = command.id.split('.').collect();
        let Some((last, namespace)) = segments.split_last().filter(|(_, namespace)| !namespace.is_empty()) else {
            return Err(format!("`{}` has no namespace: a command id is a namespace and a name joined by `.`", command.id));
        };
        for depth in 1..=namespace.len() {
            let path = namespace.iter().take(depth).copied().collect::<Vec<_>>();
            if let Entry::Vacant(vacant) = by_path.entry(path) {
                let entry = namespace_of(vacant.key())?;
                vacant.insert(entry);
            }
        }
        let owner = by_path.get_mut(namespace).ok_or_else(|| format!("no namespace for `{}`", command.id))?;
        let method = method_of(command, last, &owner.name)?;
        owner.methods.push(method);
    }
    let mut namespaces: Vec<Namespace> = by_path.into_values().collect();
    for namespace in &mut namespaces {
        namespace.enums = enums_of(&namespace.methods)?;
    }
    check_names(&namespaces)?;
    Ok(namespaces)
}

fn namespace_of(path: &[&str]) -> Result<Namespace, String> {
    let joined = path.join(".");
    let Some((_, holders)) = path.split_last() else { return Err("a namespace needs a name".to_string()) };
    let type_name =
        |segments: &[&str]| segments.iter().map(|segment| camel(segment, true)).collect::<Result<String, _>>().map(|stem| format!("{stem}{TYPE_SUFFIX}"));
    let parent = if holders.is_empty() { None } else { Some(type_name(holders)?) };
    // Each segment's name is put in front of the next where that needs one, so `view.3d.9patch` is
    // `view3d9patch` and not a second name starting with a digit.
    let name =
        path.iter().try_fold(String::new(), |holder, segment| member_name(segment, &holder)).map_err(|error| format!("namespace `{joined}`: {error}"))?;
    Ok(Namespace {
        type_name: type_name(path)?,
        name,
        parent,
        doc: format!("The `{}` commands.", one_line(&joined)),
        path: joined,
        methods: Vec::new(),
        enums: Vec::new(),
    })
}

fn method_of(command: &Command, last: &str, namespace: &str) -> Result<Method, String> {
    // Read whole, so a field the surface gains stops this build until the facade says what it does with
    // it, as the diff must; `journal` changes nothing a call is typed by, and the refusal marks were read
    // by `plan`, which makes no method of a command they keep from apps.
    let Command { id, label, params, journal: _, refused_under_confinement: _, withheld_by_plugin: _, keys } = command;
    let name = member_name(last, namespace).map_err(|error| format!("command `{id}`: {error}"))?;
    let typed = keys.iter().map(|key| param_of(id, &name, key)).collect::<Result<Vec<_>, _>>()?;
    let mut seen = BTreeSet::new();
    for param in typed.iter().map(|param| param.name.as_str()).chain([EXTRA]) {
        if !seen.insert(param) {
            return Err(format!("command `{id}` has two parameters named `{param}`"));
        }
    }
    Ok(Method {
        id: id.clone(),
        name,
        summary: format!("{}.", one_line(label).trim_end_matches('.')),
        detail: format!("The engine describes its parameters as: {}", one_line(params)),
        params: typed,
    })
}

fn param_of(command: &str, method: &str, key: &crate::model::Key) -> Result<Param, String> {
    // Read whole, as `method_of` reads its command.
    let crate::model::Key { name: key_name, required, key_type, values, doc } = key;
    let name = member_name(key_name, method).map_err(|error| format!("command `{command}` key `{key_name}`: {error}"))?;
    // The surface file holds a key's type as its name and an `enum`'s words beside it; the type comes
    // back from that one spelling, and a file that spells it wrong is refused here by name, not guessed.
    let key_type = KeyType::named(key_type, values.as_deref()).map_err(|why| format!("command `{command}` key `{key_name}`: {why}"))?;
    let enum_name = matches!(key_type, KeyType::Enum(_)).then(|| format!("{}{}", upper_initial(method), upper_initial(&name)));
    let doc = format!("The key `{}`: {}", one_line(key_name), one_line(doc));
    Ok(Param { key: key_name.clone(), name, required: *required, key_type, enum_name, doc })
}

/// The enum types a namespace's methods need, one for each enum key.
fn enums_of(methods: &[Method]) -> Result<Vec<EnumType>, String> {
    let mut enums = Vec::new();
    for method in methods {
        for param in &method.params {
            let (KeyType::Enum(values), Some(name)) = (&param.key_type, &param.enum_name) else { continue };
            let cases = values
                .iter()
                .map(|value| {
                    member_name(value, &param.name)
                        .map(|case| EnumCase { name: case, value: value.clone(), doc: format!("The value `{}`.", one_line(value)) })
                        .map_err(|error| format!("command `{}` key `{}`: {error}", method.id, param.key))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let mut seen = BTreeSet::new();
            if let Some(case) = cases.iter().find(|case| !seen.insert(case.name.as_str())) {
                return Err(format!("command `{}` key `{}`: two values are named `{}`", method.id, param.key, case.name));
            }
            let doc = format!("The values the engine accepts for the key `{}` of `{}`.", one_line(&param.key), one_line(&method.id));
            enums.push(EnumType { name: name.clone(), id: method.id.clone(), key: param.key.clone(), doc, cases });
        }
    }
    Ok(enums)
}

/// What no two may share: the namespaces' types (and their files, which a disk that ignores case would
/// merge), and, in one namespace, two methods or two enums. A method and a namespace below its own may
/// share a name, as `edit.history` and the namespace of `edit.history.goto` do: Swift and Kotlin both tell
/// a property from a method by the call, so `edit.history(…)` and `edit.history.goto(…)` both keep the
/// spelling of their ids. Two namespaces below one parent may not take one name either: that name is the
/// property their parent holds them by, and the digit rule folds two ids the engine keeps apart onto one,
/// `view.3d` and `view.view3d` both wanting `view3d` in `view`.
fn check_names(namespaces: &[Namespace]) -> Result<(), String> {
    let mut types = BTreeMap::new();
    for namespace in namespaces {
        if let Some(other) = types.insert(namespace.type_name.to_lowercase(), &namespace.path) {
            return Err(format!("namespaces `{other}` and `{}` take the same type name `{}`", namespace.path, namespace.type_name));
        }
    }
    for namespace in namespaces {
        let methods: Vec<(&str, String)> = namespace.methods.iter().map(|method| (method.name.as_str(), format!("command `{}`", method.id))).collect();
        let enums = namespace.enums.iter().map(|each| (each.name.as_str(), format!("the enum of `{}` key `{}`", each.id, each.key))).collect();
        for members in [methods, enums] {
            first_clash(members).map_or(Ok(()), |(name, one, other)| Err(format!("{one} and {other} take the same name `{name}` in `{}`", namespace.path)))?;
        }
    }
    // Where each name goes is the parent's type by which `Namespace::parent` is kept, so groups can be
    // weighed one parent at a time and the error can name the parent by its id.
    let paths: BTreeMap<&str, &str> = namespaces.iter().map(|namespace| (namespace.type_name.as_str(), namespace.path.as_str())).collect();
    let mut children: BTreeMap<Option<&str>, Vec<(&str, String)>> = BTreeMap::new();
    for namespace in namespaces {
        children.entry(namespace.parent.as_deref()).or_default().push((namespace.name.as_str(), format!("namespace `{}`", namespace.path)));
    }
    for (parent, members) in children {
        let holder = parent.map_or_else(|| "the engine itself".to_string(), |type_name| paths.get(type_name).copied().unwrap_or(type_name).to_string());
        first_clash(members).map_or(Ok(()), |(name, one, other)| Err(format!("{one} and {other} take the same name `{name}` in `{holder}`")))?;
    }
    Ok(())
}

/// The first name two of `members` share, with what each of the two is. An emitter that renames a name it
/// was given weighs the renamed names with this too, so a clash Swift or Kotlin makes is an error naming
/// both, never a second declaration of one name.
pub(crate) fn first_clash(members: Vec<(&str, String)>) -> Option<(&str, String, String)> {
    let mut seen: BTreeMap<&str, String> = BTreeMap::new();
    for (name, what) in members {
        if let Some(other) = seen.insert(name, what.clone()) {
            return Some((name, other, what));
        }
    }
    None
}

/// `word` as a member's name: lower camel case, starting with a letter. Neither Swift nor Kotlin lets a
/// name start with a digit, so a word that does takes the name of what holds it in front, as Swift's
/// importer names the C constant `MTLTextureType2D` `type2D`: the namespace `view.3d` is `view3d` in
/// `view`. A leading `_` would do it too, but Swift reads that as a name kept out of the public API: its
/// documentation leaves the name out.
fn member_name(word: &str, holder: &str) -> Result<String, String> {
    let name = camel(word, false)?;
    if !starts_with_digit(&name) {
        return Ok(name);
    }
    let held = camel(&format!("{holder}.{word}"), false)?;
    if starts_with_digit(&held) { Err(format!("`{word}` starts with a digit and nothing that holds it has a name to put in front")) } else { Ok(held) }
}

fn starts_with_digit(name: &str) -> bool {
    name.starts_with(|letter: char| letter.is_ascii_digit())
}

/// Text on one line: runs of white space, line breaks included, become one space. Every piece of registry
/// text a documentation sentence carries goes through it — an id, a key, a word as well as a label or a
/// description — since a Swift doc comment is one `///` line, which a line break would end, leaving the
/// rest of the sentence as code.
pub(crate) fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `word` as an identifier in camel case. A word is cut where it holds a character that is no letter or
/// digit, the parts joined with their first letter in upper case. The first part starts in lower case
/// unless `upper_first`; a run of capitals before it is lowered as a whole (`URL` is `url`, `RGBMode` is
/// `rgbMode`), since the run is an acronym, not words. The surface's names are letters and digits
/// already, so cutting is for a name the engine may one day write otherwise.
fn camel(word: &str, upper_first: bool) -> Result<String, String> {
    let parts: Vec<&str> = word.split(|letter: char| !letter.is_ascii_alphanumeric()).filter(|part| !part.is_empty()).collect();
    let Some((first, rest)) = parts.split_first() else { return Err(format!("`{word}` has no letter or digit to name anything by")) };
    let head = if upper_first { upper_initial(first) } else { lower_initial(first) };
    Ok(std::iter::once(head).chain(rest.iter().map(|part| upper_initial(part))).collect())
}

fn upper_initial(part: &str) -> String {
    let mut letters = part.chars();
    letters.next().map(|first| first.to_ascii_uppercase().to_string() + letters.as_str()).unwrap_or_default()
}

/// The leading capitals of `part` in lower case, but the last of them when a lower-case letter follows it,
/// since that capital starts the next word.
fn lower_initial(part: &str) -> String {
    let capitals = part.chars().take_while(char::is_ascii_uppercase).count();
    let word_follows = part.chars().nth(capitals).is_some_and(|letter| letter.is_ascii_lowercase());
    let lowered = if word_follows { capitals.saturating_sub(1).max(1) } else { capitals };
    part.chars().enumerate().map(|(index, letter)| if index < lowered { letter.to_ascii_lowercase() } else { letter }).collect()
}

/// The type a key is passed as, in whichever language asks. Every word here names a type the transport
/// or the bindings already name alike in Swift and in Kotlin — `Double`, `String`, `Vec2`, `Vec3`,
/// `CraftColor`, `ItemReference`, `JSONValue` — and an enum key takes its enum type's own name, which
/// is also the same in both. Only the boolean is spelled per language, so that word alone is the
/// argument; a facade for a third language adds its word here rather than copying this table.
pub(crate) fn type_name(param: &Param, bool_name: &str) -> String {
    match &param.key_type {
        KeyType::Bool => bool_name.to_string(),
        KeyType::Number => "Double".to_string(),
        KeyType::Str => "String".to_string(),
        KeyType::Enum(_) => param.enum_name.clone().unwrap_or_else(|| "String".to_string()),
        KeyType::Vec2 => "Vec2".to_string(),
        KeyType::Vec3 => "Vec3".to_string(),
        KeyType::Color => "CraftColor".to_string(),
        KeyType::Ref => "ItemReference".to_string(),
        KeyType::Json => "JSONValue".to_string(),
    }
}

/// The text of the emitted file the test asks for, chosen the way that test matches names. Both
/// emitters' tests look up a file by name and hand back its text; only the name match differs, so that
/// difference alone is the argument.
#[cfg(test)]
pub(crate) fn text_in(mut emitted: impl Iterator<Item = (String, String)>, name: &str, keeps: impl Fn(&str, &str) -> bool) -> String {
    emitted.find(|(file, _)| keeps(file, name)).map(|(_, text)| text).unwrap_or_default()
}

/// Test-only: the plan's refusal of a command set, read through one emitter's own checks.
#[cfg(test)]
pub(crate) fn refusal(check: fn(&[Namespace]) -> Result<(), String>, commands: &[Command]) -> String {
    check(&plan(commands).unwrap()).err().unwrap_or_default()
}

/// Test-only: a command set planned and rendered by one emitter, in one step.
#[cfg(test)]
pub(crate) fn plan_emitted(render: fn(&[Namespace]) -> Vec<(String, String)>, commands: &[Command]) -> Vec<(String, String)> {
    render(&plan(commands).unwrap())
}

/// Test-only: a facade refuses an enum key whose type name would take a name the facade itself writes
/// by hand, and the names it weighs (`named_types`) hold every type its table (`type_name`) can hand
/// back. The refused names and the refusals come from this module's planning, so the proof runs once
/// and each emitter's test calls it through its own checks, then adds the names only its own language
/// writes.
#[cfg(test)]
pub(crate) fn assert_facade_names_are_refused(check: fn(&[Namespace]) -> Result<(), String>, named_types: &[&str], type_name: fn(&Param) -> String) {
    use crate::model::Key;
    for key_type in [KeyType::Bool, KeyType::Number, KeyType::Str, KeyType::Vec2, KeyType::Vec3, KeyType::Color, KeyType::Ref, KeyType::Json] {
        let param = plan(&[Command::of("a.b", "B", vec![Key::of("k", false, key_type)])])
            .unwrap()
            .pop()
            .and_then(|namespace| namespace.methods.into_iter().next())
            .and_then(|method| method.params.into_iter().next())
            .unwrap();
        assert!(named_types.contains(&type_name(&param).as_str()), "{} is named by the facade but not weighed", type_name(&param));
    }
    let kind = KeyType::Enum(vec!["layer".to_string(), "comp".to_string()]);
    let error = refusal(check, &[Command::of("effect.item", "Item", vec![Key::of("reference", false, kind.clone())])]);
    assert!(error.contains("command `effect.item` key `reference`") && error.contains("`ItemReference`"), "{error}");
    let error = refusal(check, &[Command::of("view.effects", "E", vec![Key::of("engine", false, kind.clone())])]);
    assert!(error.contains("`EffectsEngine`"), "{error}");
    let error = refusal(check, &[Command::of("a.jSON", "J", vec![Key::of("value", false, kind.clone())])]);
    assert!(error.contains("`JSONValue`"), "{error}");
    let error = refusal(check, &[Command::of("a.craft", "C", vec![Key::of("color", false, kind)])]);
    assert!(error.contains("`CraftColor`"), "{error}");
}

/// Test-only: the pin's promise proved against a renderer: one file per namespace, one call per
/// command an app can call (`call_marker` is the language's spelling of a method), and each such
/// command's id written on exactly one id line, which both layouts indent alike, while the id of a
/// command no app can call is written on none. The rendered files come back for a test to weigh
/// whatever else its language bans.
#[cfg(test)]
pub(crate) fn assert_pin_emitted_once(render: fn(&[Namespace]) -> Vec<(String, String)>, call_marker: &str) -> Vec<(String, String)> {
    let commands = crate::surface::commands().unwrap();
    let namespaces = plan(&commands).unwrap();
    let emitted = render(&namespaces);
    assert_eq!(emitted.len(), namespaces.len());
    let methods: usize = emitted.iter().map(|(_, text)| text.matches(call_marker).count()).sum();
    assert_eq!(methods, callable_at_the_pin());
    for command in &commands {
        let needle = format!("            \"{}\",\n", command.id);
        let lines = emitted.iter().map(|(_, text)| text.matches(&needle).count()).sum::<usize>();
        assert_eq!(lines, usize::from(command.callable()), "{} is callable: {}", command.id, command.callable());
    }
    emitted
}

/// Test-only: how many commands of the pin an app can call, which is how many methods each facade has.
/// Some command of the pin is not one, so a facade that emitted every command would fail on the count.
#[cfg(test)]
pub(crate) fn callable_at_the_pin() -> usize {
    let commands = crate::surface::commands().unwrap();
    let callable = commands.iter().filter(|command| command.callable()).count();
    assert!(callable < commands.len(), "every command of the pin is callable, so the count proves nothing");
    callable
}

/// Test-only: a command whose id, key and word each carry a line break where the registry might write
/// one, for the emitters' tests that a documentation sentence stays one line.
#[cfg(test)]
pub(crate) fn broken_lines() -> Vec<Command> {
    use crate::model::Key;
    let mut command =
        Command::of("view\nleft.show\rall", "Show\nAll", vec![Key::of("k\ny", false, KeyType::Enum(vec!["on".to_string(), "of\rf".to_string()]))]);
    command.params = "{k\ny?:\ron|of\rf}".to_string();
    vec![command]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Key;

    #[test]
    fn no_documentation_sentence_holds_a_line_break_from_the_registry() {
        let namespaces = plan(&broken_lines()).unwrap();
        let mut sentences: Vec<&str> = Vec::new();
        for namespace in &namespaces {
            sentences.push(&namespace.doc);
            for method in &namespace.methods {
                sentences.extend([method.summary.as_str(), method.detail.as_str()]);
                sentences.extend(method.params.iter().map(|param| param.doc.as_str()));
            }
            for each in &namespace.enums {
                sentences.push(&each.doc);
                sentences.extend(each.cases.iter().map(|case| case.doc.as_str()));
            }
        }
        assert_eq!(sentences.len(), 7, "every kind of sentence is read: {sentences:?}");
        for sentence in sentences {
            assert!(!sentence.contains(['\n', '\r']), "{sentence:?}");
        }
    }

    #[test]
    fn a_facade_file_opens_with_the_generated_notice_naming_its_command() {
        let header = header("kotlin");
        assert!(header.starts_with(&crate::generated_notice("kotlin")), "{header}");
        let lines: Vec<&str> = header.lines().collect();
        assert_eq!(lines.len(), 2, "{header}");
        assert!(header.ends_with(".\n"), "{header}");
    }

    fn key(name: &str, key_type: KeyType) -> Key {
        Key::of(name, false, key_type)
    }

    fn command(id: &str, keys: Vec<Key>) -> Command {
        Command::of(id, "Label", keys)
    }

    fn names<'a>(methods: impl IntoIterator<Item = &'a Method>) -> Vec<&'a str> {
        methods.into_iter().map(|method| method.name.as_str()).collect()
    }

    #[test]
    fn a_name_is_camel_case_whatever_its_capitals() {
        let named = |word, upper| camel(word, upper).unwrap();
        assert_eq!(named("newText", false), "newText");
        assert_eq!(named("newText", true), "NewText");
        assert_eq!(named("URL", false), "url");
        assert_eq!(named("RGBA", false), "rgba");
        assert_eq!(named("RGBMode", false), "rgbMode");
        assert_eq!(named("Add", false), "add");
        assert_eq!(named("F10", false), "f10");
        assert_eq!(named("sameAsLayer", false), "sameAsLayer");
        assert_eq!(named("renderQueue", true), "RenderQueue");
        assert_eq!(named("3d", true), "3d");
        assert_eq!(named("motion-blur", false), "motionBlur");
        assert_eq!(named("a", false), "a");
        assert_eq!(named("A", false), "a");
        assert!(camel("", false).is_err());
        assert!(camel("--", true).is_err());
    }

    #[test]
    fn commands_are_grouped_into_the_namespaces_their_ids_name() {
        let commands = [
            command("layer.mask.motionBlur", vec![]),
            command("layer.newText", vec![key("text", KeyType::Str)]),
            command("layer.mask.add", vec![]),
            command("time.forward10", vec![]),
        ];
        let namespaces = plan(&commands).unwrap();
        let paths: Vec<&str> = namespaces.iter().map(|namespace| namespace.path.as_str()).collect();
        assert_eq!(paths, ["layer", "layer.mask", "time"]);
        let mask = namespaces.get(1).unwrap();
        assert_eq!((mask.type_name.as_str(), mask.name.as_str(), mask.parent.as_deref()), ("LayerMaskCommands", "mask", Some("LayerCommands")));
        assert_eq!(names(&mask.methods), ["motionBlur", "add"], "methods keep the order they came in");
        assert_eq!(namespaces.first().unwrap().parent, None);
        let text = namespaces.first().unwrap().methods.first().unwrap();
        assert_eq!((text.id.as_str(), text.name.as_str()), ("layer.newText", "newText"));
    }

    /// A command the surface marks refused under confinement, withheld by the plugin, or both, is no
    /// method of either facade, and a namespace that holds nothing else, itself or below it, is none.
    #[test]
    fn a_command_no_app_can_call_is_no_method() {
        let refused = Command { refused_under_confinement: true, ..command("layer.run", vec![]) };
        let withheld = Command { withheld_by_plugin: true, ..command("layer.render", vec![]) };
        let both = Command { refused_under_confinement: true, withheld_by_plugin: true, ..command("layer.both", vec![]) };
        let alone = Command { withheld_by_plugin: true, ..command("script.inner.run", vec![]) };
        let namespaces = plan(&[refused, command("layer.newText", vec![]), withheld, both, alone]).unwrap();
        let paths: Vec<&str> = namespaces.iter().map(|namespace| namespace.path.as_str()).collect();
        assert_eq!(paths, ["layer"]);
        assert_eq!(names(namespaces.iter().flat_map(|namespace| &namespace.methods)), ["newText"]);
    }

    #[test]
    fn a_namespace_with_no_command_of_its_own_is_still_a_namespace() {
        let namespaces = plan(&[command("outer.inner.run", vec![])]).unwrap();
        let paths: Vec<&str> = namespaces.iter().map(|namespace| namespace.path.as_str()).collect();
        assert_eq!(paths, ["outer", "outer.inner"]);
        assert!(namespaces.first().unwrap().methods.is_empty());
    }

    #[test]
    fn keys_become_parameters_in_order_and_an_enum_key_gets_a_type_named_after_both() {
        let mode = key("mode", KeyType::Enum(vec!["sameAsLayer".to_string(), "None".to_string()]));
        let namespaces = plan(&[command("layer.mask.motionBlur", vec![key("layer", KeyType::Ref), mode])]).unwrap();
        let method = namespaces.last().unwrap().methods.first().unwrap();
        assert_eq!(method.params.iter().map(|param| (param.key.as_str(), param.name.as_str())).collect::<Vec<_>>(), [("layer", "layer"), ("mode", "mode")]);
        let mode = method.params.get(1).unwrap();
        assert_eq!(mode.enum_name.as_deref(), Some("MotionBlurMode"));
        let each = namespaces.last().unwrap().enums.first().unwrap();
        assert_eq!(each.name, "MotionBlurMode");
        assert_eq!(
            each.cases.iter().map(|case| (case.name.as_str(), case.value.as_str())).collect::<Vec<_>>(),
            [("sameAsLayer", "sameAsLayer"), ("none", "None")]
        );
    }

    #[test]
    fn a_command_and_the_namespace_below_it_both_keep_the_name_their_ids_give() {
        let namespaces = plan(&[command("edit.history", vec![]), command("edit.history.undo", vec![]), command("edit.copy", vec![])]).unwrap();
        let edit = namespaces.first().unwrap();
        assert_eq!(names(&edit.methods), ["history", "copy"]);
        assert_eq!(namespaces.get(1).unwrap().name, "history");
        assert_eq!(names(&namespaces.get(1).unwrap().methods), ["undo"]);
    }

    #[test]
    fn a_name_that_would_start_with_a_digit_takes_the_name_of_what_holds_it() {
        let keys = vec![key("2up", KeyType::Enum(vec!["4k".to_string(), "hd".to_string()]))];
        let namespaces = plan(&[command("view.3d.front", vec![]), command("view.3d.9patch", keys), command("view.3dTools.run", vec![])]).unwrap();
        let three = namespaces.iter().find(|namespace| namespace.path == "view.3d").unwrap();
        assert_eq!((three.name.as_str(), three.type_name.as_str()), ("view3d", "View3dCommands"));
        assert_eq!(names(&three.methods), ["front", "view3d9patch"]);
        let param = three.methods.get(1).and_then(|method| method.params.first()).unwrap();
        assert_eq!((param.name.as_str(), param.enum_name.as_deref()), ("view3d9patch2up", Some("View3d9patchView3d9patch2up")));
        let cases = three.enums.first().map(|each| each.cases.iter().map(|case| case.name.as_str()).collect::<Vec<_>>()).unwrap();
        assert_eq!(cases, ["view3d9patch2up4k", "hd"]);
        assert_eq!(namespaces.iter().find(|namespace| namespace.path == "view.3dTools").unwrap().name, "view3dTools");
        let error = plan(&[command("3d.front", vec![])]).err().unwrap_or_default();
        assert!(error.contains("namespace `3d`") && error.contains("starts with a digit"), "{error}");
    }

    #[test]
    fn the_documentation_is_written_once_for_every_language() {
        let mode = Key::typed("mode", true, &KeyType::Enum(vec!["a".to_string()]), "mode:\n  a|b");
        let label = Command { params: "{mode,\n other}".to_string(), ..Command::of("layer.blur", "Blur  Layer...", vec![mode]) };
        let namespaces = plan(&[label]).unwrap();
        let layer = namespaces.first().unwrap();
        assert_eq!(layer.doc, "The `layer` commands.");
        let method = layer.methods.first().unwrap();
        assert_eq!(method.summary, "Blur Layer.");
        assert_eq!(method.detail, "The engine describes its parameters as: {mode, other}");
        assert_eq!(method.params.first().unwrap().doc, "The key `mode`: mode: a|b");
        let each = layer.enums.first().unwrap();
        assert_eq!(each.doc, "The values the engine accepts for the key `mode` of `layer.blur`.");
        assert_eq!(each.cases.first().unwrap().doc, "The value `a`.");
    }

    #[test]
    fn text_is_put_on_one_line() {
        assert_eq!(one_line("  a\n\t b   c\r\n"), "a b c");
    }

    #[test]
    fn a_key_whose_type_the_surface_spells_wrong_is_refused_naming_the_command_and_the_key() {
        let refusal = |bad: Key| plan(&[command("layer.blur", vec![bad])]).err().unwrap_or_default();
        for (bad, why) in [
            (Key::raw("mode", false, "integer", None), "no type is named `integer`"),
            (Key::raw("mode", false, "enum", None), "an `enum` names its words"),
            (Key::raw("mode", false, "bool", Some(&["on"])), "only an `enum` has words"),
        ] {
            let error = refusal(bad);
            assert!(error.starts_with("command `layer.blur` key `mode`: ") && error.contains(why), "{error}");
        }
    }

    #[test]
    fn what_would_take_one_name_is_refused_naming_both() {
        let error = |commands: &[Command]| plan(commands).err().unwrap_or_default();
        assert!(error(&[command("lonely", vec![])]).contains("`lonely` has no namespace"));
        assert!(error(&[command(".x", vec![])]).contains("no letter or digit"));
        assert!(error(&[command("a.b.", vec![])]).contains("no letter or digit"));
        let twice = error(&[command("a.x-y", vec![]), command("a.xY", vec![])]);
        assert!(twice.contains("`a.x-y`") && twice.contains("`a.xY`") && twice.contains("`xY`"), "{twice}");
        // Two commands of one id are two methods of one name, nowhere apart to stand: this refusal,
        // not a second id check, is what turns a duplicated registry away from a facade.
        assert!(error(&[command("a.b", vec![]), command("a.b", vec![])]).contains("command `a.b` and command `a.b` take the same name `b` in `a`"));
        assert!(error(&[command("a.b", vec![key("x", KeyType::Json), key("X", KeyType::Json)])]).contains("two parameters named `x`"));
        assert!(error(&[command("a.b", vec![key("extra", KeyType::Json)])]).contains("two parameters named `extra`"));
        let cases = KeyType::Enum(vec!["Add".to_string(), "add".to_string()]);
        assert!(error(&[command("a.b", vec![key("m", cases)])]).contains("two values are named `add`"));
        let enums = error(&[
            command("a.fooBar", vec![key("baz", KeyType::Enum(vec!["x".to_string()]))]),
            command("a.foo", vec![key("barBaz", KeyType::Enum(vec!["y".to_string()]))]),
        ]);
        assert!(enums.contains("`a.fooBar` key `baz`") && enums.contains("`a.foo` key `barBaz`") && enums.contains("`FooBarBaz`"), "{enums}");
        let types = error(&[command("ab.c", vec![]), command("aB.d", vec![])]);
        assert!(types.contains("`ab`") && types.contains("`aB`"), "{types}");
        let property = error(&[command("view.3d.front", vec![]), command("view.view3d.back", vec![])]);
        assert!(
            property.contains("`view.3d`") && property.contains("`view.view3d`") && property.contains("`view3d`") && property.contains("`view`"),
            "{property}"
        );
    }

    #[test]
    fn every_command_at_the_pin_is_planned_once() {
        let commands = crate::surface::commands().unwrap();
        let namespaces = plan(&commands).unwrap();
        let methods: usize = namespaces.iter().map(|namespace| namespace.methods.len()).sum();
        assert_eq!(methods, callable_at_the_pin());
        assert!(namespaces.iter().all(|namespace| namespace.parent.as_ref().is_none_or(|parent| namespaces.iter().any(|other| &other.type_name == parent))));
    }
}
