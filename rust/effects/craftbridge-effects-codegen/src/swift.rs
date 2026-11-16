//! The Swift facade: one file for each command namespace, with a typed `async throws` method for each
//! command, in the shape the plan's API generator design gives:
//!
//! ```swift
//! extension EffectsEngine.LayerCommands {
//!     public func newText(text: String? = nil, extra: [String: JSONValue] = [:]) async throws -> JSONValue
//! }
//! ```
//!
//! Grouping, naming, ordering and the documentation's sentences come from [`crate::facade`]; only how
//! Swift spells and lays them out is here. The emitted code names `EffectsEngine`, `JSONValue`, `Vec2`,
//! `Vec3`, `CraftColor` and `ItemReference`, which the hand-written Swift sources define, and the internal
//! `EffectsEngine.command` they call through. It meets every Swift rule written code does, which the gate's
//! Swift stages read it for: no force unwrap, `try!` or `fatalError`, so a call that fails throws.

use crate::facade::{Dialect, EXTRA, EXTRA_DOC, EnumType, FROM_WORD_DOC, Method, Namespace, Param, RETURNS_DOC, THROWS_DOC, WORD_DOC, first_clash};
use crate::types::KeyType;

/// The words Swift refuses as a name, without backticks, in at least one place the facade writes one: a
/// method, a property, an enum key's value (a static member), or a parameter's label or value, each settled
/// by compiling it. The
/// contextual keywords it takes as a name in all of those (`open`, `async`, `actor`, `package`, `some`)
/// are left out; `Type` and `Protocol` are in, since a member of either name cannot be reached by a dot.
const KEYWORDS: &[&str] = &[
    "Any",
    "Self",
    "Type",
    "Protocol",
    "as",
    "associatedtype",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "continue",
    "default",
    "defer",
    "deinit",
    "do",
    "else",
    "enum",
    "extension",
    "fallthrough",
    "false",
    "fileprivate",
    "for",
    "func",
    "guard",
    "if",
    "import",
    "in",
    "init",
    "inout",
    "internal",
    "is",
    "let",
    "nil",
    "operator",
    "precedencegroup",
    "private",
    "protocol",
    "public",
    "repeat",
    "rethrows",
    "return",
    "self",
    "static",
    "struct",
    "subscript",
    "super",
    "switch",
    "throw",
    "throws",
    "true",
    "try",
    "typealias",
    "var",
    "where",
    "while",
];

/// `Optional`'s own case names: a call passing `.none` for an optional parameter takes the optional's own
/// `.none` over an enum value of that name, which would silently leave the key out. An enum value of
/// either name is renamed; a method, property or parameter of the name is no enum value and keeps it.
const OPTIONAL_CASES: &[&str] = &["none", "some"];

/// The stored property each namespace's type holds its engine by. A namespace below another is a property
/// of its parent's type, so one named this would be a second declaration of the name there.
const ENGINE: &str = "engine";

/// Every type the emitted code names without qualifying it. An enum key's value type is nested in its
/// namespace's type, so one of the same name would stand in for it there, and still compile where both
/// conform alike: a reference key typed `ItemReference` would take an enum key's words. [`check`] refuses
/// such a name. A value type's name joins its method's name and its key's, each with a capital first, so a
/// name with one capital (`Bool`, `Double`, `String`, `Sendable`, `Hashable`) cannot be one, nor can
/// `Vec2` or `Vec3`, since a key that starts with a digit takes its method's name in front. The acronyms
/// can: a command `x.jSON` with a key `value` would name one `JSONValue`. Every entry stays weighed, as it is
/// this list, not how names are built today, that [`check`] reads.
const NAMED_TYPES: &[&str] = &[
    "Bool",
    "CraftColor",
    "Double",
    "EffectsEngine",
    "Hashable",
    "ItemReference",
    "JSONStringRepresentable",
    "JSONValue",
    "Sendable",
    "String",
    "Vec2",
    "Vec3",
];

/// The Swift spelling of an identifier.
pub(crate) struct Swift;

impl Dialect for Swift {
    fn escape(&self, name: &str) -> String {
        if KEYWORDS.contains(&name) { format!("`{name}`") } else { name.to_string() }
    }
}

