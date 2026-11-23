//! The type rules: how one key of a command's `params` text becomes a type.
//!
//! The registry gives a key's type only as prose, in the piece of the `params` text that describes it
//! (`preserveFrameRate?: bool`, `position? [x,y]`), so a type is read from that prose or not at all.
//! Every rule below accepts a piece only when the prose says the type outright and says nothing more: a
//! value that carries a further alternative (`bool|null`, `[x,y]|null`, `[r,g,b]|#hex|false`, or a note
//! that offers one, `(or display name)`) or text the rules do not know stays `Json`, because the engine
//! accepts that key's value whatever it is and a typed API narrower than the engine would reject a call
//! the engine accepts. A rule that cannot be sure therefore errs towards `Json`, never towards a narrower
//! type. That holds for the words an enum lists too: the prose writes a value and a stand-in for a kind
//! of value the same way (`timeBase?: layer|comp` names two values, `output?: path|template` a file path
//! or a template's name), so a word the rules know as a stand-in keeps the key `Json`, even where another
//! command lists the same word as a value.

/// The type of one key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum KeyType {
    Bool,
    Number,
    Str,
    /// One of these words.
    Enum(Vec<String>),
    /// `[x,y]`.
    Vec2,
    /// `[x,y,z]`.
    Vec3,
    /// `[r,g,b]` or `#hex`.
    Color,
    /// An item by id, by name or by index.
    Ref,
    /// Anything the rules do not type: the value passes through as it is.
    Json,
}

impl KeyType {
    /// The name the surface file gives the type (the `type` enum of its schema).
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::Number => "number",
            Self::Str => "string",
            Self::Enum(_) => "enum",
            Self::Vec2 => "vec2",
            Self::Vec3 => "vec3",
            Self::Color => "color",
            Self::Ref => "ref",
            Self::Json => "json",
        }
    }

    /// Every type, an `enum` with no words of its own: what [`Self::named`] reads a name back against, so
    /// each name is spelt once, in [`Self::name`], and what the facade model lists the SDK's types over.
    pub(crate) const EVERY: [KeyType; 9] =
        [Self::Bool, Self::Number, Self::Str, Self::Enum(Vec::new()), Self::Vec2, Self::Vec3, Self::Color, Self::Ref, Self::Json];

    /// The type a surface file names `name`, with `values` for its words when it gives any: [`Self::name`]
    /// read back. A name no type bears, words beside any type but an `enum`, or an `enum` without its
    /// words is a surface malformed just where a typed reader reads its type: the reason says which, so
    /// the reader can refuse it by name rather than guess.
    pub(crate) fn named(name: &str, values: Option<&[String]>) -> Result<KeyType, String> {
        let Some(found) = Self::EVERY.into_iter().find(|each| each.name() == name) else { return Err(format!("no type is named `{name}`")) };
        match (found, values) {
            (Self::Enum(_), Some(words)) => Ok(Self::Enum(words.to_vec())),
            (Self::Enum(_), None) => Err("an `enum` names its words, and this one names none".to_string()),
            (_, Some(_)) => Err(format!("only an `enum` has words, but its type is `{name}`")),
            (plain, None) => Ok(plain),
        }
    }
}

/// The words that name a type, and the type each names.
const TYPE_WORDS: &[(&str, KeyType)] = &[("bool", KeyType::Bool), ("string", KeyType::Str)];

/// Words that say a value is a number. `n` is how the prose writes "a number" (`steps?: n`).
const NUMBER_WORDS: &[&str] = &["number", "integer", "int", "float", "n"];

/// Units whose mention, bare or in parentheses, says a value is a number.
const UNITS: &[&str] = &["s", "ms", "px", "fps", "mm", "%", "\u{b0}", "deg"];

/// What a reference to an item is written as; two or more of them, alternatives, make a `Ref`.
const REF_WORDS: &[&str] = &["id", "uid", "name", "index", "#n"];

/// JSON's literals: `bool|null` lists a boolean and a null, not two words.
const LITERALS: &[&str] = &["null", "true", "false"];

/// Words the prose writes for a kind of value rather than a value, where an enum would otherwise be read:
/// a file path, a template's name, a timecode, a frame number, a scale factor
/// (`resolution?: full|half|third|quarter|scale` takes a number for `scale`). An enum holding one would
/// refuse every path, name or number the engine takes for it. A stand-in the pinned prose starts to use
/// shows as a new enum in the surface's diff when the pin moves, which is where this list grows.
const STAND_INS: &[&str] = &["path", "template", "timecode", "frame", "scale"];

