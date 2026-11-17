//! The iOS half of the font listing (AD2), run on a simulator: every app-facing call lists
//! exactly the families the phone itself lists (`UIFont.familyNames`, read here through the C call
//! it is built on) plus the families the app bundles, with matching `count`; and every face the
//! engine registered — the listed ones and the hidden ones — still draws, which is why the filter
//! trims listings and never the registry.

#![cfg(target_os = "ios")]

mod fontlisting;
mod scratch;

use std::collections::BTreeSet;

use craftbridge_core::{CraftApp, CraftError, platform};
use fontlisting::{SCRIPT_RUNNER_REFUSED, TEXT_FONTS, count_agrees, family_names, font_listing, mcp_listings, script_listing};
use scratch::Scratch;
use serde_json::{Value, json};

/// The families the bundle carries (the two subsets) plus EffectCraft's own statics, which every
/// listing must keep and report `bundled`.
const EXPECTED_BUNDLED: [&str; 5] = ["Inter", "JetBrains Mono", "Noto Serif", "Noto Sans SC", "Noto Sans TC"];

/// Five of the platform's families — the Arabic sets — are named in the C family list in Latin
/// renderings while the phone lists them, and EffectCraft's faces name them, in their own script
/// (V1's check: the app-facing list shows them "under their Arabic names"). The mapping is the
/// platform's own: these are the same five families, named both ways.
const LATIN_RENDERS: [(&str, &str); 5] = [
    ("Al Nile", "\u{627}\u{644}\u{646}\u{64A}\u{644}"),
    ("Damascus", "\u{688}\u{6CC}\u{645}\u{633}\u{6A9}\u{633}"),
    ("Farah", "\u{641}\u{631}\u{62D}"),
    ("Geeza Pro", "\u{6AF}\u{6CC}\u{632}\u{627} \u{67E}\u{631}\u{648}"),
    ("Mishafi", "\u{645}\u{650}\u{635}\u{62D}\u{641}\u{64A}"),
];

/// One `exec` of the app, as the tests call it: the answer stays a `Result` for the `#[test]`
/// function to unwrap, as every helper of a test in this repository does.
fn exec(app: &mut craftbridge_effects::EffectCraftApp, command: &str, params: Value) -> craftbridge_core::Result<Value> {
    app.request("exec", json!({ "command": command, "params": params }))
}

/// The families the acceptance says the app must see: the phone's own (with the five Arabic
/// families spelled as the phone spells them) plus the bundled ones.
fn expected_families() -> BTreeSet<String> {
    let listed = platform::available_family_names();
    assert!(listed.len() >= 86, "iOS lists its families: found {}", listed.len());
    let mut expected: BTreeSet<String> = listed
        .into_iter()
        .map(|name| match LATIN_RENDERS.iter().find(|(latin, _)| *latin == name) {
            Some((_, native)) => (*native).to_string(),
            None => name,
        })
        .collect();
    expected.extend(EXPECTED_BUNDLED.iter().map(|name| (*name).to_string()));
    expected
}

fn listed_families(listing: &Value) -> BTreeSet<String> {
    family_names(listing).into_iter().collect()
}

#[test]
fn the_listing_is_the_phones_own_families_plus_the_bundled() {
    let scratch = fontlisting::scratch_with_bundled_fonts().expect("a scratch folder");
    assert_eq!(scratch.path(), scratch.root().join("container"), "the container sits in the scratch folder it was prepared from");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    let response = font_listing(&mut app).expect("the font listing answers");
    assert_eq!(listed_families(&response), expected_families(), "every app-facing list says what the phone lists, plus what the app bundles");
    assert!(count_agrees(&response), "`count` says what the trimmed list holds");
    for entry in response.get("families").and_then(Value::as_array).expect("families") {
        let origin = entry.get("origin").and_then(Value::as_str).unwrap_or_default();
        assert!(origin == "system" || origin == "bundled", "a kept family reports where it came from, not `{origin}`");
        assert!(entry.get("styles").and_then(Value::as_array).is_some_and(|styles| !styles.is_empty()), "a kept family lists its kept styles");
    }
    for bundled in EXPECTED_BUNDLED {
        let entry =
            response.get("families").and_then(Value::as_array).into_iter().flatten().find(|entry| entry.get("family").and_then(Value::as_str) == Some(bundled));
        assert_eq!(
            entry.and_then(|entry| entry.get("origin").and_then(Value::as_str)),
            Some("bundled"),
            "the bundled families report bundled, as list_fonts promises"
        );
    }
}

