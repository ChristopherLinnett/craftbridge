//! The Kotlin facade: one file for each command namespace, with a typed `suspend fun` for each command,
//! in the shape the plan's API generator design gives:
//!
//! ```kotlin
//! public val EffectsEngine.layer: LayerCommands
//!
//! public class LayerCommands internal constructor(internal val engine: EffectsEngine) {
//!     public suspend fun newText(
//!         vararg namedOnly: NamedArguments,
//!         text: String? = null,
//!         extra: Map<String, JSONValue>? = null,
//!     ): JSONValue
//! }
//! ```
//!
//! Grouping, naming, ordering and the documentation's sentences come from [`crate::facade`]; only how
//! Kotlin spells and lays them out is here. The emitted code names `EffectsEngine`, `JSONValue`,
//! `NamedArguments`, `Vec2`, `Vec3`, `CraftColor` and `ItemReference`, which the hand-written Kotlin
//! sources define, and the internal `EffectsEngine.command` they call through. It meets every Kotlin rule
//! written code does, which the gate's android stage reads it for with Spotless, detekt and Lint: no `!!`,
//! no `TODO()`, no exit, so a call that fails throws, as the Swift facade does.
//!
//! Every argument of a call is passed by name, as the Swift facade's labels make every Swift call pass
//! its own. Kotlin has no named-only parameter, so each method takes first a `vararg` of a type with no
//! value ([`NAMED_ARGUMENTS`]): an argument passed by position would fill it and does not compile, and
//! every parameter after a `vararg` can only be named. A key EffectCraft adds, before another or after
//! the last, so takes no old call's value, not even a positional `extra`'s, which is the condition the
//! diff counts a new optional key compatible on wherever it stands (see [`crate::diff`]).
//!
//! A method's code names nothing a command, key or namespace of the registry could take from it. A
//! parameter is in scope in the defaults after it and in the body, a method of the class outranks a
//! library function of its name wherever its arguments fit, and a package's name loses to either, so a
//! bare `engine`, a bare `emptyMap()` (which a command `x.emptyMap` would answer, being a method that
//! takes no argument) and a qualified `kotlin.collections.emptyMap()` (which a key `kotlin` would take)
//! could all be reached by the wrong declaration. So the body reaches the engine through `this`, the
//! keys go to it as pairs, and `extra` defaults to the literal `null`.
//!
//! An enum key is emitted as a `value class` with a companion constant per word, never as a Kotlin
//! `enum class`: a `when` over a value class has no exhaustive form, so the compiler makes every caller
//! write an `else`, and a word the engine adds later breaks no build that compiles today — the Kotlin
//! shape of the same decision the Swift struct makes. Its constructor makes the value of any word, as the
//! Swift struct's `init(rawValue:)` does.

use crate::facade::{self, Dialect, ENGINE, EXTRA, EXTRA_DOC, EnumType, FROM_WORD_DOC, Method, Namespace, Param, RETURNS_DOC, THROWS_DOC, WORD_DOC};
use crate::types::KeyType;

/// The words Kotlin refuses as a plain identifier: its hard keywords, every one of which the facade can
/// reach as a name (a key `in`, a command `view.do`). The soft keywords (`await`, `field`, `param`,
/// `receiver`, and the modifiers like `data` or `suspend`) name a declaration cleanly, and putting
/// backticks where none are needed would only say the language reserves a word it does not. Names come
/// out with a lower-case first letter, so Kotlin's uppercase keywords are out of reach, like Swift's.
const KEYWORDS: &[&str] = &[
    "as",
    "break",
    "class",
    "continue",
    "do",
    "else",
    "false",
    "for",
    "fun",
    "if",
    "in",
    "interface",
    "is",
    "null",
    "object",
    "package",
    "return",
    "super",
    "this",
    "throw",
    "true",
    "try",
    "typealias",
    "typeof",
    "val",
    "var",
    "when",
    "while",
];

/// Every type the emitted code names, whether through an import, from the module's own package or, as
/// `JvmInline`, from the language's own. An enum key's values' type is nested in its namespace's class,
/// so one of these names there would stand in for the type it shadows — a reference key typed
/// `ItemReference` would take an enum key's words where both conform alike, and `@JvmInline` would name
/// the nested type — and [`check`] refuses such a name. A value type's name joins its method's name and
/// its key's, each with a capital first, so a name with one capital (`Boolean`, `Double`, `String`,
/// `Map`) cannot be one, nor can `Vec2` or `Vec3`, since a key that starts with a digit takes its
/// method's name in front. The acronyms and the two-word names can: a command `x.jSON` with a key `value`
/// would name one `JSONValue`, and `x.jvm` with a key `inline` one `JvmInline`. Every entry stays
/// weighed, as it is this list, not how names are built today, that [`check`] reads.
const NAMED_TYPES: &[&str] = &[
    "Boolean",
    "CraftColor",
    "Double",
    "EffectsEngine",
    "ItemReference",
    "JSONRepresentable",
    "JSONValue",
    "JvmInline",
    "Map",
    "NamedArguments",
    "String",
    "Vec2",
    "Vec3",
];

