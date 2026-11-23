//! The facade model: the surface as a facade is planned from it, written out as JSON.
//!
//! `cargo xtask generate` writes this file to
//! `rust/effects/craftbridge-effects-codegen/generated/facade-model.json`. That path is the contract for
//! whoever reads it: a generator for a language this crate holds no emitter of builds its facade from the
//! file there, instead of reading EffectCraft's registry and deciding names for itself. The model is what
//! [`crate::facade::plan`] decides at the pinned revision — every namespace and the names it gives, every
//! method and parameter with its type, enum and words, and every sentence the facades write — with what
//! [`crate::facade`] fixes for every facade alike, the type the SDK defines for a key and the member a
//! namespace's type keeps its engine by, plus what the surface marks each command: `journal`. What the
//! Swift and Kotlin emitters are given and what this file holds are one decision of
//! [`crate::facade::plan`], so they cannot disagree; a test weighs the file against that same plan and
//! against what the emitters write.
//!
//! The shape is the one `plan/contracts/facade-model.schema.json` describes, written in code here and
//! nowhere else, as [`crate::model`] does for the surface file: what the schema requires is required of
//! the text and named when it is missing, and what it leaves optional is absent rather than written null.
//! JSON holds no comment, so — like the surface file — the file carries no `@generated` notice. Unlike the
//! surface file, it names its shape, [`SCHEMA_VERSION`]: its reader is built in another repository, from
//! a commit of its own, and a JSON reader passes over a key it does not know, so a shape it was not written
//! for would be read wrong without a word; the number lets it refuse that shape by name. The surface file
//! names none, since this crate reads every surface it is given, an earlier pin's too, through the
//! defaults of the keys a file lacks.

use crate::facade::{ENGINE, EXTRA, EXTRA_DOC, EnumCase, EnumType, FROM_WORD_DOC, Method, Namespace, Param, RETURNS_DOC, THROWS_DOC, WORD_DOC, sdk_type};
use crate::model::Surface;
use crate::types::KeyType;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The codegen's subcommand that writes this file, as each emitter's module owns its one name for `run`.
pub(crate) const COMMAND: &str = "facade-model";

/// The file this module writes, beside the surface file it plans: one generator run reads the surface
/// once and writes both, and one gate check weighs both against the same regeneration.
pub(crate) const FILE: &str = "facade-model.json";

/// The shape the file is written in, which the schema fixes. A change a reader of the file would read
/// otherwise — a key added, renamed or dropped, or one coming to mean something else — raises it, here
/// and in the schema, so a reader built for an earlier shape refuses the file by this number.
pub(crate) const SCHEMA_VERSION: u32 = 1;

/// The model's text: `namespaces`, the plan of `surface`'s commands, in the shape the schema describes —
/// pretty-printed, objects with their keys in sorted order, ending in one newline, exactly as
/// [`crate::model::Surface::render`] writes the surface. `namespaces` is what [`crate::facade::plan`]
/// answers and what the emitters' file builders take, so a test hands one plan to this and to them and
/// weighs the two against each other.
pub(crate) fn render(surface: &Surface, namespaces: &[Namespace]) -> Result<String, String> {
    let value = serde_json::to_value(FacadeModel::of(surface, namespaces)).map_err(|error| format!("cannot write the facade model as JSON: {error}"))?;
    let mut text = serde_json::to_string_pretty(&value).map_err(|error| format!("cannot write the facade model as JSON: {error}"))?;
    text.push('\n');
    Ok(text)
}

/// The whole model of one pinned engine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FacadeModel {
    schema_version: u32,
    app: String,
    engine_version: String,
    pin: String,
    /// The member every namespace's type keeps its engine by ([`ENGINE`]).
    engine: String,
    extra: String,
    extra_doc: String,
    returns_doc: String,
    throws_doc: String,
    word_doc: String,
    from_word_doc: String,
    /// The type the SDK defines for a key, by the name of the key's type, for each key type it defines one
    /// for ([`sdk_type`]); a key of any other type is passed as the language's own boolean, number or
    /// string, or as its enum type.
    sdk_types: BTreeMap<String, String>,
    namespaces: Vec<ModelNamespace>,
}