#[test]
fn every_app_facing_call_shape_answers_the_same_list() {
    let scratch = fontlisting::scratch_with_bundled_fonts().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    let expected = expected_families();
    assert_eq!(listed_families(&font_listing(&mut app).expect("the font listing answers")), expected);
    let run = app.request("run", json!({ "steps": [{ "command": TEXT_FONTS }] })).expect("`run` answers");
    let run_fonts = run.get(0).and_then(Value::as_object).and_then(|row| row.get("result")).cloned().expect("a step carries its result");
    assert_eq!(listed_families(&run_fonts), expected, "a `run` step lists the same trimmed list");
    assert!(count_agrees(&run_fonts), "a `run` step's `count` says what its trimmed list holds");
    let batch = exec(&mut app, "engine.batch", json!({ "steps": [{ "command": TEXT_FONTS }] })).expect("the batch answers");
    assert_eq!(listed_families(&batch["results"][0]), expected, "an `engine.batch` row lists the same trimmed list");
    assert!(count_agrees(&batch["results"][0]));
    for (tool, listing) in mcp_listings(&mut app).expect("every MCP tool answers a listing") {
        assert_eq!(listed_families(&listing), expected, "the MCP tool `{tool}` lists the same trimmed list");
        assert!(count_agrees(&listing), "the MCP tool `{tool}`'s `count` says what its trimmed list holds");
    }
    // The engine's own script runner would answer from the full registry, and the phone's session
    // is confined from its first command: the confinement refuses `file.runScript` outright, in its
    // own words, so no script row answers the app-facing caller untrimmed.
    match script_listing(&mut app, scratch.path()) {
        Err(CraftError::Unsupported { message }) => assert_eq!(message, SCRIPT_RUNNER_REFUSED),
        other => panic!("a confined session refuses the script runner as Unsupported, got {other:?}"),
    }
}

/// The acceptance's drawing proof: a face the listing never shows is still registered and still
/// chosen when its character comes up — trimming the listings never trimmed the registry. The
/// acceptance's 𞤀 is U+1E900, ADLAM CAPITAL LETTER ALIF, the first letter of the script the
/// hidden Noto Sans Adlam covers.
#[test]
fn hidden_faces_still_draw() {
    use effectcraft_text::fonts;
    // Every face the phone offers must be registered when the fallback answers; building the app
    // is what registers them, whatever order the tests of this file run in. The bundled folder is
    // not needed here: the character must answer from a hidden platform face of its own.
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    let listed = platform::available_family_names();
    let shown = listed_families(&font_listing(&mut app).expect("the font listing answers"));
    let preferred = fonts::resolve("Inter", "Regular").face;
    let alif = char::from_u32(0x1E900).expect("the Adlam capital alif is a character");
    let answer = fonts::face(fonts::fallback_for_locale(alif, preferred, "en-US"));
    eprintln!("of {} registered faces, U+1E900 answers from {} / {}", fonts::all_faces().len(), answer.info.family, answer.info.style);
    assert_eq!(answer.info.family, "Noto Sans Adlam", "𞤀 falls back to the face that covers its script");
    assert!(answer.has_char(alif), "the hidden face draws 𞤀 with a glyph of its own, not a missing-glyph box");
    assert!(!listed.contains(&answer.info.family), "the phone itself does not list {}", answer.info.family);
    assert!(!shown.contains(&answer.info.family), "the app-facing listing does not show {}, and it draws anyway", answer.info.family);
}

/// The bundled subsets are chosen for the two written standards they cover:
/// Simplified characters prefer the Simplified subset, Traditional the Traditional one.
#[test]
fn the_written_standards_choose_their_bundled_faces() {
    use effectcraft_text::fonts;
    let scratch = fontlisting::scratch_with_bundled_fonts().expect("a scratch folder");
    let _app = scratch.app().expect("the app starts from prepared folders");
    let preferred = fonts::resolve("Inter", "Regular").face;
    let hans = char::from_u32(0x6C49).expect("the Simplified hanzi is a character");
    let han_face = fonts::face(fonts::fallback_for_locale(hans, preferred, "zh-Hans"));
    assert_eq!(
        (han_face.info.family.as_str(), han_face.info.style.as_str()),
        ("Noto Sans SC", "Regular"),
        "Simplified Chinese resolves to the bundled Simplified face"
    );
    let hant = char::from_u32(0x6F22).expect("the Traditional hanzi is a character");
    let han_face = fonts::face(fonts::fallback_for_locale(hant, preferred, "zh-Hant"));
    assert_eq!(
        (han_face.info.family.as_str(), han_face.info.style.as_str()),
        ("Noto Sans TC", "Regular"),
        "Traditional Chinese resolves to the bundled Traditional face"
    );
}
