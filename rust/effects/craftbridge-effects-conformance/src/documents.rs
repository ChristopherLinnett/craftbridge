//! The JSON documents a side prints, read and compared: a step's output as a list of documents, the
//! scratch folder rewritten to one marker so two folders do not count as a gap, what a comparison sets
//! aside, and equality in which a number is its value.

use crate::corpus::{Entries, Within};
use serde_json::{Number, Value};
use std::path::Path;

/// What the scratch folder is written as, in both sides' output.
pub(crate) const FOLDER_MARKER: &str = "<DIR>";

/// Every document of a step's output, in order: the command line prints one, and `mcp` one per reply,
/// each on a line or across several, which the reader of JSON values takes alike.
pub(crate) fn stream(text: &str) -> Result<Vec<Value>, String> {
    serde_json::Deserializer::from_str(text).into_iter::<Value>().collect::<Result<Vec<_>, _>>().map_err(|error| format!("the output is not JSON: {error}"))
}

/// `text` with every spelling of `folder` written as [`FOLDER_MARKER`]: as handed over, and as the system
/// spells it after resolving links (`/var` is `/private/var` on macOS), which is what the command line
/// prints.
pub(crate) fn without_folder(text: &str, folder: &Path) -> String {
    let mut spellings = vec![folder.display().to_string()];
    if let Ok(real) = folder.canonicalize() {
        spellings.push(real.display().to_string());
    }
    // The longest first, so one spelling that holds the other is never left half rewritten.
    spellings.sort_by_key(|spelling| std::cmp::Reverse(spelling.len()));
    spellings.iter().fold(text.to_string(), |text, spelling| text.replace(spelling.as_str(), FOLDER_MARKER))
}

/// Takes out of `document` what a comparison sets aside.
pub(crate) fn set_aside(document: &mut Value, omit: &[String], entries: &[Entries], within: &[Within]) {
    for rule in within {
        // A string that holds no JSON is left as it is, and compared as the text it is.
        if let Some(Value::String(text)) = document.pointer_mut(&rule.pointer)
            && let Ok(mut inner) = serde_json::from_str::<Value>(text)
        {
            rule.omit.iter().for_each(|pointer| remove(&mut inner, pointer));
            *text = inner.to_string();
        }
    }
    for pointer in omit {
        remove(document, pointer);
    }
    for rule in entries {
        if let Some(Value::Array(items)) = document.pointer_mut(&rule.pointer) {
            let named = |item: &Value| item.get(&rule.key).and_then(Value::as_str).is_some_and(|text| text.starts_with(&rule.prefix));
            for item in items.iter_mut().filter(|item| named(item)) {
                rule.omit.iter().for_each(|pointer| remove(item, pointer));
            }
        }
    }
}

/// Removes the value at `pointer`, if there is one: the parent is found by the pointer's own rules.
fn remove(document: &mut Value, pointer: &str) {
    let Some((parent, key)) = pointer.rsplit_once('/') else { return };
    let key = key.replace("~1", "/").replace("~0", "~");
    match document.pointer_mut(parent) {
        Some(Value::Object(entries)) => {
            entries.remove(&key);
        }
        Some(Value::Array(items)) => {
            if let Some(at) = key.parse::<usize>().ok().filter(|at| *at < items.len()) {
                items.remove(at);
            }
        }
        _ => {}
    }
}

/// Where the actual document first differs from the reference, as a pointer and what each holds (the actual's
/// first); none when they are the same.
pub(crate) fn difference(ours: &Value, theirs: &Value) -> Option<String> {
    differ(ours, theirs, "")
}

