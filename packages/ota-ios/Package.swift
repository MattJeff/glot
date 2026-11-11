// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "Glot",
    platforms: [.iOS(.v15), .macOS(.v12), .tvOS(.v15), .watchOS(.v8)],
    products: [.library(name: "Glot", targets: ["Glot"])],
    targets: [
        .target(name: "Glot"),
        .testTarget(name: "GlotTests", dependencies: ["Glot"]),
    ]
)
