import Foundation
import Testing
import viewCore

@Suite("Discovery")
struct DiscoveryTests {
  @Test func emptyDirectoryProducesNoBatches() async throws {
    let root = try TestDirectory.make()
    defer { root.remove() }

    let batches = BatchRecorder()
    let summary = try await DirectoryScanner.scan(
      root: RootDirectory(url: root.url),
      batchSink: { await batches.append($0) }
    )
    let delivered = await batches.values

    #expect(delivered.isEmpty)
    #expect(summary.itemCount == 0)
    #expect(summary.batchCount == 0)
    #expect(summary.skippedDirectoryCount == 0)
  }

  @Test func flattensSupportedImagesAndMoviesAndSkipsNoise() async throws {
    let root = try TestDirectory.make()
    defer { root.remove() }
    try createMedia("photo.png", in: root.url)
    try createMedia("clip.mov", in: root.url)
    try createMedia("nested/portrait.jpeg", in: root.url)
    try createMedia("notes.txt", in: root.url)
    try createMedia("unknown", in: root.url)

    let batches = BatchRecorder()
    _ = try await DirectoryScanner.scan(
      root: RootDirectory(url: root.url),
      batchSink: { await batches.append($0) }
    )

    let items = await batches.values.flatMap { $0 }
    #expect(items.map(\.url.lastPathComponent).sorted() == ["clip.mov", "photo.png", "portrait.jpeg"])
    #expect(items.first(where: { $0.url.lastPathComponent == "clip.mov" })?.kind == .movie)
    #expect(items.first(where: { $0.url.lastPathComponent == "photo.png" })?.kind == .image)
  }

  @Test func skipsDotHiddenPackageAndSymbolicLinkEntries() async throws {
    let root = try TestDirectory.make()
    defer { root.remove() }
    let visible = try createMedia("visible.png", in: root.url)
    try createMedia(".hidden.png", in: root.url)
    var hidden = try createMedia("attribute-hidden.png", in: root.url)
    var hiddenValues = URLResourceValues()
    hiddenValues.isHidden = true
    try hidden.setResourceValues(hiddenValues)
    try createMedia("bundle.app/inside.png", in: root.url)
    let nested = try createMedia("nested/inside.png", in: root.url)
    try FileManager.default.createSymbolicLink(
      at: root.url.appending(path: "linked-file.png"),
      withDestinationURL: visible
    )
    try FileManager.default.createSymbolicLink(
      at: root.url.appending(path: "linked-directory"),
      withDestinationURL: root.url.appending(path: "nested")
    )
    try FileManager.default.createSymbolicLink(
      at: root.url.appending(path: "nested/cycle"),
      withDestinationURL: root.url
    )

    let batches = BatchRecorder()
    _ = try await DirectoryScanner.scan(
      root: RootDirectory(url: root.url),
      batchSink: { await batches.append($0) }
    )

    let items = await batches.values.flatMap { $0 }
    #expect(items.map(\.url.path) == [nested.path, visible.path])
  }

  @Test func skipsUnreadableNestedDirectoryAndCountsIt() async throws {
    let root = try TestDirectory.make()
    defer { root.remove() }
    try createMedia("visible.png", in: root.url)
    let unreadable = try root.createDirectory("unreadable")
    try createMedia("unreadable/inside.png", in: root.url)
    try FileManager.default.setAttributes([.posixPermissions: 0], ofItemAtPath: unreadable.url.path)
    defer { try? FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: unreadable.url.path) }

    let batches = BatchRecorder()
    let summary = try await DirectoryScanner.scan(
      root: RootDirectory(url: root.url),
      batchSink: { await batches.append($0) }
    )

