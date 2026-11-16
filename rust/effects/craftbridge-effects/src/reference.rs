//! A comp or layer reference read as the command line's `reference` reads its text
//! (`apps/effectcraft-cli/src/main.rs:293-298`), the one reading its `render-frame`, `props`, `get`
//! and `set` give their `--comp` and `--layer`: the methods standing for those commands read their
//! reference here, so each finds what its command finds for the same words.
//!
//! It is not the reading of every command. `render` and `bench` hand their `--comp` text to the engine
//! as it came (`main.rs:906-907`, `:1072-1073`), and the engine takes a string for a name before it
//! tries it as digits, and `-` or an empty string as a name (`crates/engine/src/cmd/params.rs:91-104`):
//! where a comp is named `1`, `render --comp 1` renders that comp and `render-frame --comp 1` the comp
//! whose id is 1. So `export` and `timings` hand the engine their `comp` as given, never through here.

use craftbridge_core::{CraftError, Result};
use serde_json::Value;
use serde_json::map::Map;

/// What a reference's JSON holds, before either rule for its text reads it.
pub(crate) enum Given<'a> {
    /// A whole number.
    Id(u64),
    /// Any text.
    Text(&'a str),
}

/// The one shape check of a comp or layer reference: absent or null is none, a whole number is an
/// id, text is text, and anything else names nothing and is refused, as the caller's to fix. What
/// the text means is each command's own rule: [`reference`] reads it as the command line's
/// `reference` does, and `export` keeps it as given.
pub(crate) fn given<'a>(obj: &'a Map<String, Value>, key: &str, method: &str) -> Result<Option<Given<'a>>> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(id)) => id.as_u64().map(|id| Some(Given::Id(id))).ok_or_else(|| not_a_reference(key, method)),
        Some(Value::String(text)) => Ok(Some(Given::Text(text))),
        Some(_) => Err(not_a_reference(key, method)),
    }
}

fn not_a_reference(key: &str, method: &str) -> CraftError {
    CraftError::invalid_params(format!("`{method}`: `{key}` must be an id, a name or `-`"))
}

/// A comp or layer reference as the command line reads one: a number is an id, a string of digits is
/// the same id, `-` or an empty string names none (the active comp), and any other string is a name
/// or `#n`. An absent or null one is none, `Value::Null`, which the tool reads as the active one.
pub(crate) fn reference(obj: &Map<String, Value>, key: &str, method: &str) -> Result<Value> {
    Ok(match given(obj, key, method)? {
        None => Value::Null,
        Some(Given::Id(id)) => Value::from(id),
        Some(Given::Text("-" | "")) => Value::Null,
        Some(Given::Text(text)) => text.parse::<u64>().map_or_else(|_| Value::from(text), Value::from),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn read(value: Value) -> Result<Value> {
        let obj = json!({ "comp": value });
        reference(obj.as_object().unwrap(), "comp", "frame")
    }

    /// Every spelling the command line's `reference` has a meaning for, and the shapes it has none
    /// for, which are refused in one sentence naming the key and the method.
    #[test]
    fn a_reference_is_read_as_the_command_line_reads_it() {
        assert_eq!(read(json!(7)), Ok(json!(7)));
        assert_eq!(read(json!("7")), Ok(json!(7)), "digits are the id they spell");
        assert_eq!(read(json!("-")), Ok(Value::Null));
        assert_eq!(read(json!("")), Ok(Value::Null));
        assert_eq!(read(Value::Null), Ok(Value::Null));
        assert_eq!(read(json!("Main")), Ok(json!("Main")));
        assert_eq!(read(json!("#2")), Ok(json!("#2")));
        assert_eq!(read(json!("-1")), Ok(json!("-1")), "a signed number is a name");
        // The digits are read as Rust reads a `u64`, which is not the engine's all-digits rule: a
        // leading `+` and leading zeros still spell the id, and a number past `u64` or text around
        // the digits is a name.
        assert_eq!(read(json!("+7")), Ok(json!(7)));
        assert_eq!(read(json!("007")), Ok(json!(7)));
        assert_eq!(read(json!("4294967296")), Ok(json!(4_294_967_296_u64)), "an id is a `u64`, not a `u32`");
        assert_eq!(read(json!("18446744073709551615")), Ok(json!(u64::MAX)));
        assert_eq!(read(json!("18446744073709551616")), Ok(json!("18446744073709551616")));
        assert_eq!(read(json!(" 7")), Ok(json!(" 7")));
        assert_eq!(read(json!("7 ")), Ok(json!("7 ")));
        assert_eq!(reference(&Map::new(), "comp", "frame"), Ok(Value::Null), "absent is none");
        for bad in [json!(1.5), json!(-1), json!(true), json!([1]), json!({"id": 1})] {
            assert_eq!(read(bad.clone()), Err(CraftError::invalid_params("`frame`: `comp` must be an id, a name or `-`")), "{bad}");
        }
    }
}