fn differ(ours: &Value, theirs: &Value, at: &str) -> Option<String> {
    let shown = |value: &Value| value.to_string().chars().take(160).collect::<String>();
    match (ours, theirs) {
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                return Some(format!("{}: {} items against {}", place(at), a.len(), b.len()));
            }
            a.iter().zip(b).enumerate().find_map(|(index, (a, b))| differ(a, b, &format!("{at}/{index}")))
        }
        (Value::Object(a), Value::Object(b)) => {
            if let Some(key) = a.keys().find(|key| !b.contains_key(*key)) {
                return Some(format!("{}: only the actual has `{key}`", place(at)));
            }
            if let Some(key) = b.keys().find(|key| !a.contains_key(*key)) {
                return Some(format!("{}: only the reference has `{key}`", place(at)));
            }
            a.iter().find_map(|(key, a)| b.get(key).and_then(|b| differ(a, b, &format!("{at}/{}", key.replace('~', "~0").replace('/', "~1")))))
        }
        _ if same(ours, theirs) => None,
        _ => Some(format!("{}: {} against {}", place(at), shown(ours), shown(theirs))),
    }
}

fn place(at: &str) -> &str {
    if at.is_empty() { "the document" } else { at }
}

/// JSON equality in which a number is its value: TOML writes `90` where the oracle prints `90.0`.
pub(crate) fn same(found: &Value, wanted: &Value) -> bool {
    match (found, wanted) {
        (Value::Number(a), Value::Number(b)) => same_number(a, b),
        (Value::Array(a), Value::Array(b)) => a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same(a, b)),
        (Value::Object(a), Value::Object(b)) => a.len() == b.len() && a.iter().all(|(key, a)| b.get(key).is_some_and(|b| same(a, b))),
        _ => found == wanted,
    }
}