    #expect(await batches.values.flatMap { $0 }.map(\.url.lastPathComponent) == ["visible.png"])
    #expect(summary.skippedDirectoryCount == 1)
  }

  @Test func unreadableRootFailsWithItsPath() async throws {
    let root = try TestDirectory.make()
    defer { root.remove() }
    try FileManager.default.setAttributes([.posixPermissions: 0], ofItemAtPath: root.url.path)
    defer { try? FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: root.url.path) }

    var observedError: DirectoryScannerError?
    do {
      _ = try await DirectoryScanner.scan(root: RootDirectory(url: root.url), batchSink: { _ in })
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

  @Test func ordersDirectoryEntriesDepthFirstWithNumericCaseInsensitiveNames() async throws {
    let root = try TestDirectory.make()
    defer { root.remove() }
    try createMedia("A.png", in: root.url)
    try createMedia("a/c.png", in: root.url)
    try createMedia("a/d/1.png", in: root.url)
    try createMedia("frame10.png", in: root.url)
    try createMedia("frame2.png", in: root.url)

    let batches = BatchRecorder()
    _ = try await DirectoryScanner.scan(
      root: RootDirectory(url: root.url),
      batchSink: { await batches.append($0) }
    )

    let paths = await batches.values.flatMap { $0 }.map {
      $0.url.path.replacingOccurrences(of: root.url.path + "/", with: "")
    }
    #expect(paths == [
      "a/c.png", "a/d/1.png", "A.png", "frame2.png", "frame10.png",
    ])
  }

  @Test func preservesDuplicateBasenamesWhitespaceUnicodeAndNewlines() async throws {
    let root = try TestDirectory.make()
    defer { root.remove() }
    try createMedia("one/same.png", in: root.url)
    try createMedia("two/same.png", in: root.url)
    let names = [" with spaces 🧀.png", "line\nbreak.png", "quote'\".png", "e\u{301}.png"]
    for name in names {
      try createMedia(name, in: root.url)
    }

    let batches = BatchRecorder()
    _ = try await DirectoryScanner.scan(
      root: RootDirectory(url: root.url),
      batchSink: { await batches.append($0) }
    )

    let items = await batches.values.flatMap { $0 }
    #expect(items.count == 6)
    #expect(items.filter { $0.url.lastPathComponent == "same.png" }.count == 2)
    #expect(Set(items.map(\.url.lastPathComponent)).isSuperset(of: Set(names + ["same.png"])))
  }

  @Test(arguments: [127, 128, 129])
  func batchesAt128Items(count: Int) async throws {
    let root = try TestDirectory.make()
    defer { root.remove() }
    for index in 0..<count {
      try createMedia("image-\(index).png", in: root.url)
    }

    let batches = BatchRecorder()
    let summary = try await DirectoryScanner.scan(
      root: RootDirectory(url: root.url),
      batchSink: { await batches.append($0) }
    )
    let batchSizes = await batches.values.map(\.count)

    #expect(batchSizes == (count <= 128 ? [count] : [128, count - 128]))
    #expect(summary.itemCount == count)
    #expect(summary.batchCount == batchSizes.count)
  }

  @Test func cancellationAfterFirstBatchStopsFurtherDelivery() async throws {
    let root = try TestDirectory.make()
    defer { root.remove() }
    for index in 0..<300 {
      try createMedia("image-\(index).png", in: root.url)
    }
    let gate = BatchGate()
    let scanTask = Task {
      try await DirectoryScanner.scan(
        root: RootDirectory(url: root.url),
        batchSink: { _ in
          await gate.hold()
        }
      )
    }
    await gate.waitForEntry()
    scanTask.cancel()
    await gate.release()

    await #expect(throws: CancellationError.self) {
      try await scanTask.value
    }
  }

  @discardableResult
  private func createMedia(_ relativePath: String, in root: URL) throws -> URL {
    let url = root.appending(path: relativePath)
    try FileManager.default.createDirectory(
      at: url.deletingLastPathComponent(),
      withIntermediateDirectories: true
    )
    try Data().write(to: url)
    return url
  }
}

private actor BatchRecorder {
  private(set) var values: [[MediaItem]] = []

  func append(_ batch: [MediaItem]) {
    values.append(batch)
  }
}

private actor BatchGate {
  private var isHolding = false
  private var entryContinuation: CheckedContinuation<Void, Never>?
  private var releaseContinuation: CheckedContinuation<Void, Never>?

  func hold() async {
    isHolding = true
    entryContinuation?.resume()
    entryContinuation = nil
    await withCheckedContinuation { releaseContinuation = $0 }
  }

  func waitForEntry() async {
    guard !isHolding else { return }
    await withCheckedContinuation { entryContinuation = $0 }
  }

  func release() {
    releaseContinuation?.resume()
    releaseContinuation = nil
  }
}
