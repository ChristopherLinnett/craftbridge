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

/// A comp or layer reference as the command line reads one: a number is an id, a string of digits is
/// the same id, `-` or an empty string names none (the active comp), and any other string is a name
/// or `#n`. An absent or null one is none, `Value::Null`, which the tool reads as the active one.
/// Anything else names nothing and is refused, as the caller's to fix.
pub(crate) fn reference(obj: &Map<String, Value>, key: &str, method: &str) -> Result<Value> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(Value::Null),
        Some(Value::Number(id)) if id.is_u64() => Ok(Value::Number(id.clone())),
        Some(Value::String(text)) if text == "-" || text.is_empty() => Ok(Value::Null),
        Some(Value::String(text)) => Ok(text.parse::<u64>().map_or_else(|_| Value::from(text.as_str()), Value::from)),
        Some(_) => Err(CraftError::invalid_params(format!("`{method}`: `{key}` must be an id, a name or `-`"))),
    }
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