/// A colour's channels in brackets, as [`letters`] reads them. A colour is these and [`HEX`] as the two
/// alternatives, in either order.
const RGB_CHANNELS: &str = "r,g,b";
/// A colour written as a hexadecimal string.
const HEX: &str = "#hex";

/// Type the key `key` from `piece`, the part of the `params` text that describes it. A piece that names
/// several keys (`time? (s) | frame?`) says nothing about which of them a word describes, and none is
/// typed: the first key is followed by the `|` that adds the next, which no rule accepts, and the others
/// are not what the piece starts with.
pub(crate) fn type_key(key: &str, piece: &str) -> KeyType {
    let Some(rest) = piece.trim().strip_prefix(key) else { return KeyType::Json };
    let rest = rest.strip_prefix('?').unwrap_or(rest);
    if rest.starts_with(is_word_char) {
        // `key` was only the start of a longer name.
        return KeyType::Json;
    }
    let value = rest.trim_start();
    let value = value.strip_prefix(':').map_or(value, str::trim_start);
    type_value(value)
}

/// Type the value text that starts a key's description.
fn type_value(value: &str) -> KeyType {
    if value.starts_with('[') {
        return type_bracket(value);
    }
    if value.starts_with('(') {
        return type_parenthesis(value);
    }
    if let Some(rest) = value.strip_prefix(HEX) {
        // A colour written hex first: `#hex|[r,g,b]`.
        return match after_bar(rest).and_then(|tail| group(tail, '[', ']')) {
            Some((inner, after)) if letters(inner) == RGB_CHANNELS && is_closed(after) => KeyType::Color,
            _ => KeyType::Json,
        };
    }
    let Some((items, after)) = alternation(value) else { return KeyType::Json };
    if !is_closed(after) {
        return KeyType::Json;
    }
    match items.as_slice() {
        [word] => type_word(word),
        // Two or more words, since `alternation` gives at least one: a reference when each names an item,
        // an enum when each is a value.
        many if many.iter().all(|item| REF_WORDS.contains(item)) => KeyType::Ref,
        many if many.iter().all(|item| is_enum_value(item)) => KeyType::Enum(many.iter().map(|item| (*item).to_string()).collect()),
        _ => KeyType::Json,
    }
}

/// The type a value written as one word says: a type's name, a number word or a unit.
fn type_word(word: &str) -> KeyType {
    match TYPE_WORDS.iter().find(|(name, _)| *name == word) {
        Some((_, key_type)) => key_type.clone(),
        None if NUMBER_WORDS.contains(&word) || UNITS.contains(&word) => KeyType::Number,
        None => KeyType::Json,
    }
}

/// `(s)`, `(px)`, `(16–256)`: a unit or a numeric range in parentheses says a number.
fn type_parenthesis(value: &str) -> KeyType {
    let Some((inner, after)) = group(value, '(', ')') else { return KeyType::Json };
    let inner = inner.trim();
    if is_closed(after) && (UNITS.contains(&inner) || is_numeric_range(inner)) { KeyType::Number } else { KeyType::Json }
}

/// `[x,y]`, `[x,y,z]` and `[r,g,b]|#hex`.
fn type_bracket(value: &str) -> KeyType {
    let Some((inner, after)) = group(value, '[', ']') else { return KeyType::Json };
    match letters(inner).as_str() {
        "x,y" if is_closed(after) => KeyType::Vec2,
        "x,y,z" if is_closed(after) => KeyType::Vec3,
        RGB_CHANNELS => match after_bar(after).and_then(|tail| tail.strip_prefix(HEX)) {
            Some(tail) if is_closed(tail) => KeyType::Color,
            _ => KeyType::Json,
        },
        _ => KeyType::Json,
    }
}

/// The text between brackets without its white space, so `[x, y]` reads as `[x,y]`.
fn letters(inner: &str) -> String {
    inner.chars().filter(|letter| !letter.is_whitespace()).collect()
}

/// What follows the `|` that starts `text`, white space aside; `None` when no `|` starts it.
fn after_bar(text: &str) -> Option<&str> {
    text.trim_start().strip_prefix('|').map(str::trim_start)
}

fn is_word_char(letter: char) -> bool {
    letter.is_ascii_alphanumeric() || letter == '_' || letter == '#'
}

