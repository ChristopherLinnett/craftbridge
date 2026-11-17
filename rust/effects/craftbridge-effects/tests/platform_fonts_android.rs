//! The Android half of the font listing (AD2), run on an emulator: `/system/fonts` is registered
//! at engine start and its families show in the listing, which passes through unfiltered (Android
//! answers no face list), and a `Folders.fonts` folder an app carries arrives beside them.

#![cfg(target_os = "android")]

mod fontlisting;
mod scratch;

use std::path::PathBuf;

use craftbridge_core::CraftError;
use fontlisting::{SCRIPT_RUNNER_REFUSED, count_agrees, family_names, font_listing, mcp_listings, script_listing};
use scratch::Scratch;

/// Where the run-script harness left the bundled fonts on the device (the emulator route the
/// acceptance names: `adb push` of the crate's `assets/fonts`, pointed at the test by this
/// variable); a device app would carry them the same way, unpacked into its container.
fn fonts_dir_from_environment() -> Option<PathBuf> {
    std::env::var_os("CB_FX3_FONTS").map(PathBuf::from).filter(|path| path.is_dir())
}

#[test]
fn the_emulator_lists_the_system_families() {
    let fonts: Vec<PathBuf> = fonts_dir_from_environment().into_iter().collect();
    let scratch = fontlisting::scratch_with_fonts(fonts).expect("a scratch folder");
    assert_eq!(scratch.path(), scratch.root().join("container"), "the container sits in the scratch folder it was prepared from");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    let listing = font_listing(&mut app).expect("the font listing answers");
    let names = family_names(&listing);
    assert!(names.iter().any(|name| name == "Roboto"), "the system families show: {names:?}");
    assert!(count_agrees(&listing));
    // Android answers no face list: the listing is the registry's own, untouched, families and all.
    assert_eq!(names.len(), effectcraft_text::fonts::families().len(), "the listing passes through unfiltered");
    if !scratch.folders.fonts.is_empty() {
        assert!(names.iter().any(|name| name == "Noto Sans SC"), "the app's own folder arrived beside the system's: {names:?}");
        assert!(names.iter().any(|name| name == "Noto Sans TC"), "both bundled subsets: {names:?}");
    }
}

#[test]
fn an_app_with_no_font_folder_still_lists_the_system() {
    // The plain scratch: only `/system/fonts`, nothing the app carries. Registration and the
    // unfiltered listing are the platform's own doing, independent of any bundled folder.
    let scratch = Scratch::new().expect("a scratch folder");
    let mut app = scratch.app().expect("the app starts from prepared folders");
    let listing = font_listing(&mut app).expect("the font listing answers");
    assert!(family_names(&listing).iter().any(|name| name == "Roboto"), "the system families show with no app folders: {listing:?}");
    assert!(count_agrees(&listing));
    for (tool, mcp) in mcp_listings(&mut app).expect("every MCP tool answers a listing") {
        assert_eq!(family_names(&mcp), family_names(&listing), "the MCP tool `{tool}` lists the registry untouched too");
    }
    // The confined session refuses the script runner on the phone too, in the confinement's words.
    match script_listing(&mut app, scratch.path()) {
        Err(CraftError::Unsupported { message }) => assert_eq!(message, SCRIPT_RUNNER_REFUSED),
        other => panic!("a confined session refuses the script runner as Unsupported, got {other:?}"),
    }
}