/// Two JSON numbers are the same number. serde_json's own `==` is exact between two integers and between two
/// floats, and false across the two; an integer and a float are the same only when the float is integral
/// and is that integer. Rounding the integer to a float instead would make two numbers one past 2^53,
/// where an f64 no longer holds neighbouring integers apart, and pass an expectation against a different
/// number.
fn same_number(a: &Number, b: &Number) -> bool {
    if a == b {
        return true;
    }
    let integer = |n: &Number| n.as_i64().map(i128::from).or_else(|| n.as_u64().map(i128::from));
    // A fraction or an infinity is no integer. An integral float converts exactly while it fits an i128,
    // and `as` saturates one that does not, out of reach of every integer JSON holds.
    let integral = |f: f64| (f.fract() == 0.0).then_some(f as i128);
    match (integer(a), integer(b), a.as_f64(), b.as_f64()) {
        (Some(int), None, _, Some(float)) | (None, Some(int), Some(float), _) => integral(float) == Some(int),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn values_compare_as_json_with_numbers_by_value() {
        let a: Value = json!({"n": [1, 2.0], "s": "x"});
        assert!(same(&a, &json!({"s": "x", "n": [1.0, 2]})));
        assert!(!same(&a, &json!({"s": "x", "n": [1, 3]})));
        assert!(!same(&a, &json!({"s": "x"})));
        // Past what an f64 holds apart, two integers still differ: a float comparison would say no.
        let far = Value::from(9007199254740993_u64);
        assert!(!same(&far, &Value::from(9007199254740992_u64)), "two different integers are never the same number");
        assert!(same(&Value::from(9007199254740993_u64), &Value::from(9007199254740993_u64)));
        // An integer and a float meet only when the float is exactly that integer, far out as near.
        let float = |text: &str| serde_json::from_str::<Value>(text).expect("a JSON float");
        assert!(!same(&far, &float("9007199254740992.0")), "an integer is never a float that is not it");
        assert!(same(&Value::from(9007199254740992_u64), &float("9007199254740992.0")));
        assert!(!same(&Value::from(u64::MAX), &float("18446744073709551616.0")), "u64::MAX is not 2^64");
        assert!(same(&Value::from(-3), &float("-3.0")) && !same(&Value::from(90), &float("90.5")));
        // The reach of the i128 between them: a u64-wide integer meets its own float, and a float past
        // every integer JSON holds saturates out of the reach of any of them. An i64 middle would sink
        // the first and raise the second as a false match on its own largest integer.
        assert!(same(&Value::from(10_000_000_000_000_000_000_u64), &float("1e19")));
        assert!(!same(&Value::from(i64::MAX), &float("1e40")), "a float past i128 is no integer");
    }

    #[test]
    fn a_stream_is_every_document_whatever_its_layout() {
        let docs = stream("{\"a\": 1}\n[2,\n 3]\n\n\"x\"").expect("three documents");
        assert_eq!(docs, [json!({"a": 1}), json!([2, 3]), json!("x")]);
        assert!(stream("{\"a\": 1} not json").is_err());
        assert_eq!(stream("").expect("none"), Vec::<Value>::new());
    }

    #[test]
    fn the_scratch_folder_is_one_marker_in_every_spelling() {
        let scratch = tempfile::tempdir().expect("scratch");
        let real = scratch.path().canonicalize().expect("resolves");
        let text = format!("a {} b {}/x c {}", scratch.path().display(), real.display(), real.display());
        assert_eq!(without_folder(&text, scratch.path()), "a <DIR> b <DIR>/x c <DIR>");
        assert_eq!(without_folder("nothing here", scratch.path()), "nothing here");
    }

    #[test]
    fn what_a_comparison_sets_aside_leaves_the_rest_and_a_missing_value_is_no_error() {
        let mut document = json!({"engine": "x", "project": {"renderer": {"why": "y", "kind": 1}}, "list": [
            {"id": "effect.helpers.a", "enabled": false, "label": "A"}, {"id": "comp.new", "enabled": true}, {"id": 4, "enabled": true}]});
        let entries =
            [Entries { pointer: "/list".into(), key: "id".into(), prefix: "effect.helpers.".into(), omit: vec!["/enabled".into(), "/absent".into()] }];
        set_aside(&mut document, &["/engine".into(), "/project/renderer/why".into(), "/absent/deep".into(), "/list/9".into()], &entries, &[]);
        assert_eq!(
            document,
            json!({"project": {"renderer": {"kind": 1}}, "list": [{"id": "effect.helpers.a", "label": "A"}, {"id": "comp.new", "enabled": true}, {"id": 4, "enabled": true}]}),
            "only the named entries lose the named values, and keep the rest"
        );
        let mut escaped = json!({"a/b": 1, "c~d": 2, "keep": 3});
        set_aside(&mut escaped, &["/a~1b".into(), "/c~0d".into()], &[], &[]);
        assert_eq!(escaped, json!({"keep": 3}));
        // A reply that carries a document as text: the value inside it goes, the rest of the text stays.
        let mut reply = json!({"text": "{\"renderer\":{\"why\":\"y\",\"on\":true},\"n\":1}", "plain": "not json"});
        let within = [
            Within { pointer: "/text".into(), omit: vec!["/renderer/why".into()] },
            Within { pointer: "/plain".into(), omit: vec!["/x".into()] },
            Within { pointer: "/none".into(), omit: vec![] },
        ];
        set_aside(&mut reply, &[], &[], &within);
        assert_eq!(reply, json!({"text": "{\"n\":1,\"renderer\":{\"on\":true}}", "plain": "not json"}));
    }

    #[test]
    fn a_difference_names_where_the_documents_part() {
        let ours = json!({"a": [1, {"b": "x"}], "k": 1});
        assert_eq!(difference(&ours, &json!({"k": 1.0, "a": [1.0, {"b": "x"}]})), None);
        assert_eq!(difference(&ours, &json!({"k": 1, "a": [1, {"b": "y"}]})).as_deref(), Some("/a/1/b: \"x\" against \"y\""));
        assert_eq!(difference(&ours, &json!({"k": 1, "a": [1]})).as_deref(), Some("/a: 2 items against 1"));
        assert_eq!(difference(&ours, &json!({"k": 1})).as_deref(), Some("the document: only the actual has `a`"));
        assert_eq!(difference(&json!({"k": 1}), &ours).as_deref(), Some("the document: only the reference has `a`"));
        assert_eq!(difference(&json!(1), &json!(2)).as_deref(), Some("the document: 1 against 2"));
    }
}
