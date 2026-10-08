// swift-tools-version:5.3

import PackageDescription

let package = Package(
    name: "tine-ios-folder-picker-native",
    // `.macOS` is not vestigial even though this package is iOS-only, but it is
    // also NOT what fixed the original undefined-symbol failure — that was an
    // unreferenced-rlib pruning problem on the Rust side (see the `extern crate`
    // comment in src-tauri/src/ios_folder_picker.rs). Kept because it is correct
    // and matches the working exemplar, not because it was proven load-bearing.
    //
    // Why it is correct: swift-rs cross-compiles by a deliberate hack
    // (swift-rs-1.0.7 src-rs/build.rs:248-291) — it runs a PLAIN `swift build`,
    // which SwiftPM resolves as a *macOS host* build (hence the hardcoded
    // `{arch}-apple-macosx` output directory at :286), redirecting the real
    // target only through `-Xswiftc -target arm64-apple-iosNN.N[-simulator]`.
    // SwiftPM therefore evaluates macOS platform constraints regardless, and the
    // `Tauri` dependency declares `.macOS(.v10_13)`. `tauri-plugin-opener`,
    // which links through this exact path, declares both. Match it.
    platforms: [
        .macOS(.v10_13),
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
