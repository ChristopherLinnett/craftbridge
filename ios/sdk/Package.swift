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
        // Reads the conformance corpus, which is TOML, in the tests alone: no target that ships imports it.
        .package(url: "https://github.com/dduan/TOMLDecoder.git", exact: "0.4.5")
    ],
    targets: [
        // The framework is named for its target, never for a crate: which crate is the native library is
        // the layer table's to say (rust/xtask/src/layers.rs), and `cargo xtask xcframework` builds the one
        // it gives into the folder named here.
        .binaryTarget(name: "CraftBridgeFFIBinary", path: "Frameworks/CraftBridgeFFIBinary.xcframework"),
        .target(name: "CraftBridgeFFI", dependencies: ["CraftBridgeFFIBinary"]),
        .target(name: "CraftBridge", dependencies: ["CraftBridgeFFI"]),
        .target(name: "CraftBridgeEffects", dependencies: ["CraftBridge", "CraftBridgeFFI"]),
        .testTarget(
            name: "CraftBridgeEffectsTests",
            dependencies: [
                "CraftBridge", "CraftBridgeEffects", "CraftBridgeFFI",
                .product(name: "TOMLDecoder", package: "TOMLDecoder"),
            ]),
    ],
    swiftLanguageModes: [.v6]
)
