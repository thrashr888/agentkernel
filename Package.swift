// swift-tools-version: 5.9

import PackageDescription

let package = Package(
    name: "AgentKernel",
    platforms: [
        .macOS(.v13),
        .iOS(.v16),
    ],
    products: [
        .library(name: "AgentKernel", targets: ["AgentKernel"]),
    ],
    targets: [
        .target(
            name: "AgentKernel",
            path: "sdk/swift/Sources/AgentKernel"
        ),
        .testTarget(
            name: "AgentKernelTests",
            dependencies: ["AgentKernel"],
            path: "sdk/swift/Tests/AgentKernelTests"
        ),
    ]
)
