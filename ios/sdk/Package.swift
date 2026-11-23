// swift-tools-version: 6.0

import PackageDescription

let package = Package(
    name: "CraftBridge",
    platforms: [.iOS(.v17)],
    products: [
        .library(name: "CraftBridge", targets: ["CraftBridge"]),
        .library(name: "CraftBridgeEffects", targets: ["CraftBridgeEffects"]),
    ],
    dependencies: [
        // Reads the conformance corpus, which is TOML, for the tests alone: no target of a product imports it.
        .package(url: "https://github.com/dduan/TOMLDecoder.git", exact: "0.4.5")
    ],
    targets: [
        // The framework is named for its target, never for a crate: which crate is the native library is
        // the layer table's to say (rust/xtask/src/layers.rs), and `cargo xtask xcframework` builds the one
        // it gives. A checkout names the folder it is built into; a release's copy of this manifest names
        // the zip its release serves, and that zip's checksum, in its place.
        .binaryTarget(name: "CraftBridgeFFIBinary", path: "Frameworks/CraftBridgeFFIBinary.xcframework"),
        .target(name: "CraftBridgeFFI", dependencies: ["CraftBridgeFFIBinary"]),
        .target(name: "CraftBridge", dependencies: ["CraftBridgeFFI"]),
        .target(name: "CraftBridgeEffects", dependencies: ["CraftBridge", "CraftBridgeFFI"]),
        // The conformance corpus as Swift reads it, and how a scenario's folder is staged: what both test
        // targets need of the corpus, written once. A test target can share no source with another, and
        // xcodebuild refuses a test target that depends on one, so what they share is a module of its own. It
        // is in no product, so no app links it, and it takes the core alone, so it makes no call of a plugin.
        .target(
            name: "CraftBridgeEffectsConformance",
            dependencies: ["CraftBridge", .product(name: "TOMLDecoder", package: "TOMLDecoder")]),
        .testTarget(
            name: "CraftBridgeEffectsTests",
            dependencies: [
                "CraftBridge", "CraftBridgeEffects", "CraftBridgeEffectsConformance", "CraftBridgeFFI",
                .product(name: "TOMLDecoder", package: "TOMLDecoder"),
            ]),
        // The package as another package sees it, an app's or a binding to another language: the targets of
        // the two products, the corpus's reader, and no package access, so a call of the engine these tests
        // make is a call any package can make, and one they could not make would not compile here.
        .testTarget(
            name: "CraftBridgeEffectsPublicAPITests",
            dependencies: ["CraftBridge", "CraftBridgeEffects", "CraftBridgeEffectsConformance"],
            packageAccess: false),
    ],
    swiftLanguageModes: [.v6]
)
