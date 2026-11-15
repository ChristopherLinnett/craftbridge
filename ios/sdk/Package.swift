// swift-tools-version: 6.0

import PackageDescription

let package = Package(
    name: "CraftBridge",
    platforms: [.iOS(.v17)],
    products: [
        .library(name: "CraftBridge", targets: ["CraftBridge"]),
        .library(name: "CraftBridgeEffects", targets: ["CraftBridgeEffects"]),
    ],
    targets: [
        .target(name: "CraftBridge"),
        .target(name: "CraftBridgeEffects", dependencies: ["CraftBridge"]),
        .testTarget(name: "CraftBridgeEffectsTests", dependencies: ["CraftBridge", "CraftBridgeEffects"]),
    ],
    swiftLanguageModes: [.v6]
)