/// The package every generated file declares: the `:effects` module's namespace (AGENTS §9), the package
/// of the hand-written types the emitted code names beside it.
const PACKAGE: &str = "io.github.christopherlinnett.craftbridge.effects";

/// The longest line ktlint allows; documentation wraps under it (`.editorconfig`'s `max_line_length`).
const MAX_LINE: usize = 140;

/// The parameter every method takes first, a `vararg` of [`NAMED_ARGUMENTS`], so that every argument
/// after it is passed by name; a key of this name would be a second parameter of the name.
const NAMED_ONLY: &str = "namedOnly";

/// The type of [`NAMED_ONLY`]: `:core` defines it with no instance, so no call can fill the `vararg`.
const NAMED_ARGUMENTS: &str = "NamedArguments";

/// What documents [`NAMED_ONLY`], the same for every method.
const NAMED_ONLY_DOC: &str = "Takes no value: it stands first so that every argument after it is passed by name.";

/// The Kotlin spelling of an identifier.
pub(crate) struct Kotlin;

impl Dialect for Kotlin {
    fn escape(&self, name: &str) -> String {
        if KEYWORDS.contains(&name) { format!("`{name}`") } else { name.to_string() }
    }
}

/// The names the Kotlin emitter declares for itself, weighed for clashes the facade cannot see: an enum
/// key's values' type named like a type the emitted code names for its own would stand in for it inside
/// the namespace's class, and a key named like [`NAMED_ONLY`] would be a second parameter of that name.
/// So is a namespace below another named like the [`ENGINE`] member every namespace's class keeps its
/// engine by. The other parameter names, which Kotlin takes exactly as the facade picked them, need no
/// weighing here: the facade weighed those, [`crate::facade::EXTRA`] among them.
pub(crate) fn check(namespaces: &[Namespace]) -> Result<(), String> {
    for namespace in namespaces {
        if namespace.parent.is_some() && Kotlin.escape(&namespace.name) == ENGINE {
            return Err(format!(
                "namespace `{}`: its property would be named `{ENGINE}`, which every namespace's class of the Kotlin facade keeps its engine by",
                namespace.path
            ));
        }
        for method in &namespace.methods {
            if let Some(param) = method.params.iter().find(|param| param.name == NAMED_ONLY) {
                return Err(format!(
                    "command `{}` key `{}`: its parameter would be named `{NAMED_ONLY}`, which every method of the Kotlin facade takes first so that a call names its arguments",
                    method.id, param.key
                ));
            }
        }
        for each in &namespace.enums {
            if NAMED_TYPES.contains(&each.name.as_str()) {
                return Err(format!(
                    "command `{}` key `{}`: its values' type would be named `{}`, which the Kotlin facade names for a type of its own",
                    each.id, each.key, each.name
                ));
            }
        }
    }
    Ok(())
}

/// The file name and text of each namespace's Kotlin file, in the order of the namespaces. The folder they
/// are written into is the package's own folder, which `cargo xtask generate` names, so that this
/// facade owns that folder and no other generator's package folder beside it.
pub(crate) fn files(namespaces: &[Namespace]) -> Vec<(String, String)> {
    namespaces.iter().map(|namespace| (format!("{}.kt", namespace.type_name), file(namespace))).collect()
}

/// The subcommand that writes the Kotlin facade: the one name `run` answers to and its files' notice gives.
pub(crate) const COMMAND: &str = "kotlin";

fn file(namespace: &Namespace) -> String {
    let mut text = facade::header(COMMAND);
    text.push_str(&format!("\npackage {PACKAGE}\n\n"));
    // ktlint keeps one blank line between the blocks, so a file with no import has no import block.
    let imports = imports(namespace);
    if !imports.is_empty() {
        for import in imports {
            text.push_str(&format!("import io.github.christopherlinnett.craftbridge.core.{import}\n"));
        }
        text.push('\n');
    }
    // The class is declared next to the property that hands it out, in the namespace that holds it;
    // its values and methods follow in it, the shape the plan gives.
    let (holder, holder_value) = match &namespace.parent {
        Some(parent) => (parent.as_str(), ENGINE),
        None => ("EffectsEngine", "this"),
    };
    text.push_str(&doc("", &namespace.doc, []));
    text.push_str(&format!(
        "public val {holder}.{}: {}\n    get() = {}({holder_value})\n",
        Kotlin.escape(&namespace.name),
        namespace.type_name,
        namespace.type_name
    ));
    let members: Vec<String> = namespace.enums.iter().map(enum_block).chain(namespace.methods.iter().map(method_block)).collect();
    text.push('\n');
    text.push_str(&doc("", &namespace.doc, []));
    text.push_str(&format!("public class {} internal constructor(\n    internal val engine: EffectsEngine,\n", namespace.type_name));
    if members.is_empty() {
        text.push_str(")\n");
    } else {
        text.push_str(") {\n");
        text.push_str(&members.join("\n"));
        text.push_str("}\n");
    }
    text
}