/// One command namespace, as [`Namespace`] decides it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelNamespace {
    path: String,
    type_name: String,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent: Option<String>,
    doc: String,
    methods: Vec<ModelMethod>,
    enums: Vec<ModelEnum>,
}

/// One command, as [`Method`] decides it. `journal` is the surface's mark of whether the engine records
/// the command in its journal, the steps it writes out as a replay script; the model writes it because a
/// generated facade of another language has no other way to say which commands the engine journals.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct ModelMethod {
    id: String,
    name: String,
    summary: String,
    detail: String,
    journal: bool,
    params: Vec<ModelParam>,
}

/// One parameter, as [`Param`] decides it. A key's type is written with the name
/// [`crate::types::KeyType::name`] gives it, the one the surface file uses.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct ModelParam {
    key: String,
    name: String,
    required: bool,
    #[serde(rename = "type")]
    key_type: String,
    #[serde(default, rename = "enumName", skip_serializing_if = "Option::is_none")]
    enum_name: Option<String>,
    doc: String,
}

/// The type of an enum key, as [`EnumType`] decides it: `command` is the id of the command whose key it
/// types.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct ModelEnum {
    name: String,
    command: String,
    key: String,
    doc: String,
    cases: Vec<ModelCase>,
}

/// One value of an enum key, as [`EnumCase`] decides it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct ModelCase {
    name: String,
    value: String,
    doc: String,
}

impl FacadeModel {
    /// The model of `surface`, planned into `namespaces`. Every sentence and name the facades write once for
    /// every method or namespace is written here once for the whole model, from the same constants.
    fn of(surface: &Surface, namespaces: &[Namespace]) -> Self {
        FacadeModel {
            schema_version: SCHEMA_VERSION,
            app: surface.app.clone(),
            engine_version: surface.engine_version.clone(),
            pin: surface.pin.clone(),
            engine: ENGINE.to_string(),
            extra: EXTRA.to_string(),
            extra_doc: EXTRA_DOC.to_string(),
            returns_doc: RETURNS_DOC.to_string(),
            throws_doc: THROWS_DOC.to_string(),
            word_doc: WORD_DOC.to_string(),
            from_word_doc: FROM_WORD_DOC.to_string(),
            sdk_types: KeyType::EVERY.iter().filter_map(|key_type| sdk_type(key_type).map(|name| (key_type.name().to_string(), name.to_string()))).collect(),
            namespaces: namespaces.iter().map(ModelNamespace::of).collect(),
        }
    }
}

impl ModelNamespace {
    fn of(namespace: &Namespace) -> Self {
        ModelNamespace {
            path: namespace.path.clone(),
            type_name: namespace.type_name.clone(),
            name: namespace.name.clone(),
            parent: namespace.parent.clone(),
            doc: namespace.doc.clone(),
            methods: namespace.methods.iter().map(ModelMethod::of).collect(),
            enums: namespace.enums.iter().map(ModelEnum::of).collect(),
        }
    }
}

impl ModelMethod {
    fn of(method: &Method) -> Self {
        ModelMethod {
            id: method.id.clone(),
            name: method.name.clone(),
            summary: method.summary.clone(),
            detail: method.detail.clone(),
            journal: method.journal,
            params: method.params.iter().map(ModelParam::of).collect(),
        }
    }
}

impl ModelParam {
    fn of(param: &Param) -> Self {
        ModelParam {
            key: param.key.clone(),
            name: param.name.clone(),
            required: param.required,
            key_type: param.key_type.name().to_string(),
            enum_name: param.enum_name.clone(),
            doc: param.doc.clone(),
        }
    }
}

impl ModelEnum {
    fn of(each: &EnumType) -> Self {
        ModelEnum {
            name: each.name.clone(),
            command: each.id.clone(),
            key: each.key.clone(),
            doc: each.doc.clone(),
            cases: each.cases.iter().map(ModelCase::of).collect(),
        }
    }
}

