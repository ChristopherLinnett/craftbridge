# Bundled font sources

The two region-subset faces the SDK ships, because iOS 26 ships Simplified and Traditional Chinese
in PingFang outlines EffectCraft cannot draw, and the phone's own listing (and drawing) then needs
these (plan §19, AD2). Both files are copied unchanged from the official
[`notofonts/noto-cjk`](https://github.com/notofonts/noto-cjk) at commit
`a99a4354c68964f6bfac488d01010b1fb6d9178a` (the newest commit touching `Sans/`; the bytes also
match the published `Sans2.004` release tag). The repository publishes no checksum files, so the
commit pin plus these SHA-256s are the check, and `LICENSE` is the font's own SIL Open Font License
1.1, shipped beside them as that licence requires. Their author is Adobe, whose copyright both
files carry in their name tables ("Copyright 2014-2021 Adobe (http://www.adobe.com/)"); Noto is a
trademark of Google.

| File | Source | SHA-256 | Licence |
|---|---|---|---|
| `NotoSansSC-Regular.otf` | `Sans/SubsetOTF/SC/NotoSansSC-Regular.otf` | `faa6c9df652116dde789d351359f3d7e5d2285a2b2a1f04a2d7244df706d5ea9` | SIL OFL 1.1 (`LICENSE`) |
| `NotoSansTC-Regular.otf` | `Sans/SubsetOTF/TC/NotoSansTC-Regular.otf` | `5bab0cb3c1cf89dde07c4a95a4054b195afbcfe784d69d75c340780712237537` | SIL OFL 1.1 (`LICENSE`) |
| `LICENSE` | `Sans/LICENSE` | `6a73f9541c2de74158c0e7cf6b0a58ef774f5a780bf191f2d7ec9cc53efe2bf2` | the OFL text itself |

An application ships this folder as one of its `Folders.fonts`; the Swift and Kotlin packages
(IOS-1, AND-1) copy it into the app's container as a bundle resource and an unpacked asset, so the
license travels with the fonts. The SDK lists these families with origin `bundled`.