/// A word an enum may list: a name made of letters, digits and underscores that starts with a letter, and
/// none the rules read as something other than a value — a type's name, a number word, a unit, a way to
/// name an item, a JSON literal or a stand-in for a kind of value.
fn is_enum_value(item: &str) -> bool {
    let not_values = [NUMBER_WORDS, UNITS, REF_WORDS, LITERALS, STAND_INS];
    item.starts_with(|letter: char| letter.is_ascii_alphabetic())
        && !item.contains('#')
        && !TYPE_WORDS.iter().any(|(name, _)| *name == item)
        && !not_values.iter().any(|words| words.contains(&item))
}

/// `16–256`, `1-100`, `0.5-2`: two numbers with a dash between.
fn is_numeric_range(text: &str) -> bool {
    text.split_once(['-', '\u{2013}']).is_some_and(|(low, high)| low.trim().parse::<f64>().is_ok() && high.trim().parse::<f64>().is_ok())
}

/// Does nothing but notes in parentheses follow? Any other text could widen what the value accepts, and
/// so does a note that opens with `or`, which offers another value (`(or display name)`).
fn is_closed(after: &str) -> bool {
    let mut rest = after.trim_start();
    while !rest.is_empty() {
        let Some((note, tail)) = group(rest, '(', ')') else { return false };
        if note.split_whitespace().next().is_some_and(|word| word.eq_ignore_ascii_case("or")) {
            return false;
        }
        rest = tail.trim_start();
    }
    true
}

/// The leading words of `text` joined by `|`, with what follows. `None` when an alternative is not a
/// word (`a|[x,y]`), because then the list is not words alone.
fn alternation(text: &str) -> Option<(Vec<&str>, &str)> {
    let mut items = Vec::new();
    let mut rest = text;
    loop {
        let (word, after) = split_where(rest, is_word_char);
        if word.is_empty() {
            return None;
        }
        items.push(word);
        match after_bar(after) {
            Some(next) => rest = next,
            None => return Some((items, after)),
        }
    }
}

/// The text between the opening bracket at the start of `text` and its matching close, and what follows;
/// `None` when `text` does not start with the opening bracket or never closes it.
fn group(text: &str, open: char, close: char) -> Option<(&str, &str)> {
    let body = text.strip_prefix(open)?;
    let mut depth = 1usize;
    for (at, letter) in body.char_indices() {
        if letter == open {
            depth += 1;
        } else if letter == close {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some((body.get(..at)?, body.get(at + close.len_utf8()..)?));
            }
        }
    }
    None
}