/// The name an enum value is declared with: an enum value is a static member of its type, so the renamed
/// [`OPTIONAL_CASES`] names are what a call's `.none` would otherwise take for the optional's own.
fn case_name(dialect: &dyn Dialect, name: &str) -> String {
    if OPTIONAL_CASES.contains(&name) { format!("{name}Value") } else { dialect.escape(name) }
}

/// The Swift names the emitter declares for itself, weighed for clashes the facade cannot see: two names
/// the surface keeps apart can take one Swift name once Swift's own renames are done — the values `None`
/// and `noneValue` are both `noneValue`, keys `in` and `inValue` both bind the name `inValue` — and Swift
/// would refuse the file without naming which two. The facade weighs the names it picks; what this emitter
/// renames is weighed here, naming both, so no command's emit depends on a compiler to catch it. So is a
/// namespace below another named like the [`ENGINE`] property its parent's type stores, and an enum key's
/// type named like one of [`NAMED_TYPES`], which the compiler would not catch at all.
pub(crate) fn check(namespaces: &[Namespace]) -> Result<(), String> {
    for namespace in namespaces {
        if namespace.parent.is_some() && Swift.escape(&namespace.name) == ENGINE {
            return Err(format!(
                "namespace `{}`: its property would be named `{ENGINE}`, which every namespace's type of the Swift facade stores its engine by",
                namespace.path
            ));
        }
        for method in &namespace.methods {
            let named: Vec<(String, String)> = method.params.iter().map(|param| (binding(&Swift, param).1, format!("key `{}`", param.key))).collect();
            if let Some((name, one, other)) = first_clash(named.iter().map(|(name, what)| (name.as_str(), what.clone())).collect()) {
                return Err(format!("command `{}`: {one} and {other} take the Swift name `{name}`", method.id));
            }
        }
        for each in &namespace.enums {
            if NAMED_TYPES.contains(&each.name.as_str()) {
                return Err(format!(
                    "command `{}` key `{}`: its values' type would be named `{}`, which the Swift facade names for a type of its own",
                    each.id, each.key, each.name
                ));
            }
            let named: Vec<(String, String)> = each.cases.iter().map(|case| (case_name(&Swift, &case.name), format!("the value `{}`", case.value))).collect();
            if let Some((name, one, other)) = first_clash(named.iter().map(|(name, what)| (name.as_str(), what.clone())).collect()) {
                return Err(format!("command `{}` key `{}`: {one} and {other} take the Swift name `{name}`", each.id, each.key));
            }
        }
    }
    Ok(())
}

/// The file name and text of each namespace's Swift file, in the order of the namespaces.
pub(crate) fn files(namespaces: &[Namespace]) -> Vec<(String, String)> {
    namespaces.iter().map(|namespace| (format!("{}.swift", namespace.type_name), file(&Swift, namespace))).collect()
}

fn file(dialect: &dyn Dialect, namespace: &Namespace) -> String {
    let type_name = &namespace.type_name;
    let name = dialect.escape(&namespace.name);
    let (holder, holder_value) = match &namespace.parent {
        Some(parent) => (format!("EffectsEngine.{parent}"), ENGINE),
        None => ("EffectsEngine".to_string(), "self"),
    };
    let mut text = String::new();
    text.push_str(&header());
    text.push_str("\nimport CraftBridge\n\n");
    // The type is declared next to the property that hands it out, in the namespace that holds it; the
    // methods follow in an extension, the shape the plan gives.
    let declaration = format!(
        "extension {holder} {{\n    /// {doc}\n    public var {name}: EffectsEngine.{type_name} {{\n        EffectsEngine.{type_name}({ENGINE}: {holder_value})\n    }}\n}}\n\nextension EffectsEngine {{\n    /// {doc}\n    public struct {type_name}: Sendable {{\n        let {ENGINE}: EffectsEngine\n    }}\n}}\n",
        doc = namespace.doc,
    );
    text.push_str(&declaration);
    if !namespace.enums.is_empty() || !namespace.methods.is_empty() {
        text.push_str(&format!("\nextension EffectsEngine.{type_name} {{\n"));
        let blocks: Vec<String> =
            namespace.enums.iter().map(|each| enum_block(dialect, each)).chain(namespace.methods.iter().map(|method| method_block(dialect, method))).collect();
        text.push_str(&blocks.join("\n"));
        text.push_str("}\n");
    }
    text
}

/// The subcommand that writes the Swift facade: the one name `run` answers to and its files' notice gives.
pub(crate) const COMMAND: &str = "swift";