/// The `:core` types and extensions the namespace's file names, and only those, in the sorted order
/// ktlint wants: `JSONValue`, which every method returns and passes `extra` as and every enum value's
/// type is written as, and [`NAMED_ARGUMENTS`], which every method takes first; the types of its keys;
/// the `jsonValue` extensions, through which a `Boolean`, a `Double` or a `String` key passes, every
/// other key type carrying its own; and `JSONRepresentable`, which its enum values' types are written as.
fn imports(namespace: &Namespace) -> Vec<&'static str> {
    let mut named: Vec<&'static str> = Vec::new();
    let mut push = |name: &'static str| {
        if !named.contains(&name) {
            named.push(name);
        }
    };
    if !namespace.methods.is_empty() {
        push("JSONValue");
        push(NAMED_ARGUMENTS);
    }
    for method in &namespace.methods {
        for param in &method.params {
            match param.key_type {
                KeyType::Bool | KeyType::Number | KeyType::Str => push("jsonValue"),
                KeyType::Vec2 => push("Vec2"),
                KeyType::Vec3 => push("Vec3"),
                KeyType::Color => push("CraftColor"),
                KeyType::Enum(_) | KeyType::Ref | KeyType::Json => {}
            }
        }
    }
    if !namespace.enums.is_empty() {
        push("JSONValue");
        push("JSONRepresentable");
    }
    named.sort_unstable();
    named
}

/// An enum key's values' type. Its constructor is how a word becomes a value, whether this file lists
/// the word or the engine added it since, so its documentation says so; a type with no listed word has
/// no companion, which ktlint would refuse empty.
fn enum_block(each: &EnumType) -> String {
    let mut text = doc("    ", &each.doc, [("constructor", FROM_WORD_DOC)]);
    text.push_str(&format!(
        "    @JvmInline\n    public value class {}(\n{}        public val word: String,\n    ) : JSONRepresentable {{\n        /** The value as the engine takes it: its word. */\n        override val jsonValue: JSONValue\n            get() = JSONValue.Text(word)\n",
        each.name,
        doc("        ", WORD_DOC, [])
    ));
    if !each.cases.is_empty() {
        text.push_str("\n        public companion object {\n");
        for (index, case) in each.cases.iter().enumerate() {
            if index > 0 {
                text.push('\n');
            }
            text.push_str(&doc("            ", &case.doc, []));
            text.push_str(&format!(
                "            public val {}: {} = {}(\"{}\")\n",
                Kotlin.escape(&case.name),
                each.name,
                each.name,
                string_literal(&case.value)
            ));
        }
        text.push_str("        }\n");
    }
    text.push_str("    }\n");
    text
}

fn method_block(method: &Method) -> String {
    let mut tags: Vec<(String, String)> = vec![(format!("param {NAMED_ONLY}"), NAMED_ONLY_DOC.to_string())];
    tags.extend(method.params.iter().map(|param| (format!("param {}", param.name), param.doc.clone())));
    tags.push((format!("param {EXTRA}"), EXTRA_DOC.to_string()));
    tags.push(("return".to_string(), RETURNS_DOC.to_string()));
    tags.push(("throws kotlin.Throwable".to_string(), THROWS_DOC.to_string()));
    let mut text = doc("    ", &method.summary, [("", method.detail.as_str())].into_iter().chain(tags.iter().map(|(tag, body)| (tag.as_str(), body.as_str()))));
    let mut parameters: Vec<String> = vec![format!("vararg {NAMED_ONLY}: {NAMED_ARGUMENTS}")];
    parameters.extend(method.params.iter().map(parameter));
    // `null`, a literal, not `emptyMap()`: a command named `emptyMap` is a method of the class, which a bare
    // call in a default value reaches before the standard library's (see the module's documentation).
    parameters.push(format!("{EXTRA}: Map<String, JSONValue>? = null"));
    // Every method has two parameters at least, the one that makes its arguments named and `extra`, and
    // ktlint_official puts each parameter of such a signature on a line of its own.
    text.push_str(&format!("    public suspend fun {}(\n", Kotlin.escape(&method.name)));
    for parameter in &parameters {
        text.push_str(&format!("        {parameter},\n"));
    }
    text.push_str("    ): JSONValue =\n");
    // Through `this`, as the Swift facade goes through `self`: a key named like the stored engine is a
    // parameter of that name, which a bare `engine` would reach instead of the member. The keys follow as
    // pairs, so no call to a library function is left for a name the registry gives to take.
    text.push_str(&format!("        this.{ENGINE}.command(\n            \"{}\",\n            {EXTRA},\n", string_literal(&method.id)));
    for param in &method.params {
        let value = Kotlin.escape(&param.name);
        let access = if param.required { value } else { format!("{value}?") };
        text.push_str(&format!("            \"{}\" to {access}.jsonValue,\n", string_literal(&param.key)));
    }
    text.push_str("        )\n");
    text
}