/// The longest start of `text` whose letters all satisfy `keep`, and the rest.
fn split_where(text: &str, keep: fn(char) -> bool) -> (&str, &str) {
    let end = text.find(|letter: char| !keep(letter)).unwrap_or(text.len());
    text.split_at_checked(end).unwrap_or(("", text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(piece: &str) -> KeyType {
        let key: String = piece.chars().take_while(|letter| letter.is_ascii_alphanumeric() || *letter == '_').collect();
        type_key(&key, piece)
    }

    fn words(list: &[&str]) -> KeyType {
        KeyType::Enum(list.iter().map(|word| (*word).to_string()).collect())
    }

    #[test]
    fn bool_is_a_bool_and_nothing_wider() {
        assert_eq!(typed("preserveFrameRate?: bool"), KeyType::Bool);
        assert_eq!(typed("average?: bool (5\u{d7}5)"), KeyType::Bool);
        assert_eq!(typed("flag: bool"), KeyType::Bool);
        assert_eq!(typed("flag?: bool|null"), KeyType::Json);
        assert_eq!(typed("flag?: boolean"), KeyType::Json);
        assert_eq!(typed("flag?: bool or null"), KeyType::Json);
        assert_eq!(typed("flag?: bool (a note) (and another)"), KeyType::Bool);
        assert_eq!(typed("flag?: bool (a note) | null"), KeyType::Json, "an alternative after a note still widens the value");
        assert_eq!(typed("flag?: bool (or a name)"), KeyType::Json, "a note that opens with `or` offers another value");
        assert_eq!(typed("flag?: bool (Or a name)"), KeyType::Json);
        assert_eq!(typed("flag?: bool (ordered)"), KeyType::Bool, "only the word `or` opens an alternative");
        assert_eq!(typed("flag?: bool (never closed"), KeyType::Json);
        assert_eq!(typed("flag?: bool text before (a note)"), KeyType::Json, "a note counts only where the text starts with it");
        assert_eq!(typed("flag?: bool (a note) more)"), KeyType::Json, "text after a note is no note, even before a `)`");
    }

    #[test]
    fn string_is_a_string_and_nothing_wider() {
        assert_eq!(typed("text?: string"), KeyType::Str);
        assert_eq!(typed("text?: string|null"), KeyType::Json);
    }

    #[test]
    fn numbers_come_from_a_unit_a_range_or_a_number_word() {
        assert_eq!(typed("duration? (s)"), KeyType::Number);
        assert_eq!(typed("scale? (%)"), KeyType::Number);
        assert_eq!(typed("rotation? (\u{b0})"), KeyType::Number);
        assert_eq!(typed("extent? (px)"), KeyType::Number);
        assert_eq!(typed("size? (16\u{2013}256)"), KeyType::Number);
        assert_eq!(typed("pressure? (1-100)"), KeyType::Number);
        assert_eq!(typed("sensorWidth?: mm (default 36)"), KeyType::Number);
        assert_eq!(typed("rate?: fps"), KeyType::Number);
        assert_eq!(typed("size?: px"), KeyType::Number);
        assert_eq!(typed("count?: number"), KeyType::Number);
        assert_eq!(typed("count?: integer"), KeyType::Number);
        assert_eq!(typed("count?: int"), KeyType::Number);
        assert_eq!(typed("count?: float"), KeyType::Number);
        assert_eq!(typed("steps?: n (undo n steps)"), KeyType::Number);
        assert_eq!(typed("count?: number|auto"), KeyType::Json);
        assert_eq!(typed("rate?: n|auto"), KeyType::Json);
        assert_eq!(typed("time? (layer s by default)"), KeyType::Json);
        assert_eq!(typed("amount? (effect space)"), KeyType::Json);
        assert_eq!(typed("size? (s) or more"), KeyType::Json);
    }

    #[test]
    fn vectors_are_two_or_three_numbers_and_nothing_wider() {
        assert_eq!(typed("position? [x,y]"), KeyType::Vec2);
        assert_eq!(typed("position?: [x, y]"), KeyType::Vec2);
        assert_eq!(typed("position? [x,y] (px)"), KeyType::Vec2);
        assert_eq!(typed("position? [x,y,z]"), KeyType::Vec3);
        assert_eq!(typed("position?: [x, y, z]"), KeyType::Vec3);
        assert_eq!(typed("rect?: [x, y, w, h]"), KeyType::Json);
        assert_eq!(typed("rect?: [x, y] | null"), KeyType::Json);
        assert_eq!(typed("points: [[x,y],\u{2026}]"), KeyType::Json);
    }

    #[test]
    fn a_colour_is_rgb_or_hex_and_nothing_wider() {
        assert_eq!(typed("background? [r,g,b]|#hex"), KeyType::Color);
        assert_eq!(typed("background?: [r, g, b] | #hex"), KeyType::Color);
        assert_eq!(typed("fill?: [r,g,b]|#hex|false"), KeyType::Json);
        assert_eq!(typed("fill?: [r,g,b]"), KeyType::Json);
        assert_eq!(typed("fill?: [r,g,b,a]|#hex"), KeyType::Json);
        assert_eq!(typed("color? #hex|[r,g,b]"), KeyType::Color, "either order");
        assert_eq!(typed("color?: #hex | [r, g, b] (a note)"), KeyType::Color);
        assert_eq!(typed("color?: #hex|[r,g,b]|false"), KeyType::Json);
        assert_eq!(typed("color?: #hex|[r,g,b,a]"), KeyType::Json);
        assert_eq!(typed("color?: #hex"), KeyType::Json);
        assert_eq!(typed("color?: #hexagon|[r,g,b]"), KeyType::Json);
    }

    #[test]
    fn a_reference_is_two_or_more_ways_to_name_an_item() {
        assert_eq!(typed("effect: id|name"), KeyType::Ref);
        assert_eq!(typed("effect?: index|uid|name"), KeyType::Ref);
        assert_eq!(typed("pin: uid | name"), KeyType::Ref);
        assert_eq!(typed("layer: id|name|#n"), KeyType::Ref);
        assert_eq!(typed("layer: id"), KeyType::Json);
        assert_eq!(typed("layers: [id|name|#n]"), KeyType::Json);
        assert_eq!(typed("parent: layer|null"), KeyType::Json);
        assert_eq!(typed("layer: id|name|null"), KeyType::Json);
    }

    #[test]
    fn an_enum_is_two_or_more_bare_words() {
        assert_eq!(typed("renderer?: classic3D|advanced3D"), words(&["classic3D", "advanced3D"]));
        assert_eq!(typed("mode: sameAsLayer|on|off"), words(&["sameAsLayer", "on", "off"]));
        assert_eq!(typed("timeBase?: layer|comp (default layer)"), words(&["layer", "comp"]));
        assert_eq!(typed("kind: range | wiggly | expression"), words(&["range", "wiggly", "expression"]));
        assert_eq!(typed("mode: on"), KeyType::Json);
        assert_eq!(typed("mode: on|off|[x,y]"), KeyType::Json);
        assert_eq!(typed("mode: on|off|null"), KeyType::Json);
        assert_eq!(typed("mode: on|off or something else"), KeyType::Json);
        assert_eq!(typed("mode: 3d|2d"), KeyType::Json);
        assert_eq!(typed("style: dropShadow|satin (or display name)"), KeyType::Json, "the note offers another value");
        assert_eq!(typed("mode: on|off (default on) | null"), KeyType::Json);
    }

    #[test]
    fn a_word_the_rules_read_as_no_value_is_no_enum_value() {
        // One word of each kind, each beside a word that is a value: a type, a number word, a unit, a way to
        // name an item, a JSON literal, and every stand-in for a kind of value.
        for word in
            ["bool", "string", "n", "number", "fps", "px", "s", "id", "uid", "name", "index", "null", "true", "path", "template", "timecode", "frame", "scale"]
        {
            assert_eq!(typed(&format!("mode?: auto|{word}")), KeyType::Json, "{word}");
            assert_eq!(typed(&format!("mode?: {word}|auto")), KeyType::Json, "{word}");
        }
        assert_eq!(typed("output?: path|template"), KeyType::Json);
        assert_eq!(typed("resolution?: full|half|third|quarter|scale"), KeyType::Json);
        assert_eq!(typed("startTimecode?: timecode|frame (Override Start)"), KeyType::Json);
        assert_eq!(typed("mode?: auto|frames"), words(&["auto", "frames"]), "a word is matched whole");
    }

    #[test]
    fn a_key_with_no_type_in_its_piece_is_json() {
        assert_eq!(typed("layer?"), KeyType::Json);
        assert_eq!(typed("attributes as layer.setText"), KeyType::Json);
        assert_eq!(typed("path (.wasm / .wat plug-in, WebAssembly ABI 1 or 2)"), KeyType::Json);
        assert_eq!(type_key("layer", ""), KeyType::Json);
        assert_eq!(type_key("lay", "layer?: bool"), KeyType::Json);
        assert_eq!(type_key("a", "abool"), KeyType::Json, "`a` is only the start of the name `abool`");
    }

    #[test]
    fn a_piece_naming_several_keys_types_none_of_them() {
        assert_eq!(type_key("time", "time? (s)"), KeyType::Number);
        for (key, piece) in [
            ("time", "time? (s) | frame?"),
            ("frame", "time? (s) | frame?"),
            ("effect", "effect? | effects?: [uid|name|index]"),
            ("effects", "effect? | effects?: [uid|name|index]"),
            ("path", "path|prop: bool"),
            ("prop", "path|prop: bool"),
        ] {
            assert_eq!(type_key(key, piece), KeyType::Json, "{key} in {piece}");
        }
    }

    #[test]
    fn every_type_reads_back_from_its_name_and_a_malformed_one_is_refused_saying_why() {
        let words = ["a".to_string(), "b".to_string()];
        for key_type in KeyType::EVERY {
            let back = KeyType::named(key_type.name(), matches!(key_type, KeyType::Enum(_)).then_some(&words[..]));
            assert_eq!(back, Ok(if matches!(key_type, KeyType::Enum(_)) { KeyType::Enum(words.to_vec()) } else { key_type.clone() }));
        }
        assert_eq!(KeyType::named("integer", None), Err("no type is named `integer`".to_string()));
        assert_eq!(KeyType::named("enum", None), Err("an `enum` names its words, and this one names none".to_string()));
        assert_eq!(KeyType::named("bool", Some(&words)), Err("only an `enum` has words, but its type is `bool`".to_string()));
    }

    #[test]
    fn every_type_has_the_name_the_schema_gives_it() {
        // `EVERY` is what a name is read back against, so it is what must hold every name the schema gives.
        let names: Vec<&str> = KeyType::EVERY.iter().map(KeyType::name).collect();
        assert_eq!(names, ["bool", "number", "string", "enum", "vec2", "vec3", "color", "ref", "json"]);
    }
}