/// The file's first lines: the generated notice, then where to make a change instead.
fn header() -> String {
    format!("{}// Change the generator, which reads EffectCraft's command registry at the pinned revision.\n", crate::generated_notice(COMMAND))
}

/// An enum key is emitted as a `RawRepresentable` struct with a static member per value, not as a Swift
/// `enum`: no `switch` over a struct can be exhaustive, so the compiler makes every caller write a
/// `default` clause, and a word the engine adds later breaks no build that compiles today. A call's
/// `.word` spelling keeps working because each word is a static member of the type.
fn enum_block(dialect: &dyn Dialect, each: &EnumType) -> String {
    let mut text = format!(
        "    /// {}\n    public struct {}: Sendable, Hashable, JSONStringRepresentable {{\n        /// {WORD_DOC}\n        public let rawValue: String\n\n        /// {FROM_WORD_DOC}\n        public init(rawValue: String) {{\n            self.rawValue = rawValue\n        }}\n",
        each.doc, each.name
    );
    for case in &each.cases {
        text.push_str(&format!(
            "        /// {}\n        public static let {} = {}(rawValue: \"{}\")\n",
            case.doc,
            case_name(dialect, &case.name),
            each.name,
            string_literal(&case.value)
        ));
    }
    text.push_str("    }\n");
    text
}

fn method_block(dialect: &dyn Dialect, method: &Method) -> String {
    let mut text = format!("    /// {}\n    ///\n    /// {}\n    ///\n", method.summary, method.detail);
    let many = !method.params.is_empty();
    text.push_str(if many { "    /// - Parameters:\n" } else { "    /// - Parameter " });
    for param in &method.params {
        text.push_str(&format!("    ///   - {}: {}\n", binding(dialect, param).1, param.doc));
    }
    text.push_str(&if many { format!("    ///   - {EXTRA}: {EXTRA_DOC}\n") } else { format!("{EXTRA}: {EXTRA_DOC}\n") });
    text.push_str(&format!("    /// - Returns: {RETURNS_DOC}\n    /// - Throws: {THROWS_DOC}\n"));
    let mut parameters: Vec<String> = method.params.iter().map(|param| parameter(dialect, param)).collect();
    parameters.push(format!("{EXTRA}: [String: JSONValue] = [:]"));
    text.push_str(&format!("    public func {}(\n", dialect.escape(&method.name)));
    text.push_str(&parameters.iter().map(|parameter| format!("        {parameter}")).collect::<Vec<_>>().join(",\n"));
    text.push_str(&format!("\n    ) async throws -> JSONValue {{\n        try await self.{ENGINE}.command(\n"));
    text.push_str(&format!("            \"{}\",\n", string_literal(&method.id)));
    if method.params.is_empty() {
        text.push_str("            [:],\n");
    } else {
        text.push_str("            [\n");
        // swift-format wants a trailing comma in a collection of several elements and none in one of a single element.
        let comma = if method.params.len() > 1 { "," } else { "" };
        for param in &method.params {
            let value = binding(dialect, param).1;
            let access = if param.required { value } else { format!("{value}?") };
            text.push_str(&format!("                \"{}\": {access}.jsonValue{comma}\n", string_literal(&param.key)));
        }
        text.push_str("            ],\n");
    }
    text.push_str(&format!("            {EXTRA}: {EXTRA}\n        )\n    }}\n"));
    text
}

/// One parameter of a method: required, or optional and left out by default.
fn parameter(dialect: &dyn Dialect, param: &Param) -> String {
    let (label, local) = binding(dialect, param);
    let swift_type = type_name(param);
    let name = if label == local { label } else { format!("{label} {local}") };
    if param.required { format!("{name}: {swift_type}") } else { format!("{name}: {swift_type}? = nil") }
}

/// The label a call writes for a key and the name the method's body and its documentation use for the
/// value. They are one name, but for a reserved word: its label is the word in backticks, since a call
/// writes `in:` however the language reads the word, and the value is named apart, since swift-format's
/// check of a method's documentation cannot match a name written in backticks.
fn binding(dialect: &dyn Dialect, param: &Param) -> (String, String) {
    let label = dialect.escape(&param.name);
    if label.starts_with('`') { (label, format!("{}Value", param.name)) } else { (label.clone(), label) }
}