/// One parameter of a method: required, or optional and left out by default.
fn parameter(param: &Param) -> String {
    let name = Kotlin.escape(&param.name);
    let kotlin_type = type_name(param);
    if param.required { format!("{name}: {kotlin_type}") } else { format!("{name}: {kotlin_type}? = null") }
}

/// The Kotlin type a key is passed as: the shared table, with Kotlin's word for a boolean.
fn type_name(param: &Param) -> String {
    facade::type_name(param, "Boolean")
}

/// A KDoc comment at `indent`: one line if the text fits one and there are no sections, a block
/// otherwise. Each section adds a paragraph after a blank comment line; its tag, empty for a plain
/// paragraph, names the KDoc tag whose text is wrapped with it, and consecutive tags share no blank
/// line between them, as hand-written KDoc lines them.
fn doc<'a>(indent: &str, text: &str, sections: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    // The one-line form's frame is the widest a line of the comment carries.
    let width = room(indent.chars().count() + "/** ".len() + " */".len());
    let lines = wrapped(&comment_safe(text), width);
    let mut sections = sections.into_iter().peekable();
    if let (None, [line]) = (sections.peek(), lines.as_slice()) {
        return format!("{indent}/** {line} */\n");
    }
    let mut out = format!("{indent}/**\n");
    for line in lines {
        out.push_str(&format!("{indent} * {line}\n"));
    }
    let mut tag_was_last = false;
    for (tag, body) in sections {
        if tag.is_empty() || !tag_was_last {
            out.push_str(&format!("{indent} *\n"));
        }
        tag_was_last = !tag.is_empty();
        if tag.is_empty() {
            for line in wrapped(&comment_safe(body), width) {
                out.push_str(&format!("{indent} * {line}\n"));
            }
            continue;
        }
        let first = format!("{indent} * @{tag} ");
        let rest = format!("{indent} * ");
        for line in wrapped_after(&first, &rest, &comment_safe(body)) {
            out.push_str(&format!("{line}\n"));
        }
    }
    out.push_str(&format!("{indent} */\n"));
    out
}

/// `text` as the inside of a block comment: Kotlin nests block comments, so a `/*` the registry writes in
/// a label or a description would open one that nothing closes, and a `*/` would end the KDoc early. Each
/// spells its slash as the HTML entity KDoc's Markdown reads as the slash, so the text keeps its meaning.
fn comment_safe(text: &str) -> String {
    text.replace("*/", "*&#47;").replace("/*", "&#47;*")
}

/// A word of the text, or a piece of a word that is too long for any whole line: the parts of one word
/// run together without a space, so breaking says nothing the word did not.
enum Piece<'a> {
    Word(&'a str),
    Part(&'a str),
}

/// `text`'s words, each too long for `width` split: a `|`-joined list, as the registry spells a word
/// list, at its `|`s; anything else at `width` itself, since a line no longer than that is what the
/// formatter allows and no name is worth a comment that breaks it otherwise.
fn pieces(text: &str, width: usize) -> Vec<Piece<'_>> {
    text.split_whitespace().flat_map(|word| split_long(word, width)).collect()
}

fn split_long<'a>(word: &'a str, width: usize) -> Vec<Piece<'a>> {
    // A cut takes one letter at least, so a word always gets shorter and the split ends.
    let width = width.max(1);
    if word.chars().count() <= width {
        return vec![Piece::Word(word)];
    }
    let mut out: Vec<Piece<'a>> = Vec::new();
    let mut rest = word;
    while rest.chars().count() > width {
        let cut = fit_cut(rest, width);
        let (head, tail) = rest.split_at(cut);
        rest = tail;
        out.push(Piece::Part(head));
    }
    out.push(if out.is_empty() { Piece::Word(rest) } else { Piece::Part(rest) });
    out
}

/// Where to cut a too-long word, as a byte index on a character boundary: after the last `|` that fits
/// a line of `width`, or, when a single `|`-joined item is what is too long, at `width` itself.
fn fit_cut(word: &str, width: usize) -> usize {
    let mut last_pipe = 0;
    for (count, (index, letter)) in word.char_indices().enumerate() {
        if count == width {
            return if last_pipe > 0 { last_pipe } else { index };
        }
        if letter == '|' {
            last_pipe = index + 1;
        }
    }
    word.len()
}

/// `text` split into lines no longer than `width` characters, at spaces, and at the parts of a word
/// too long for a whole line.
fn wrapped(text: &str, width: usize) -> Vec<String> {
    wrap_pieces(&pieces(text, width), "", "", width, width)
}

