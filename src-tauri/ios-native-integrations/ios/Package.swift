// swift-tools-version:5.3

import PackageDescription

// Same shape as ../../ios-folder-picker-native/ios/Package.swift, whose
// comments explain the `.macOS` platform entry.
let package = Package(
    name: "tine-ios-native-integrations",
    platforms: [
        .macOS(.v10_13),
        .iOS(.v14),
    ],
    products: [
        .library(
            name: "tine-ios-native-integrations",
            type: .static,
            targets: ["tine-ios-native-integrations"]),
    ],
    dependencies: [
        .package(name: "Tauri", path: "../.tauri/tauri-api")
    ],
    targets: [
        .target(
            name: "tine-ios-native-integrations",
            dependencies: [
                .byName(name: "Tauri")
            ],
            path: "Sources")
    ]
)
