import Foundation
import Testing
import viewCore

@Suite("Launch options")
struct LaunchOptionsTests {
  @Test func defaultsToBrowsingTheCurrentDirectory() throws {
    let root = try TestDirectory.make()
    defer { root.remove() }

    let options = try LaunchOptions.resolve(arguments: [], currentDirectory: root.url.path)

    #expect(options.mode == .browser)
    #expect(options.root.url.path == root.url.path)
  }

  @Test func recursiveFlagSelectsTheFlattenedGrid() throws {
    let root = try TestDirectory.make()
    defer { root.remove() }

    for flag in ["--recursive", "-r"] {
      let options = try LaunchOptions.resolve(arguments: [flag], currentDirectory: root.url.path)
      #expect(options.mode == .recursive)
      #expect(options.root.url.path == root.url.path)
    }
  }

  @Test func acceptsTheDirectoryBeforeOrAfterTheFlag() throws {
    let root = try TestDirectory.make()
    defer { root.remove() }
    let nested = try root.createDirectory("photos")

    for arguments in [["photos", "--recursive"], ["--recursive", "photos"]] {
      let options = try LaunchOptions.resolve(arguments: arguments, currentDirectory: root.url.path)
      #expect(options.mode == .recursive)
      #expect(options.root.url.path == nested.url.path)
    }
  }

  @Test func directoryAfterDoubleDashMayStartWithADash() throws {
    let root = try TestDirectory.make()
    defer { root.remove() }
    let nested = try root.createDirectory("-photos")

    let options = try LaunchOptions.resolve(
      arguments: ["--", "-photos"],
      currentDirectory: root.url.path
    )

    #expect(options.mode == .browser)
    #expect(options.root.url.path == nested.url.path)
  }

  @Test func rejectsUnknownOptions() {
    #expect(throws: LaunchError.unknownOption("--flat")) {
      try LaunchOptions.resolve(arguments: ["--flat"], currentDirectory: "/")
    }
  }

  @Test func rejectsMoreThanOneDirectory() throws {
    let root = try TestDirectory.make()
    defer { root.remove() }

    #expect(throws: LaunchError.unexpectedArgument("second")) {
      try LaunchOptions.resolve(
        arguments: ["first", "second"],
        currentDirectory: root.url.path
      )
    }
  }
}