/// The Swift type a key is passed as.
fn type_name(param: &Param) -> String {
    match &param.key_type {
        KeyType::Bool => "Bool".to_string(),
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

/// `text` between the quotes of a Swift string literal.
fn string_literal(text: &str) -> String {
    text.chars()
        .map(|letter| match letter {
            '\\' => "\\\\".to_string(),
            '"' => "\\\"".to_string(),
            letter if letter.is_control() => format!("\\u{{{:x}}}", u32::from(letter)),
            letter => letter.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facade::plan;
    use crate::model::{Command, Key};

    fn emitted(commands: &[Command]) -> Vec<(String, String)> {
        files(&plan(commands).unwrap())
    }

    fn text_of(commands: &[Command], name: &str) -> String {
        emitted(commands).into_iter().find(|(file, _)| file == name).map(|(_, text)| text).unwrap_or_default()
    }

    #[test]
    fn keywords_are_escaped_and_the_optionals_case_names_renamed_only_as_enum_values() {
        for word in ["default", "in", "self", "Type", "await", "import"] {
            assert_eq!(Swift.escape(word), format!("`{word}`"));
        }
        for word in ["position", "text", "async", "actor", "open", "none", "some"] {
            assert_eq!(Swift.escape(word), word);
        }
        assert_eq!(case_name(&Swift, "none"), "noneValue");
        assert_eq!(case_name(&Swift, "some"), "someValue");
        assert_eq!(case_name(&Swift, "default"), "`default`");
        assert_eq!(case_name(&Swift, "sameAsLayer"), "sameAsLayer");
    }

    #[test]
    fn a_method_has_the_shape_the_plan_gives() {
        let keys = vec![Key::of("text", false, KeyType::Str), Key::of("effect", true, KeyType::Ref)];
        let text = text_of(&[Command::of("layer.newText", "New Text.", keys)], "LayerCommands.swift");
        for wanted in [
            "extension EffectsEngine.LayerCommands {",
            "    public func newText(\n        text: String? = nil,\n        effect: ItemReference,\n        extra: [String: JSONValue] = [:]\n    ) async throws -> JSONValue {",
            "        try await self.engine.command(\n            \"layer.newText\",\n            [\n                \"text\": text?.jsonValue,\n                \"effect\": effect.jsonValue,\n            ],\n            extra: extra\n        )",
            "    /// New Text.\n",
            "    /// - Parameters:\n    ///   - text: The key `text`: text?\n",
            "    ///   - extra: Keys the typed parameters do not cover",
            "    /// - Returns: The engine's reply.\n    /// - Throws: Whatever the engine refuses the call with.\n",
            "public var layer: EffectsEngine.LayerCommands {\n        EffectsEngine.LayerCommands(engine: self)\n    }",
            "public struct LayerCommands: Sendable {",
            "import CraftBridge",
        ] {
            assert!(text.contains(wanted), "missing {wanted:?} in\n{text}");
        }
        assert!(text.starts_with("// @generated "), "{text}");
    }

    #[test]
    fn a_method_without_keys_takes_only_extra() {
        let text = text_of(&[Command::of("time.forward10", "Forward", vec![])], "TimeCommands.swift");
        assert!(text.contains("    /// - Parameter extra: Keys"), "{text}");
        assert!(text.contains("            \"time.forward10\",\n            [:],\n            extra: extra\n"), "{text}");
    }

    #[test]
    fn every_key_type_has_its_swift_type() {
        let keys = [
            ("a", KeyType::Bool, "Bool"),
            ("b", KeyType::Number, "Double"),
            ("c", KeyType::Str, "String"),
            ("d", KeyType::Vec2, "Vec2"),
            ("e", KeyType::Vec3, "Vec3"),
            ("f", KeyType::Color, "CraftColor"),
            ("g", KeyType::Ref, "ItemReference"),
            ("h", KeyType::Json, "JSONValue"),
            ("i", KeyType::Enum(vec!["x".to_string(), "y".to_string()]), "RunI"),
        ];
        let text =
            text_of(&[Command::of("n.run", "Run", keys.iter().map(|(name, key_type, _)| Key::of(name, false, key_type.clone())).collect())], "NCommands.swift");
        for (name, _, swift) in keys {
            assert!(text.contains(&format!("        {name}: {swift}? = nil")), "{name} {swift} in\n{text}");
        }
    }

    #[test]
    fn an_enum_key_gets_a_value_type_no_switch_can_be_exhaustive_over() {
        let mode = KeyType::Enum(vec!["sameAsLayer".to_string(), "None".to_string(), "default".to_string()]);
        let text = text_of(&[Command::of("layer.mask.motionBlur", "Blur", vec![Key::of("mode", true, mode)])], "LayerMaskCommands.swift");
        for wanted in [
            "    /// The values the engine accepts for the key `mode` of `layer.mask.motionBlur`.\n    public struct MotionBlurMode: Sendable, Hashable, JSONStringRepresentable {",
            "        /// The word the engine takes for this value.\n        public let rawValue: String",
            "        /// The value of a word, one listed here or one the engine added since; the engine checks it.\n        public init(rawValue: String) {",
            "        public init(rawValue: String) {\n            self.rawValue = rawValue\n        }",
            "        /// The value `None`.\n        public static let noneValue = MotionBlurMode(rawValue: \"None\")\n",
            "public static let sameAsLayer = MotionBlurMode(rawValue: \"sameAsLayer\")",
            "public static let `default` = MotionBlurMode(rawValue: \"default\")",
            "        mode: MotionBlurMode,\n",
            "\"mode\": mode.jsonValue\n",
            "extension EffectsEngine.LayerCommands {\n    /// The `layer.mask` commands.\n    public var mask: EffectsEngine.LayerMaskCommands {\n        EffectsEngine.LayerMaskCommands(engine: engine)\n    }",
        ] {
            assert!(text.contains(wanted), "missing {wanted:?} in\n{text}");
        }
        assert!(!text.contains("public enum "), "{text}");
    }

    #[test]
    fn a_reserved_word_labels_a_parameter_in_backticks_and_names_its_value_apart() {
        let text = text_of(&[Command::of("a.b", "B", vec![Key::of("in", false, KeyType::Json), Key::of("repeat", true, KeyType::Bool)])], "ACommands.swift");
        assert!(text.contains("        `in` inValue: JSONValue? = nil,\n        `repeat` repeatValue: Bool,\n"), "{text}");
        assert!(text.contains("\"in\": inValue?.jsonValue,\n                \"repeat\": repeatValue.jsonValue,"), "{text}");
        assert!(text.contains("    ///   - inValue: The key `in`"), "{text}");
    }

    #[test]
    fn two_values_the_swift_renames_into_one_name_are_refused() {
        let mode = KeyType::Enum(vec!["None".to_string(), "noneValue".to_string()]);
        let error = refusal(&[Command::of("layer.mask.spin", "Spin", vec![Key::of("mode", false, mode)])]);
        assert!(
            error.contains("command `layer.mask.spin` key `mode`")
                && error.contains("the value `None`")
                && error.contains("the value `noneValue`")
                && error.contains("Swift name `noneValue`"),
            "{error}"
        );
    }

    #[test]
    fn two_keys_the_swift_binds_into_one_name_are_refused() {
        let keys = vec![Key::of("in", true, KeyType::Json), Key::of("inValue", false, KeyType::Bool)];
        let error = refusal(&[Command::of("edit.paste", "Paste", keys)]);
        assert!(
            error.contains("command `edit.paste`") && error.contains("key `in`") && error.contains("key `inValue`") && error.contains("Swift name `inValue`"),
            "{error}"
        );
        let apart = vec![Key::of("in", true, KeyType::Json), Key::of("at", false, KeyType::Bool)];
        assert!(refusal(&[Command::of("edit.paste", "Paste", apart)]).is_empty(), "keys the renames leave apart pass");
    }

    #[test]
    fn an_enum_key_whose_type_would_take_a_name_the_facade_uses_is_refused() {
        let kind = KeyType::Enum(vec!["layer".to_string(), "comp".to_string()]);
        let error = refusal(&[Command::of("effect.item", "Item", vec![Key::of("reference", false, kind.clone())])]);
        assert!(error.contains("command `effect.item` key `reference`") && error.contains("`ItemReference`"), "{error}");
        // The names the layout writes by hand are weighed too, through ids that would take them: the two
        // words of a value type's name build `EffectsEngine` exactly.
        let error = refusal(&[Command::of("view.effects", "E", vec![Key::of("engine", false, kind.clone())])]);
        assert!(error.contains("`EffectsEngine`"), "{error}");
        // The acronyms are within reach too, through a method that starts with a lower-case letter.
        let error = refusal(&[Command::of("a.jSON", "J", vec![Key::of("value", false, kind.clone())])]);
        assert!(error.contains("`JSONValue`"), "{error}");
        let error = refusal(&[Command::of("a.jSONString", "J", vec![Key::of("representable", false, kind.clone())])]);
        assert!(error.contains("`JSONStringRepresentable`"), "{error}");
        // So is any name of two words whose initials are capitals, not just an acronym: a method `craft`
        // with a key `color` would name one `CraftColor`.
        let error = refusal(&[Command::of("a.craft", "C", vec![Key::of("color", false, kind.clone())])]);
        assert!(error.contains("`CraftColor`"), "{error}");
        for key_type in [KeyType::Bool, KeyType::Number, KeyType::Str, KeyType::Vec2, KeyType::Vec3, KeyType::Color, KeyType::Ref, KeyType::Json] {
            let param = plan(&[Command::of("a.b", "B", vec![Key::of("k", false, key_type)])])
                .unwrap()
                .pop()
                .and_then(|namespace| namespace.methods.into_iter().next())
                .and_then(|method| method.params.into_iter().next())
                .unwrap();
            assert!(NAMED_TYPES.contains(&type_name(&param).as_str()), "{} is named by the facade but not weighed", type_name(&param));
        }
    }

    #[test]
    fn a_namespace_below_another_named_like_the_stored_engine_is_refused() {
        let error = refusal(&[Command::of("view.engine.front", "Front", vec![])]);
        assert!(error.contains("namespace `view.engine`") && error.contains("`engine`"), "{error}");
        // The fold builds the same name from another spelling, so it is refused too: `Engine` and `ENGINE`
        // are the same property of the same parent.
        let caps = refusal(&[Command::of("view.ENGINE.front", "Front", vec![])]);
        assert!(caps.contains("namespace `view.ENGINE`") && caps.contains("`engine`"), "{caps}");
        let word = refusal(&[Command::of("view.Engine.front", "Front", vec![])]);
        assert!(word.contains("namespace `view.Engine`") && word.contains("`engine`"), "{word}");
        // What the engine and the compiler leave apart stays: Swift names a property and a method of one
        // type apart by their full names, so a top-level namespace or a command of the name is no clash.
        let kept = refusal(&[
            Command::of("engine.info", "Info", vec![]),
            Command::of("view.engines.front", "Front", vec![]),
            Command::of("view.engine", "Engine", vec![]),
        ]);
        assert!(kept.is_empty(), "{kept}");
    }

    /// The error [`check`] gives for `commands`, or the empty string when it finds nothing.
    fn refusal(commands: &[Command]) -> String {
        check(&plan(commands).unwrap()).err().unwrap_or_default()
    }

    #[test]
    fn a_namespace_with_no_command_of_its_own_still_has_its_type() {
        let files = emitted(&[Command::of("outer.inner.run", "Run", vec![])]);
        assert_eq!(files.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), ["OuterCommands.swift", "OuterInnerCommands.swift"]);
        let outer = files.first().map(|(_, text)| text.as_str()).unwrap_or_default();
        assert!(outer.contains("public struct OuterCommands") && !outer.contains("public func"), "{outer}");
    }

    #[test]
    fn what_the_engine_writes_cannot_end_a_string_literal_early() {
        assert_eq!(string_literal("a\"b\\c\nd"), "a\\\"b\\\\c\\u{a}d");
        let text = text_of(&[Command::of("a.b\"c", "B", vec![Key::of("k\"", false, KeyType::Json)])], "ACommands.swift");
        assert!(text.contains("\"a.b\\\"c\","), "{text}");
    }

    #[test]
    fn every_namespace_and_command_at_the_pin_is_emitted_once() {
        let commands = crate::surface::commands();
        let namespaces = plan(&commands).unwrap();
        let emitted = files(&namespaces);
        assert_eq!(emitted.len(), namespaces.len());
        let methods: usize = emitted.iter().map(|(_, text)| text.matches("    public func ").count()).sum();
        assert_eq!(methods, commands.len());
        for command in &commands {
            let needle = format!("            \"{}\",\n", command.id);
            assert_eq!(emitted.iter().map(|(_, text)| text.matches(&needle).count()).sum::<usize>(), 1, "{}", command.id);
        }
    }
}
