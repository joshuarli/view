import Foundation
import Testing
@testable import viewCore

@Suite("Directory listing")
struct DirectoryListerTests {
  @Test func listsFoldersFirstThenFilesByDescendingSize() async throws {
    let root = try TestDirectory.make()
    defer { root.remove() }
    try createMedia("small.png", contents: "1", in: root.url)
    try createMedia("large.png", contents: "12345", in: root.url)
    try createMedia("clip.mov", contents: "123", in: root.url)
    try createMedia("notes.txt", contents: "123456789", in: root.url)
    try root.createDirectory("zeta")
    try root.createDirectory("alpha")

    let entries = try await DirectoryLister.list(directory: root.url)

    #expect(entries.map(\.url.lastPathComponent) == [
      "alpha", "zeta", "large.png", "clip.mov", "small.png",
    ])
    #expect(entries[0].kind == .directory)
    #expect(entries[0].fileSize == nil)
    #expect(entries[2].kind == .image)
    #expect(entries[3].kind == .movie)
  }

  @Test func breaksSizeTiesWithNaturalNameOrder() async throws {
    let root = try TestDirectory.make()
    defer { root.remove() }
    try createMedia("frame10.png", contents: "1234", in: root.url)
    try createMedia("frame2.png", contents: "1234", in: root.url)

    let entries = try await DirectoryLister.list(directory: root.url)

    #expect(entries.map(\.url.lastPathComponent) == ["frame2.png", "frame10.png"])
  }

  @Test func skipsHiddenPackagesSymbolicLinksAndUnsupportedFiles() async throws {
    let root = try TestDirectory.make()
    defer { root.remove() }
    let visible = try createMedia("visible.png", contents: "12", in: root.url)
    try createMedia(".hidden.png", contents: "12", in: root.url)
    var hidden = try createMedia("attribute-hidden.png", contents: "12", in: root.url)
    var hiddenValues = URLResourceValues()
    hiddenValues.isHidden = true
    try hidden.setResourceValues(hiddenValues)
    try createMedia("bundle.app/inside.png", contents: "12", in: root.url)
    let nested = try root.createDirectory("nested")
    try createMedia("nested/inside.png", contents: "12", in: root.url)
    try createMedia(".secret/nested.png", contents: "12", in: root.url)
    try createMedia("audio.mp3", contents: "12", in: root.url)
    try FileManager.default.createSymbolicLink(
      at: root.url.appending(path: "linked-file.png"),
      withDestinationURL: visible
    )
    try FileManager.default.createSymbolicLink(
      at: root.url.appending(path: "linked-directory"),
      withDestinationURL: nested.url
    )

    let entries = try await DirectoryLister.list(directory: root.url)

    #expect(entries.map(\.url.lastPathComponent) == ["nested", "visible.png"])
    #expect(entries[0].kind == .directory)
  }

  @Test func unknownSizesComeAfterKnownSizes() {
    let unknown = MediaItem(
      url: URL(filePath: "/listing/unknown.png"),
      kind: .image,
      modificationDate: nil,
      fileSize: nil
    )
    let measured = MediaItem(
      url: URL(filePath: "/listing/measured.png"),
      kind: .image,
      modificationDate: nil,
      fileSize: 1
    )

    let sorted = [unknown, measured].sorted(by: DirectoryLister.comesBefore)

    #expect(sorted.map(\.url.lastPathComponent) == ["measured.png", "unknown.png"])
  }

  @Test func skipsAnEntryWhoseMetadataCannotBeRead() async throws {
    let root = try TestDirectory.make()
    defer { root.remove() }
    let unreadable = try createMedia("a-unreadable.png", contents: "1", in: root.url)
    let later = try createMedia("z-later.png", contents: "1", in: root.url)

    let entries = try await DirectoryLister.list(
      directory: root.url,
      readResourceValues: { url, keys in
        if url == unreadable { throw MetadataFailure.denied }
        return try url.resourceValues(forKeys: keys)
      }
    )

    #expect(entries.map(\.url) == [later])
  }

  @Test func emptyDirectoryListsNothing() async throws {
    let root = try TestDirectory.make()
    defer { root.remove() }

    let entries = try await DirectoryLister.list(directory: root.url)

    #expect(entries.isEmpty)
  }

  @Test func unreadableDirectoryFailsWithItsPath() async throws {
    let root = try TestDirectory.make()
    defer { root.remove() }
    try FileManager.default.setAttributes([.posixPermissions: 0], ofItemAtPath: root.url.path)
    defer { try? FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: root.url.path) }

    var observedError: DirectoryScannerError?
    do {
      _ = try await DirectoryLister.list(directory: root.url)
    } catch let error as DirectoryScannerError {
      observedError = error
    } catch {
      Issue.record("Expected a directory scanner error, got \(error)")
    }

    if case .rootUnreadable(let path, _) = observedError {
      #expect(path == root.url.path)
    } else {
      Issue.record("Expected the unreadable root error")
    }
  }

  @discardableResult
  private func createMedia(_ relativePath: String, contents: String, in root: URL) throws -> URL {
    let url = root.appending(path: relativePath)
    try FileManager.default.createDirectory(
      at: url.deletingLastPathComponent(),
      withIntermediateDirectories: true
    )
    try Data(contents.utf8).write(to: url)
    return url
  }
}

private enum MetadataFailure: Error, Sendable {
  case denied
}