/// `text` joined into lines that start with `first` and continue with `rest`, none longer than
/// [`MAX_LINE`] characters.
fn wrapped_after(first: &str, rest: &str, text: &str) -> Vec<String> {
    let first_room = room(first.chars().count());
    wrap_pieces(&pieces(text, first_room), first, rest, first_room, room(rest.chars().count()))
}

/// What is left of a line once `used` characters of it are taken, one at least. A name from the
/// registry can be longer than any line: what is left is then a line too long for ktlint, which the
/// gate names, never a count below zero that stops the generator.
fn room(used: usize) -> usize {
    MAX_LINE.saturating_sub(used).max(1)
}

fn wrap_pieces(pieces: &[Piece<'_>], first: &str, rest: &str, first_room: usize, room_after: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut prefix = first;
    let mut room = first_room;
    let mut line = String::new();
    for piece in pieces {
        let word = match piece {
            Piece::Word(word) => *word,
            Piece::Part(part) => *part,
        };
        let spaced = matches!(piece, Piece::Word(_)) && !line.is_empty();
        let need = word.chars().count() + usize::from(spaced);
        if !line.is_empty() && need > room {
            lines.push(format!("{prefix}{line}"));
            prefix = rest;
            room = room_after;
            line = String::new();
        }
        if spaced {
            line.push(' ');
        }
        line.push_str(word);
        room = room.saturating_sub(need);
    }
    lines.push(format!("{prefix}{line}"));
    lines
}

/// `text` between the quotes of a Kotlin string literal: `$` interpolated nothing on its own only when
/// not followed by an identifier or `{`, but escaping every one keeps the rule one escape table rather
/// than a condition on what follows.
fn string_literal(text: &str) -> String {
    facade::string_literal(text, "$", |letter| format!("\\u{:04x}", u32::from(letter)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facade::{plan_emitted, refusal, text_in};
    use crate::model::{Command, Key};

    fn text_of(commands: &[Command], name: &str) -> String {
        text_in(plan_emitted(files, commands).into_iter(), name, |file, want| file.rsplit('/').next() == Some(want))
    }

    /// The imports a file of `text` opens with, one name each.
    fn imported(text: &str) -> Vec<&str> {
        text.lines().filter_map(|line| line.strip_prefix("import io.github.christopherlinnett.craftbridge.core.")).collect()
    }

    #[test]
    fn reserved_words_are_escaped_and_everything_else_is_plain() {
        for word in ["in", "do", "object", "when", "typeof", "fun", "val", "is", "as"] {
            assert_eq!(Kotlin.escape(word), format!("`{word}`"));
        }
        for word in ["await", "open", "it", "field", "param", "receiver", "data", "sealed", "suspend", "value"] {
            assert_eq!(Kotlin.escape(word), word, "{word} is no Kotlin keyword and takes no backticks");
        }
        assert_eq!(Kotlin.escape("none"), "none", "Kotlin has no optional-dot sugar that takes a value's own name away, so no case is renamed");
    }

    #[test]
    fn a_method_takes_every_key_by_name_in_order_and_extra_last() {
        let text = text_of(
            &[Command::of(
                "layer.newText",
                "New Text",
                vec![Key::of("text", false, KeyType::Str), Key::of("weight", false, KeyType::Number), Key::of("track", true, KeyType::Ref)],
            )],
            "LayerCommands.kt",
        );
        assert!(
            text.contains(
                "    public suspend fun newText(\n        vararg namedOnly: NamedArguments,\n        text: String? = null,\n        weight: Double? = null,\n        track: ItemReference,\n        extra: Map<String, JSONValue>? = null,\n    ): JSONValue =\n        this.engine.command(\n            \"layer.newText\",\n            extra,\n            \"text\" to text?.jsonValue,\n            \"weight\" to weight?.jsonValue,\n            \"track\" to track.jsonValue,\n        )\n"
            ),
            "{text}"
        );
        assert!(text.contains("     * @param namedOnly Takes no value: it stands first so that every argument after it is passed by name.\n     * @param text The key `text`: text?"), "{text}");
        assert!(text.contains("@param extra Keys the typed parameters do not cover"), "{text}");
        assert!(text.contains("@return The engine's reply."), "{text}");
        assert!(text.contains("@throws kotlin.Throwable Whatever the engine refuses the call with."), "{text}");
    }

    #[test]
    fn a_command_with_no_keys_still_takes_its_arguments_by_name_and_sends_only_extra() {
        let text = text_of(&[Command::of("anim.addKeyframe", "Add Keyframe", vec![])], "AnimCommands.kt");
        assert!(
            text.contains("    public suspend fun addKeyframe(\n        vararg namedOnly: NamedArguments,\n        extra: Map<String, JSONValue>? = null,\n    ): JSONValue =\n        this.engine.command(\n            \"anim.addKeyframe\",\n            extra,\n        )\n"),
            "{text}"
        );
        assert!(text.contains("@param extra Keys"), "{text}");
    }

    #[test]
    fn every_key_type_takes_its_kotlin_type() {
        for (key_type, name) in [
            (KeyType::Bool, "Boolean"),
            (KeyType::Number, "Double"),
            (KeyType::Str, "String"),
            (KeyType::Vec2, "Vec2"),
            (KeyType::Vec3, "Vec3"),
            (KeyType::Color, "CraftColor"),
            (KeyType::Ref, "ItemReference"),
            (KeyType::Json, "JSONValue"),
        ] {
            let text = text_of(&[Command::of("a.b", "B", vec![Key::of("k", false, key_type)])], "ACommands.kt");
            assert!(text.contains(&format!("k: {name}? = null,")), "{name} missing from {text}");
            assert!(text.contains(r#""k" to k?.jsonValue,"#), "{name} maps wrongly in {text}");
        }
    }

    #[test]
    fn an_enum_key_is_a_value_class_so_no_when_can_be_exhaustive() {
        let kind = KeyType::Enum(vec!["sameAsLayer".to_string(), "gaussianBlur".to_string()]);
        let text = text_of(&[Command::of("layer.motionBlur", "Motion Blur", vec![Key::of("mode", false, kind)])], "LayerCommands.kt");
        assert!(text.contains("    @JvmInline\n    public value class MotionBlurMode(\n"), "{text}");
        assert!(!text.contains("enum class"), "a Kotlin enum would let a `when` be exhaustive, and a word the engine adds later would break the build: {text}");
        assert!(text.contains("public companion object {"), "{text}");
        assert!(text.contains("public val sameAsLayer: MotionBlurMode = MotionBlurMode(\"sameAsLayer\")"), "{text}");
        assert!(text.contains("public val gaussianBlur: MotionBlurMode = MotionBlurMode(\"gaussianBlur\")"), "{text}");
        // The constructor is the one way any word becomes a value, as Swift's `init(rawValue:)` is.
        assert!(text.contains(&format!("     * @constructor {FROM_WORD_DOC}\n     */\n    @JvmInline\n")), "{text}");
        assert!(text.contains("        public val word: String,\n    ) : JSONRepresentable {"), "{text}");
        assert!(!text.contains("fromWord"), "{text}");
        assert!(text.contains("override val jsonValue: JSONValue\n            get() = JSONValue.Text(word)"), "{text}");
        assert!(text.contains("mode: MotionBlurMode? = null,"), "{text}");
    }

    #[test]
    fn an_enum_key_whose_surface_lists_no_word_has_no_empty_companion() {
        let text = text_of(&[Command::of("layer.motionBlur", "Motion Blur", vec![Key::of("mode", false, KeyType::Enum(vec![]))])], "LayerCommands.kt");
        assert!(text.contains("            get() = JSONValue.Text(word)\n    }\n"), "{text}");
        assert!(!text.contains("companion"), "{text}");
    }

    #[test]
    fn a_namespace_below_another_is_a_property_of_its_parent_holding_its_engine() {
        let commands = [Command::of("layer.mask.motionBlur", "Motion Blur", vec![])];
        let names: Vec<String> =
            plan_emitted(files, &commands).into_iter().map(|(name, _)| name.rsplit('/').map(str::to_string).next().unwrap_or_default()).collect();
        assert_eq!(names, ["LayerCommands.kt", "LayerMaskCommands.kt"], "{names:?}");
        let parent = text_of(&commands, "LayerCommands.kt");
        assert!(parent.contains("public val EffectsEngine.layer: LayerCommands\n    get() = LayerCommands(this)"), "{parent}");
        let mask = text_of(&commands, "LayerMaskCommands.kt");
        assert!(mask.contains("public val LayerCommands.mask: LayerMaskCommands\n    get() = LayerMaskCommands(engine)"), "{mask}");
        assert!(mask.contains("public class LayerMaskCommands internal constructor(\n    internal val engine: EffectsEngine,\n) {"), "{mask}");
    }

    #[test]
    fn the_file_imports_what_it_names_and_nothing_it_does_not() {
        let typed = text_of(
            &[Command::of("layer.newText", "New Text", vec![Key::of("pos", false, KeyType::Vec2), Key::of("tint", false, KeyType::Color)])],
            "LayerCommands.kt",
        );
        assert_eq!(imported(&typed), ["CraftColor", "JSONValue", "NamedArguments", "Vec2"], "{typed}");
        // A boolean, a number and a string pass through `:core`'s extensions; every other key type,
        // `JSONValue` itself among them, carries its own `jsonValue`.
        let plain = text_of(&[Command::of("a.b", "B", vec![Key::of("k", false, KeyType::Json), Key::of("r", false, KeyType::Ref)])], "ACommands.kt");
        assert_eq!(imported(&plain), ["JSONValue", "NamedArguments"], "{plain}");
        for key_type in [KeyType::Bool, KeyType::Number, KeyType::Str] {
            let text = text_of(&[Command::of("a.b", "B", vec![Key::of("k", false, key_type)])], "ACommands.kt");
            assert_eq!(imported(&text), ["JSONValue", "NamedArguments", "jsonValue"], "{text}");
        }
        let kind = KeyType::Enum(vec!["on".to_string()]);
        let enums = text_of(&[Command::of("a.b", "B", vec![Key::of("k", false, kind)])], "ACommands.kt");
        assert_eq!(imported(&enums), ["JSONRepresentable", "JSONValue", "NamedArguments"], "{enums}");
        let holder = text_of(&[Command::of("outer.inner.run", "Run", vec![])], "OuterCommands.kt");
        assert!(imported(&holder).is_empty(), "a namespace with no command of its own names none of them: {holder}");
    }

    #[test]
    fn what_the_engine_writes_cannot_end_a_string_literal_early_or_interpolate() {
        assert_eq!(string_literal("a\"b\\c$de"), "a\\\"b\\\\c\\$de");
        assert_eq!(string_literal("a\nb"), "a\\u000ab");
        let text = text_of(&[Command::of("a.b\"c", "B", vec![Key::of("k$", false, KeyType::Json)])], "ACommands.kt");
        assert!(text.contains("\"a.b\\\"c\","), "{text}");
        assert!(text.contains("\"k\\$\" to"), "{text}");
    }

    #[test]
    fn registry_text_cannot_open_or_close_a_comment() {
        // Each way registry text reaches a comment: the label as the summary, the `params` text as the
        // paragraph after it, and a key's piece in its `@param` tag.
        let tricky = "ends */ early, opens /* nested, and /*/ and /**/ and **/";
        let mut command = Command::of("a.b", tricky, vec![Key::of("k", false, KeyType::Json)]);
        command.params = tricky.to_string();
        if let Some(key) = command.keys.first_mut() {
            key.doc = tricky.to_string();
        }
        let text = text_of(&[command], "ACommands.kt");
        assert_eq!(text.matches("ends *&#47; early").count(), 3, "{text}");
        let mut depth = 0_i32;
        let mut rest = text.as_str();
        while let Some(at) = rest.find(['/', '*']) {
            let pair = rest.get(at..at + 2).unwrap_or("");
            match pair {
                "/*" => {
                    depth += 1;
                    rest = rest.get(at + 2..).unwrap_or("");
                }
                "*/" => {
                    depth -= 1;
                    assert!(depth >= 0, "closed early: {text}");
                    rest = rest.get(at + 2..).unwrap_or("");
                }
                _ => rest = rest.get(at + 1..).unwrap_or(""),
            }
        }
        assert_eq!(depth, 0, "a comment is left open: {text}");
    }

    #[test]
    fn documentation_wraps_under_the_line_ktlint_allows() {
        let long = format!("New Text. {}", "filler ".repeat(60));
        let text = text_of(&[Command::of("layer.newText", long.trim(), vec![Key::of("blob", false, KeyType::Json)])], "LayerCommands.kt");
        assert!(!text.contains("  \n"), "a wrapped line keeps its words: {text}");
        for line in text.lines() {
            assert!(line.chars().count() <= MAX_LINE, "over {MAX_LINE}: {line}");
        }
    }

    #[test]
    fn a_key_name_longer_than_any_line_is_emitted_and_left_to_the_formatter_to_name() {
        // A name from the registry is input: one too long for its `@param` line leaves less room than
        // none (130), or none at all (125), and the file still comes out. Its over-long lines are then
        // ktlint's to name in the gate, as any other line over the limit is.
        for length in [130, 125, 200] {
            let key = "k".repeat(length);
            let text = text_of(&[Command::of("a.b", "B", vec![Key::of(&key, false, KeyType::Json)])], "ACommands.kt");
            assert!(text.contains(&format!("     * @param {key} ")), "{length}: {text}");
            assert!(text.contains(&format!("        {key}: JSONValue? = null,\n")), "{length}: {text}");
        }
    }

    #[test]
    fn an_enum_key_whose_type_would_take_a_name_the_facade_uses_is_refused() {
        crate::facade::assert_facade_names_are_refused(check, NAMED_TYPES, type_name);
        // The names only this language's facade writes are weighed too: its JSON machinery's, the
        // annotation every value class carries, and the type that makes every argument named.
        let kind = KeyType::Enum(vec!["layer".to_string(), "comp".to_string()]);
        for (id, key, name) in [("a.jSON", "representable", "JSONRepresentable"), ("a.jvm", "inline", "JvmInline"), ("a.named", "arguments", "NamedArguments")]
        {
            let error = refusal(check, &[Command::of(id, "J", vec![Key::of(key, false, kind.clone())])]);
            assert!(error.contains(&format!("`{name}`")), "{error}");
        }
    }

    #[test]
    fn a_key_named_like_the_parameter_that_makes_every_argument_named_is_refused() {
        let error = refusal(check, &[Command::of("a.b", "B", vec![Key::of("namedOnly", false, KeyType::Bool)])]);
        assert!(error.contains("command `a.b` key `namedOnly`") && error.contains("`namedOnly`") && error.contains("names its arguments"), "{error}");
        let folded = refusal(check, &[Command::of("a.b", "B", vec![Key::of("named_only", false, KeyType::Bool)])]);
        assert!(folded.contains("key `named_only`"), "the facade folds a key into the same parameter name: {folded}");
        assert!(refusal(check, &[Command::of("a.b", "B", vec![Key::of("named", false, KeyType::Bool)])]).is_empty());
    }

    #[test]
    fn a_key_named_like_the_stored_engine_does_not_take_the_call_from_it() {
        let text = text_of(&[Command::of("a.b", "B", vec![Key::of("engine", false, KeyType::Str)])], "ACommands.kt");
        assert!(text.contains("        engine: String? = null,\n") && text.contains("    ): JSONValue =\n        this.engine.command(\n"), "{text}");
    }

    /// The lines of `text` that are code, not documentation or a declaration of the name, which call a
    /// library function by a name the registry could give a command, a key or a namespace too.
    fn library_calls(text: &str) -> Vec<&str> {
        text.lines()
            .filter(|line| !line.trim_start().starts_with('*') && !line.trim_start().starts_with("/**") && !line.starts_with("//"))
            .filter(|line| !line.contains(" fun "))
            .filter(|line| ["emptyMap(", "mapOf(", "kotlin."].iter().any(|call| line.contains(call)))
            .collect()
    }

    #[test]
    fn no_command_key_or_namespace_of_a_library_function_s_name_can_take_a_call_from_it() {
        let commands = [
            Command::of("cache.emptyMap", "Empty", vec![Key::of("mapOf", false, KeyType::Str), Key::of("kotlin", false, KeyType::Json)]),
            Command::of("cache.mapOf", "Map", vec![Key::of("emptyMap", false, KeyType::Bool)]),
            Command::of("cache.kotlin.run", "Run", vec![]),
            Command::of("cache.diskStats", "Stats", vec![]),
        ];
        let text = text_of(&commands, "CacheCommands.kt");
        assert!(library_calls(&text).is_empty(), "{:?}\n{text}", library_calls(&text));
        assert_eq!(text.matches("        extra: Map<String, JSONValue>? = null,\n").count(), 3, "{text}");
        assert!(text.contains("            \"cache.diskStats\",\n            extra,\n        )\n"), "{text}");
    }

    #[test]
    fn a_namespace_below_another_named_like_the_stored_engine_is_refused() {
        let error = refusal(check, &[Command::of("view.engine.front", "Front", vec![])]);
        assert!(error.contains("namespace `view.engine`") && error.contains("`engine`"), "{error}");
        let caps = refusal(check, &[Command::of("view.Engine.front", "Front", vec![])]);
        assert!(caps.contains("namespace `view.Engine`") && caps.contains("`engine`"), "{caps}");
        let kept = refusal(check, &[Command::of("engine.info", "Info", vec![]), Command::of("view.engines.front", "Front", vec![])]);
        assert!(kept.is_empty(), "{kept}");
    }

    #[test]
    fn a_namespace_with_no_command_of_its_own_still_has_its_class() {
        let outer = text_of(&[Command::of("outer.inner.run", "Run", vec![])], "OuterCommands.kt");
        assert!(outer.contains("public class OuterCommands internal constructor(\n    internal val engine: EffectsEngine,\n)\n"), "{outer}");
        assert!(outer.contains(&format!("\npackage {PACKAGE}\n\n/** The `outer` commands. */\n")), "one blank line, as ktlint keeps it: {outer}");
        assert!(!outer.contains("public suspend fun"), "{outer}");
    }

    #[test]
    fn every_namespace_and_command_at_the_pin_is_emitted_once() {
        let emitted = crate::facade::assert_pin_emitted_once(files, "    public suspend fun ");
        let methods: usize = emitted.iter().map(|(_, text)| text.matches("(\n        vararg namedOnly: NamedArguments,\n").count()).sum();
        assert_eq!(methods, crate::surface::commands().len(), "every method takes its arguments by name");
        for (_, text) in &emitted {
            assert!(!text.contains("!!") && !text.contains("TODO(") && !text.contains("enum class"), "{text}");
            assert!(library_calls(text).is_empty(), "{:?}", library_calls(text));
            for line in text.lines() {
                assert!(line.chars().count() <= MAX_LINE, "over {MAX_LINE}: {line}");
            }
        }
    }

    #[test]
    fn the_files_are_named_for_their_namespaces_and_declare_the_effects_package() {
        for (name, text) in plan_emitted(files, &[Command::of("layer.newText", "New Text", vec![])]) {
            assert_eq!(name, "LayerCommands.kt");
            assert!(text.starts_with(&facade::header(COMMAND)), "{text}");
            assert!(text.contains(&format!("\npackage {PACKAGE}\n")), "{text}");
        }
    }
}
