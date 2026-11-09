import Foundation
import Testing
import viewCore

@Suite("Root directory resolution")
struct RootDirectoryTests {
  @Test func relativeArgumentResolvesAgainstCurrentDirectory() throws {
    let root = try TestDirectory.make()
    defer { root.remove() }
    let nested = try root.createDirectory("nested")

    let resolved = try RootDirectory.resolve(argument: "nested", currentDirectory: root.url.path)

    #expect(resolved.url.path == nested.url.path)
  }

  @Test func omittedArgumentUsesCurrentDirectory() throws {
    let root = try TestDirectory.make()
    defer { root.remove() }

    let resolved = try RootDirectory.resolve(argument: nil, currentDirectory: root.url.path)

    #expect(resolved.url.path == root.url.path)
  }

  @Test func nonexistentArgumentIsRejected() throws {
    let root = try TestDirectory.make()
    defer { root.remove() }

    #expect(throws: LaunchError.notFound(root.url.appending(path: "missing").path)) {
      try RootDirectory.resolve(argument: "missing", currentDirectory: root.url.path)
    }
  }

  @Test func regularFileArgumentIsRejected() throws {
    let root = try TestDirectory.make()
    defer { root.remove() }
    let file = try root.createFile("note.txt", contents: "x")

    #expect(throws: LaunchError.notDirectory(file.path)) {
      try RootDirectory.resolve(argument: "note.txt", currentDirectory: root.url.path)
    }
  }

  @Test func spacesAndUnicodeArePreserved() throws {
    let root = try TestDirectory.make()
    defer { root.remove() }
    let directory = try root.createDirectory("holiday photos – 2026")

    let resolved = try RootDirectory.resolve(
      argument: "holiday photos – 2026",
      currentDirectory: root.url.path
    )

    #expect(resolved.url.lastPathComponent == "holiday photos – 2026")
    #expect(resolved.url.path == directory.url.path)
  }
}
