//! The desktop half of the font listing (AD2): where no platform face list is answered, the
//! listing passes through unfiltered, and the app's own `Folders.fonts` are registered like any
//! other folder. The device answers have their own files, `platform_fonts_ios` and
//! `platform_fonts_android`, run on the simulator and the emulator.

#![cfg(not(any(target_os = "ios", target_os = "android")))]

mod fontlisting;
mod scratch;

use std::sync::{Mutex, MutexGuard, PoisonError};

use craftbridge_core::{CraftApp, CraftError};
use fontlisting::{SCRIPT_RUNNER_REFUSED, TEXT_FONTS, count_agrees, family_names, font_listing, mcp_listings, script_listing};
use scratch::Scratch;
use serde_json::{Value, json};

/// The font registry is one per process and only grows: a test that registers a folder must not
/// do it between another test's two reads of the whole registry, so the tests here take turns.
static REGISTRY: Mutex<()> = Mutex::new(());

fn registry_turn() -> MutexGuard<'static, ()> {
    REGISTRY.lock().unwrap_or_else(PoisonError::into_inner)
}

#[test]
fn the_apps_font_folder_is_registered_and_the_listing_passes_through() {
    let _turn = registry_turn();
    let scratch = fontlisting::scratch_with_bundled_fonts().expect("a scratch folder");
    assert_eq!(scratch.path(), scratch.root().join("container"), "the container sits in the scratch folder it was prepared from");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    let response = font_listing(&mut app).expect("the font listing answers");
    let names = family_names(&response);
    assert!(names.iter().any(|name| name == "Noto Sans SC"), "the bundled Simplified face reports its family: {names:?}");
    assert!(names.iter().any(|name| name == "Noto Sans TC"), "the bundled Traditional face reports its family: {names:?}");
    assert!(names.iter().any(|name| name == "Inter"), "EffectCraft's built-in faces stay listed");
    assert!(count_agrees(&response));
    // No platform answers a face list here, so the listing is the registry's own, untouched.
    assert_eq!(names.len(), effectcraft_text::fonts::families().len(), "the desktop listing passes through unfiltered");
}

#[test]
fn batches_and_command_lists_pass_through_unfiltered_too() {
    let _turn = registry_turn();
    // A plain scratch, with no font folders at all: what passes through is still the whole registry.
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    let full = family_names(&font_listing(&mut app).expect("the font listing answers")).len();

    let batch_params = json!({
        "steps": [
            { "command": TEXT_FONTS },
            { "command": "comp.new", "params": { "name": "C", "width": 100, "height": 100, "duration": 1, "frameRate": 30 } },
        ]
    });
    let batch = app.request("exec", json!({ "command": "engine.batch", "params": batch_params })).expect("the batch answers");
    let results = batch.get("results").and_then(Value::as_array).expect("a batch answers results");
    assert_eq!(results.len(), 2);
    assert_eq!(family_names(&results[0]).len(), full, "the batch's `text.fonts` row is the whole registry here");
    assert!(count_agrees(&results[0]));

    // The script runner would answer from the full registry, and the confined session refuses it
    // outright, in the confinement's words: no script row reaches the app at all.
    match script_listing(&mut app, scratch.path()) {
        Err(CraftError::Unsupported { message }) => assert_eq!(message, SCRIPT_RUNNER_REFUSED),
        other => panic!("a confined session refuses the script runner as Unsupported, got {other:?}"),
    }
}

#[test]
fn the_mcp_tools_list_the_whole_registry_where_no_face_list_is_supplied() {
    let _turn = registry_turn();
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    let full = family_names(&font_listing(&mut app).expect("the font listing answers"));
    let listings = mcp_listings(&mut app).expect("every MCP tool answers a listing");
    assert_eq!(listings.len(), 5, "list_fonts, execute_command, command_run, batch and command_batch each answer one");
    for (tool, listing) in listings {
        assert_eq!(family_names(&listing), full, "`{tool}` lists the registry untouched");
        assert!(count_agrees(&listing), "`{tool}`'s `count` says what its list holds");
    }
}