impl ModelCase {
    fn of(case: &EnumCase) -> Self {
        ModelCase { name: case.name.clone(), value: case.value.clone(), doc: case.doc.clone() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Command, Key};
    use serde_json::Value;
    use std::collections::BTreeSet;

    /// A surface with one enum key, one reference, a namespace below another, and both marks of
    /// journalling, which is what these tests weigh the file against.
    fn fixture() -> Surface {
        let motion_blur = Command::of(
            "layer.mask.motionBlur",
            "Motion Blur.",
            vec![Key::typed("mode", false, &KeyType::Enum(vec!["sameAsLayer".to_string(), "None".to_string()]), "The mode.")],
        );
        Surface::of(
            vec![
                Command::of(
                    "layer.newText",
                    "New Text.",
                    vec![Key::typed("text", true, &KeyType::Str, "The text."), Key::typed("effect", false, &KeyType::Ref, "")],
                ),
                Command { journal: false, ..motion_blur },
            ],
            vec![],
        )
    }

    fn model_of(surface: &Surface) -> FacadeModel {
        FacadeModel::of(surface, &crate::facade::plan(&surface.commands).unwrap())
    }

    fn read(text: &str) -> FacadeModel {
        serde_json::from_str(text).unwrap()
    }

    /// The model's shape, which [`SCHEMA_VERSION`] names, as the schema lists it: at each level of the file,
    /// where the first object of that level stands in a model's text, the keys the schema requires there and
    /// the ones it lets a file leave out. A change to any of these lists is a change of shape, which raises
    /// the version, in the schema as here.
    const SHAPE: [(&[&str], &[&str], &[&str]); 6] = [
        (
            &[],
            &[
                "schemaVersion",
                "app",
                "engineVersion",
                "pin",
                "engine",
                "extra",
                "extraDoc",
                "returnsDoc",
                "throwsDoc",
                "wordDoc",
                "fromWordDoc",
                "sdkTypes",
                "namespaces",
            ],
            &[],
        ),
        (&["namespaces", "0"], &["path", "typeName", "name", "doc", "methods", "enums"], &["parent"]),
        (&["namespaces", "0", "methods", "0"], &["id", "name", "summary", "detail", "journal", "params"], &[]),
        (&["namespaces", "0", "methods", "0", "params", "0"], &["key", "name", "required", "type", "doc"], &["enumName"]),
        (&["namespaces", "0", "enums", "0"], &["name", "command", "key", "doc", "cases"], &[]),
        (&["namespaces", "0", "enums", "0", "cases", "0"], &["name", "value", "doc"], &[]),
    ];

    /// Every object of `value` at the level `path` leads to, an index standing for every item of its array.
    fn every<'a>(value: &'a Value, path: &[&str]) -> Vec<&'a Value> {
        path.iter().fold(vec![value], |nodes, step| {
            nodes
                .into_iter()
                .flat_map(|node| match step.parse::<usize>() {
                    Ok(_) => node.as_array().map(|items| items.iter().collect::<Vec<_>>()).unwrap_or_default(),
                    Err(_) => node.get(*step).into_iter().collect(),
                })
                .collect()
        })
    }

    /// The file holds exactly the shape its version names: every key at every level is one the schema
    /// lists there, and every key it lists there is written, the optional ones for the fixture's namespace
    /// below another and its enum key. A key the serialiser adds, drops or renames fails here, beside the
    /// version such a change raises.
    #[test]
    fn the_file_holds_exactly_the_shape_its_schema_version_names() {
        assert_eq!(SCHEMA_VERSION, 1, "SHAPE is the shape of version 1: a change of shape raises the version, here and in the schema");
        let surface = fixture();
        let value: Value = serde_json::from_str(&render(&surface, &crate::facade::plan(&surface.commands).unwrap()).unwrap()).unwrap();
        for (path, required, optional) in SHAPE {
            let held: BTreeSet<&str> =
                every(&value, path).into_iter().filter_map(Value::as_object).flat_map(|object| object.keys().map(String::as_str)).collect();
            let listed: BTreeSet<&str> = required.iter().chain(optional).copied().collect();
            assert_eq!(held, listed, "the keys written at {path:?} are not the ones the schema lists there");
        }
    }

    #[test]
    fn the_model_is_written_with_its_keys_sorted_and_read_back_whole() {
        let surface = fixture();
        let namespaces = crate::facade::plan(&surface.commands).unwrap();
        let text = render(&surface, &namespaces).unwrap();
        assert!(text.starts_with("{\n  \"app\": \"effectcraft\",\n  \"engine\": \"engine\",\n  \"engineVersion\""), "{text}");
        assert!(text.ends_with("}\n") && !text.ends_with("\n\n"));
        for (before, after) in [("\"detail\"", "\"id\""), ("\"id\"", "\"journal\""), ("\"journal\"", "\"params\""), ("\"params\"", "\"summary\"")] {
            assert!(text.find(before) < text.find(after), "{before} sorts before {after}: {text}");
        }
        assert!(!text.contains(": null"), "a key a namespace or parameter has not is absent, not null: {text}");
        assert_eq!(read(&text), model_of(&surface));
    }

    #[test]
    fn the_shared_sentences_and_names_are_written_once_for_the_whole_model() {
        let model = model_of(&fixture());
        assert_eq!(
            (
                model.schema_version,
                model.app.as_str(),
                model.engine.as_str(),
                model.extra.as_str(),
                model.extra_doc.as_str(),
                model.returns_doc.as_str(),
                model.throws_doc.as_str(),
                model.word_doc.as_str(),
                model.from_word_doc.as_str()
            ),
            (SCHEMA_VERSION, "effectcraft", ENGINE, EXTRA, EXTRA_DOC, RETURNS_DOC, THROWS_DOC, WORD_DOC, FROM_WORD_DOC)
        );
    }

    /// The model names the type each facade passes a key as: a key of every type, planned once, is passed by
    /// the Swift and the Kotlin facade alike as the type the model names for its key type, and a key of a
    /// type the model names none for is one a language passes as its own boolean, number or string, or as
    /// the enum type the model names for that key. A facade generated from the model types each key so.
    #[test]
    fn every_key_the_sdk_types_is_passed_as_the_type_the_model_names_in_both_facades() {
        let keys = KeyType::EVERY
            .iter()
            .map(|key_type| {
                let key_type = if matches!(key_type, KeyType::Enum(_)) { KeyType::Enum(vec!["on".to_string()]) } else { key_type.clone() };
                Key::of(&format!("{}Key", key_type.name()), false, key_type)
            })
            .collect();
        let surface = Surface::of(vec![Command::of("a.every", "Every.", keys)], vec![]);
        let namespaces = crate::facade::plan(&surface.commands).unwrap();
        let model = FacadeModel::of(&surface, &namespaces);
        let swift: String = crate::swift::files(&namespaces).into_iter().map(|(_, text)| text).collect();
        let kotlin: String = crate::kotlin::files(&namespaces).into_iter().map(|(_, text)| text).collect();
        let params = &namespaces.first().unwrap().methods.first().unwrap().params;
        assert_eq!(params.len(), KeyType::EVERY.len(), "a key of every type");
        for param in params {
            match model.sdk_types.get(param.key_type.name()) {
                Some(sdk) => {
                    assert!(swift.contains(&format!("        {}: {sdk}? = nil,\n", param.name)), "the Swift facade passes `{}` as {sdk}:\n{swift}", param.key);
                    assert!(
                        kotlin.contains(&format!("        {}: {sdk}? = null,\n", param.name)),
                        "the Kotlin facade passes `{}` as {sdk}:\n{kotlin}",
                        param.key
                    );
                }
                None => assert!(
                    matches!(param.key_type, KeyType::Bool | KeyType::Number | KeyType::Str | KeyType::Enum(_)),
                    "the model names no type for `{}`, a key no language passes as a type of its own",
                    param.key
                ),
            }
        }
        assert!(model.sdk_types.keys().all(|name| params.iter().any(|param| param.key_type.name() == name)), "{:?}", model.sdk_types);
    }

    #[test]
    fn the_written_model_carries_every_field_the_schema_requires() {
        let text = render(&fixture(), &crate::facade::plan(&fixture().commands).unwrap()).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            (value["schemaVersion"].as_u64(), value["engine"].as_str(), value["sdkTypes"]["json"].as_str()),
            (Some(u64::from(SCHEMA_VERSION)), Some(ENGINE), sdk_type(&KeyType::Json)),
            "the file names its shape, the member that keeps the engine and the SDK's types: {text}"
        );
        let namespaces = value["namespaces"].as_array().unwrap();
        let mask = namespaces.iter().find(|namespace| namespace["path"] == "layer.mask").expect("the planned namespaces are in the file");
        assert_eq!(mask["parent"], "LayerCommands", "a namespace below another says which");
        let method = mask["methods"].as_array().unwrap().first().unwrap();
        assert_eq!(
            (method["id"].as_str(), method["name"].as_str(), method["journal"].as_bool()),
            (Some("layer.mask.motionBlur"), Some("motionBlur"), Some(false))
        );
        assert!(method.get("summary").is_some() && method.get("detail").is_some(), "every method is documented: {text}");
        let param = method["params"].as_array().unwrap().first().unwrap();
        assert_eq!(
            (param["key"].as_str(), param["type"].as_str(), param["enumName"].as_str(), param["required"].as_bool()),
            (Some("mode"), Some("enum"), Some("MotionBlurMode"), Some(false))
        );
        let each = mask["enums"].as_array().unwrap().first().unwrap();
        assert_eq!(
            (each["name"].as_str(), each["command"].as_str(), each["key"].as_str()),
            (Some("MotionBlurMode"), Some("layer.mask.motionBlur"), Some("mode"))
        );
        let case = each["cases"].as_array().unwrap().get(1).unwrap();
        assert_eq!((case["name"].as_str(), case["value"].as_str()), (Some("none"), Some("None")), "the value keeps the engine's spelling");
        assert!(
            namespaces.iter().find(|namespace| namespace["path"] == "layer").unwrap().get("parent").is_none(),
            "a namespace the engine holds names no parent"
        );
    }

    /// The schema's `required` at every level ([`SHAPE`]), on a model text as small as the schema lets one
    /// be: what it requires is required of the text, named when missing, and what it leaves optional reads
    /// without it.
    #[test]
    fn what_the_schema_requires_is_required_and_what_it_leaves_optional_defaults() {
        let minimal = r#"{"schemaVersion":1,"app":"effectcraft","engineVersion":"1","pin":"0123abcd","engine":"engine","extra":"extra","extraDoc":"d","returnsDoc":"d","throwsDoc":"d","wordDoc":"d","fromWordDoc":"d","sdkTypes":{},
"namespaces":[{"path":"layer","typeName":"LayerCommands","name":"layer","doc":"d","methods":[{"id":"l.n","name":"newText","summary":"s","detail":"d","journal":true,
"params":[{"key":"k","name":"k","required":false,"type":"bool","doc":""}]}],
"enums":[{"name":"E","command":"l.n","key":"k","doc":"d","cases":[{"name":"on","value":"on","doc":""}]}]}]}"#;
        let read: FacadeModel = serde_json::from_str(minimal).unwrap();
        assert_eq!(read.namespaces.first().unwrap().parent, None, "a namespace the engine holds has no parent");
        assert_eq!(
            read.namespaces.first().unwrap().methods.first().unwrap().params.first().unwrap().enum_name,
            None,
            "a key that is no enum names no enum type"
        );
        let missing = |text: &str, path: &[&str], key: &str| -> String {
            let mut value: Value = serde_json::from_str(text).unwrap();
            let mut node = &mut value;
            for step in path {
                node = match step.parse::<usize>() {
                    Ok(index) => &mut node[index],
                    Err(_) => &mut node[step],
                };
            }
            node.as_object_mut().unwrap().remove(key);
            serde_json::from_str::<FacadeModel>(&value.to_string()).unwrap_err().to_string()
        };
        for (path, required, _) in SHAPE {
            for key in required {
                let error = missing(minimal, path, key);
                assert!(error.contains(key), "{path:?} with no {key}: {error}");
            }
        }
    }

    /// The acceptance that the model, the Swift facade and the Kotlin facade are one decision: the model is
    /// rendered from the same `facade::plan` value the emitters are handed, and every namespace, method,
    /// parameter, enum and case of it stands in the file once, with the names as the plan gave them — and
    /// every method once in each emitter's text, so none is missing from either side of the comparison.
    #[test]
    fn the_model_is_the_one_plan_the_emitters_are_given() {
        let commands = crate::surface::commands().unwrap();
        let namespaces = crate::facade::plan(&commands).unwrap();
        let surface = Surface::of(commands, vec![]);
        let model = read(&render(&surface, &namespaces).unwrap());
        assert_eq!(model.namespaces.len(), namespaces.len(), "one entry per planned namespace");
        let swift = crate::swift::files(&namespaces);
        let kotlin = crate::kotlin::files(&namespaces);
        assert_eq!((swift.len(), kotlin.len()), (namespaces.len(), namespaces.len()), "one file per namespace, as the model has one entry");
        let swift_text: String = swift.iter().map(|(_, file)| file.as_str()).collect();
        let kotlin_text: String = kotlin.iter().map(|(_, file)| file.as_str()).collect();
        let mut methods = 0;
        for (planned, written) in namespaces.iter().zip(&model.namespaces) {
            assert_eq!(
                (written.path.as_str(), written.type_name.as_str(), written.name.as_str(), written.parent.as_deref(), written.doc.as_str()),
                (planned.path.as_str(), planned.type_name.as_str(), planned.name.as_str(), planned.parent.as_deref(), planned.doc.as_str()),
                "namespace `{}`",
                planned.path
            );
            assert!(swift.iter().any(|(name, _)| *name == format!("{}.swift", planned.type_name)), "no Swift file named for `{}`", planned.type_name);
            assert!(kotlin.iter().any(|(name, _)| *name == format!("{}.kt", planned.type_name)), "no Kotlin file named for `{}`", planned.type_name);
            assert_eq!(written.methods.len(), planned.methods.len(), "namespace `{}` lost or gained a method", planned.path);
            for (method, written_method) in planned.methods.iter().zip(&written.methods) {
                assert_eq!(
                    (
                        written_method.id.as_str(),
                        written_method.name.as_str(),
                        written_method.summary.as_str(),
                        written_method.detail.as_str(),
                        written_method.journal
                    ),
                    (method.id.as_str(), method.name.as_str(), method.summary.as_str(), method.detail.as_str(), method.journal),
                    "method `{}`",
                    method.id
                );
                assert_eq!(written_method.params.len(), method.params.len(), "method `{}` lost or gained a parameter", method.id);
                for (param, written_param) in method.params.iter().zip(&written_method.params) {
                    assert_eq!(
                        (
                            written_param.key.as_str(),
                            written_param.name.as_str(),
                            written_param.required,
                            written_param.key_type.as_str(),
                            written_param.enum_name.as_deref(),
                            written_param.doc.as_str()
                        ),
                        (param.key.as_str(), param.name.as_str(), param.required, param.key_type.name(), param.enum_name.as_deref(), param.doc.as_str()),
                        "key `{}` of `{}`",
                        param.key,
                        method.id
                    );
                }
                methods += 1;
                let call = format!("            \"{}\",\n", method.id);
                assert_eq!(swift_text.matches(&call).count(), 1, "`{}` is typed once in the Swift facade", method.id);
                assert_eq!(kotlin_text.matches(&call).count(), 1, "`{}` is typed once in the Kotlin facade", method.id);
            }
            assert_eq!(written.enums.len(), planned.enums.len(), "namespace `{}` lost or gained an enum", planned.path);
            for (each, written_enum) in planned.enums.iter().zip(&written.enums) {
                assert_eq!(
                    (written_enum.name.as_str(), written_enum.command.as_str(), written_enum.key.as_str(), written_enum.doc.as_str()),
                    (each.name.as_str(), each.id.as_str(), each.key.as_str(), each.doc.as_str()),
                    "enum `{}`",
                    each.name
                );
                assert_eq!(written_enum.cases.len(), each.cases.len(), "enum `{}` lost or gained a case", each.name);
                for (case, written_case) in each.cases.iter().zip(&written_enum.cases) {
                    assert_eq!(
                        (written_case.name.as_str(), written_case.value.as_str(), written_case.doc.as_str()),
                        (case.name.as_str(), case.value.as_str(), case.doc.as_str()),
                        "case `{}` of enum `{}`",
                        case.name,
                        each.name
                    );
                }
            }
        }
        assert_eq!(
            methods,
            crate::facade::callable_at_the_pin(),
            "the model's methods are exactly the commands an app can call, each typed once by each emitter"
        );
    }

    #[test]
    fn every_method_in_the_model_says_what_the_surface_says_about_journalling() {
        let surface = crate::surface::build(crate::surface::TEST_PIN).unwrap();
        let model = read(&render(&surface, &crate::facade::plan(&surface.commands).unwrap()).unwrap());
        let mut journalled = 0;
        let mut methods = 0;
        for namespace in &model.namespaces {
            for method in &namespace.methods {
                methods += 1;
                let command = surface
                    .commands
                    .iter()
                    .find(|command| command.id == method.id)
                    .unwrap_or_else(|| panic!("`{}` is in the model but no command of the surface", method.id));
                assert_eq!(method.journal, command.journal, "`{}` is journalled differently by the model and the surface", method.id);
                if method.journal {
                    journalled += 1;
                }
            }
        }
        assert!(
            journalled > 0 && journalled < methods,
            "the surface at the pin marks some commands journalled and some not, or this test weighs nothing: {journalled} of {methods}"
        );
    }

    #[test]
    fn the_same_surface_writes_the_same_bytes_twice() {
        let surface = crate::surface::build(crate::surface::TEST_PIN).unwrap();
        let once = render(&surface, &crate::facade::plan(&surface.commands).unwrap()).unwrap();
        let twice = render(&surface, &crate::facade::plan(&surface.commands).unwrap()).unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn names_are_written_as_the_facade_decided_them_before_any_language_escapes_them() {
        let surface = Surface::of(
            vec![
                Command::of("layer.default", "Default.", vec![Key::typed("in", false, &KeyType::Str, "")]),
                Command::of("layer.none", "None.", vec![Key::typed("mode", false, &KeyType::Enum(vec!["None".to_string()]), "")]),
            ],
            vec![],
        );
        let text = render(&surface, &crate::facade::plan(&surface.commands).unwrap()).unwrap();
        assert!(text.contains("\"name\": \"default\""), "the method keeps the name the facade gave it: {text}");
        assert!(text.contains("\"name\": \"in\""), "the parameter keeps the name the facade gave it: {text}");
        assert!(text.contains("\"name\": \"none\""), "the enum case keeps the name the facade gave it: {text}");
        assert!(
            !text.contains(": \"`"),
            "no name is written escaped; the engine's own sentences hold backticks, but a language escapes a reserved word itself when it writes one: {text}"
        );
    }

    #[test]
    fn a_key_typed_by_an_enum_names_its_type_and_the_types_are_the_ones_the_schema_allows() {
        let model = model_of(&fixture());
        let mask = model.namespaces.iter().find(|namespace| namespace.path == "layer.mask").unwrap();
        let param = mask.methods.first().unwrap().params.first().unwrap();
        assert_eq!(
            (param.key_type.as_str(), param.enum_name.as_deref()),
            ("enum", Some("MotionBlurMode")),
            "a key's type is the name the surface file spells it by"
        );
        let types: Vec<&str> = model
            .namespaces
            .iter()
            .flat_map(|namespace| namespace.methods.iter())
            .flat_map(|method| method.params.iter().map(|param| param.key_type.as_str()))
            .collect();
        for used in ["enum", "string", "ref"] {
            assert!(types.contains(&used), "the fixture types a key {used}: {types:?}");
        }
        for each in mask.enums.iter().flat_map(|each| each.cases.iter()) {
            assert!(!each.name.is_empty() && !each.value.is_empty(), "every case has its name and the engine's value");
        }
    }
}
