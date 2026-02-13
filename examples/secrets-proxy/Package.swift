// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "GondolinDemo",
    platforms: [.macOS(.v13)],
    dependencies: [
        .package(name: "AgentKernel", path: "../../sdk/swift"),
    ],
    targets: [
        .executableTarget(
            name: "GondolinDemo",
            dependencies: ["AgentKernel"],
            path: ".",
            sources: ["gondolin_demo.swift"]
        ),
    ]
)
