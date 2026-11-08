// swift-tools-version: 6.4
import PackageDescription

let swift6Settings: [SwiftSetting] = [
  .swiftLanguageMode(.v6),
  .enableUpcomingFeature("NonisolatedNonsendingByDefault"),
  .unsafeFlags(["-warnings-as-errors"]),
]

let package = Package(
  name: "view",
  platforms: [.macOS("27.0")],
  products: [
    .executable(name: "view", targets: ["view"])
  ],
  targets: [
    .target(
      name: "viewCore",
      path: "Sources/viewCore",
      swiftSettings: swift6Settings
    ),
    .target(
      name: "viewUI",
      dependencies: ["viewCore"],
      path: "Sources/view",
      swiftSettings: [
        .swiftLanguageMode(.v6),
        .enableUpcomingFeature("NonisolatedNonsendingByDefault"),
        .defaultIsolation(MainActor.self),
        .unsafeFlags(["-warnings-as-errors"]),
      ]
    ),
    .executableTarget(
      name: "view",
      dependencies: ["viewUI"],
      path: "Sources/viewMain",
      swiftSettings: [
        .swiftLanguageMode(.v6),
        .enableUpcomingFeature("NonisolatedNonsendingByDefault"),
        .defaultIsolation(MainActor.self),
        .unsafeFlags(["-warnings-as-errors"]),
      ]
    ),
    .testTarget(
      name: "viewCoreTests",
      dependencies: ["viewCore"],
      path: "Tests/viewCoreTests",
      swiftSettings: swift6Settings
    ),
    .testTarget(
      name: "viewUITests",
      dependencies: ["viewUI", "viewCore"],
      path: "Tests/viewUITests",
      swiftSettings: [
        .swiftLanguageMode(.v6),
        .enableUpcomingFeature("NonisolatedNonsendingByDefault"),
        .defaultIsolation(MainActor.self),
        .unsafeFlags(["-warnings-as-errors"]),
      ]
    ),
  ]
)
