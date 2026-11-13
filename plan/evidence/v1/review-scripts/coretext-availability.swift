import CoreText
import UIKit
// Each line is one candidate API; the compiler reports the ones iOS does not have.
func check() {
    _ = CTFontManagerCopyAvailableFontURLs()
    _ = CTFontManagerCopyAvailablePostScriptNames()
    _ = CTFontManagerCopyAvailableFontFamilyNames()
    _ = CTFontCollectionCreateFromAvailableFonts(nil)
    _ = kCTFontURLAttribute
    let f = CTFontCreateWithName("Helvetica" as CFString, 12, nil)
    _ = CTFontCopyAvailableTables(f, [])
    _ = CTFontCopyTable(f, CTFontTableTag(kCTFontTableHead), [])
    _ = CTFontManagerCreateFontDescriptorsFromData(Data() as CFData)
    _ = UIFont.familyNames
}
