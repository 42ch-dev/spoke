// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "spoke",
    products: [
        .library(name: "SpokeConnect", targets: ["SpokeConnect"]),
    ],
    targets: [
        .target(
            name: "SpokeConnect",
            dependencies: ["spoke_connectFFI"],
            path: "crates/spoke-connect/bindings/swift/generated",
            sources: ["spoke_connect.swift"]
        ),
        .binaryTarget(
            name: "spoke_connectFFI",
            path: "crates/spoke-connect/bindings/swift/xcframework/spoke_connectFFI.xcframework"
        ),
    ]
)
