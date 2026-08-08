// swift-tools-version:5.3

import PackageDescription

let package = Package(
    name: "tine-ios-folder-picker-native",
    platforms: [
        .iOS(.v14),
    ],
    products: [
        .library(
            name: "tine-ios-folder-picker-native",
            type: .static,
            targets: ["tine-ios-folder-picker-native"]),
    ],
    dependencies: [
        .package(name: "Tauri", path: "../.tauri/tauri-api")
    ],
    targets: [
        .target(
            name: "tine-ios-folder-picker-native",
            dependencies: [
                .byName(name: "Tauri")
            ],
            path: "Sources")
    ]
)
