// swift-tools-version:5.9
import PackageDescription

// Local-only iOS functional smoke for the SpokeConnect SPM product.
//
// NOT part of the published consumer manifest: this package is not referenced
// from the repo-root Package.swift and is not shipped to SPM consumers. It
// exists so a maintainer can run the golden-parity checks through an iOS
// simulator slice of the committed spoke_connectFFI.xcframework.
//
// The test target depends on the root `spoke` package by path (the exact
// `SpokeConnect` product a consumer would use) and links the committed
// xcframework through that binary target.

let package = Package(
    name: "IosSmoke",
    platforms: [
        .iOS(.v16),
    ],
    dependencies: [
        .package(name: "spoke", path: "../../../../../"),
    ],
    targets: [
        .testTarget(
            name: "IosSmokeTests",
            dependencies: [
                .product(name: "SpokeConnect", package: "spoke"),
            ],
            resources: [
                .copy("fixtures"),
            ]
        ),
    ]
)
