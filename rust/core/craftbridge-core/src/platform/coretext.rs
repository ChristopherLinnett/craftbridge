//! The font faces CoreText offers this application, read from CoreText itself.
//!
//! An iOS app must not name its system font folders: in the simulator a folder path lands on the
//! Mac's file system, and on a phone a folder walk misses fonts Apple keeps inside private
//! frameworks (PingFang sits in `PrivateFrameworks/FontServices.framework/CorePrivate`). V1
//! measured that the folders CoreText itself reports — the parents of the files of the faces from
//! `CTFontCollectionCreateFromAvailableFonts` — are the only listing that is right in both, and
//! that matching the app-facing list against these faces' (file, PostScript name) pairs gives
//! exactly `UIFont.familyNames` (plan §32 V1). This module is those two calls and nothing else:
//! the faces' (file, PostScript name) pairs, and the family names `CTFontManagerCopyAvailableFontFamilyNames`
//! lists — the same set of families `UIFont.familyNames` returns, with one spelling note: UIKit
//! shows the five Arabic-script families under their Arabic names (النيل, ڈیمسکس, فرح, گیزا پرو,
//! مِصحي) while this C call renders them in Latin ("Al Nile", "Damascus", "Farah", "Geeza Pro",
//! "Mishafi"); the names on the faces themselves are the Arabic ones.
//!
//! The calls go through the `objc2-core-text` and `objc2-core-foundation` bindings, which are
//! generated from the SDK's own headers, so every signature is the header's, and whose retained
//! values release themselves; the frameworks are linked by those crates.

use std::path::PathBuf;

use objc2_core_foundation::{CFArray, CFRetained, CFString, CFType, CFURL};
use objc2_core_text::{CTFontCollection, CTFontDescriptor, CTFontManagerCopyAvailableFontFamilyNames, kCTFontNameAttribute, kCTFontURLAttribute};

/// One face CoreText offers this application: the file holding it, and the PostScript name that
/// names it inside that file. The pair identifies the face EffectCraft's own scan of the same
/// folders finds: the pair is what the font-list filter matches on (plan §19, AD2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreTextFace {
    /// The file CoreText loads the face from, as a plain path.
    pub file: PathBuf,
    /// The face's PostScript name (`kCTFontNameAttribute`).
    pub postscript: String,
}

/// Every face CoreText offers this application, as (file, PostScript name) pairs.
///
/// V1: 303 faces in 103 files on the iOS 26.5 simulator runtime; the files of these faces are the
/// folders the engine must register, and the pairs are the faces an app-facing font list keeps.
/// A face CoreText reports without a usable file or name is skipped — it can match no face of any
/// scan, so it is no part of the listing either way.
#[must_use]
pub fn coretext_faces() -> Vec<CoreTextFace> {
    // SAFETY: no options dictionary is given, so there is none whose keys and values could be of
    // the wrong type; the collection answered is every font available to this application.
    let collection = unsafe { CTFontCollection::from_available_fonts(None) };
    // SAFETY: the collection is a live one CoreText made just above; the call only reads it.
    let Some(descriptors) = (unsafe { collection.matching_font_descriptors() }) else { return Vec::new() };
    objects_of(descriptors).filter_map(|object| object.downcast::<CTFontDescriptor>().ok()).filter_map(|descriptor| face_of(&descriptor)).collect()
}

/// The family names the platform lists for this application — what `UIFont.familyNames` returns,
/// read through the C call UIKit is built on. Sorted as Core Foundation returns them.
#[must_use]
pub fn available_family_names() -> Vec<String> {
    // SAFETY: the call takes nothing and answers an array it owns, which the binding releases.
    let names = unsafe { CTFontManagerCopyAvailableFontFamilyNames() };
    objects_of(names).filter_map(|object| object.downcast::<CFString>().ok()).map(|name| name.to_string()).collect()
}

/// The elements of an array CoreText answered, each a Core Foundation object for the caller to
/// check the type of before it reads one.
fn objects_of(array: CFRetained<CFArray>) -> impl Iterator<Item = CFRetained<CFType>> {
    // SAFETY: CoreText's arrays hold Core Foundation objects alone (descriptors, strings), and
    // every Core Foundation object is a `CFType`; nothing is read as a narrower type without the
    // type check `downcast` makes.
    let objects = unsafe { CFRetained::cast_unchecked::<CFArray<CFType>>(array) };
    objects.into_iter()
}

/// One descriptor's file and PostScript name, if it carries both as the types CoreText documents.
fn face_of(descriptor: &CTFontDescriptor) -> Option<CoreTextFace> {
    // SAFETY: both keys are CoreText's own attribute names, and copying an attribute only reads
    // the descriptor; each answer's type is checked before it is read.
    let (url, name) = unsafe { (descriptor.attribute(kCTFontURLAttribute), descriptor.attribute(kCTFontNameAttribute)) };
    let file = url?.downcast::<CFURL>().ok()?.to_file_path()?;
    let postscript = name?.downcast::<CFString>().ok()?.to_string();
    Some(CoreTextFace { file, postscript })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CoreText answers for the app, on a phone and in the simulator alike (this module is built
    /// for iOS alone): at least the system UI font is there, in a file, with a PostScript name.
    #[test]
    fn coretext_answers_faces_with_files_and_names() {
        let faces = coretext_faces();
        assert!(!faces.is_empty(), "CoreText must offer this process at least one face");
        assert!(faces.iter().all(|face| !face.postscript.is_empty() && face.file.is_absolute()));
    }

    #[test]
    fn coretext_lists_family_names() {
        let names = available_family_names();
        assert!(!names.is_empty(), "CoreText must list at least one family name");
        assert!(names.iter().all(|name| !name.is_empty()));
    }
}
