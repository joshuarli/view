import Foundation
import Testing
@testable import viewCore

@Suite("Trash")
@MainActor
struct TrashTests {
  @Test func catalogRemovalWaitsForRecycleMapping() async throws {
    var catalog = try catalog(["one.png", "two.png"], selectedIndex: 0)
    let service = TrashService()
    let recycle = RecycleHarness()
    let source = url("one.png")
    let attempt = Task {
      try await service.move(
        source,
        recycle: { try await recycle.recycle($0) },
        removeFromCatalog: { _ = catalog.remove($0) }
      )
    }

    await recycle.waitUntilStarted()
    #expect(catalog.count == 2)
    #expect(catalog.selectedURL == source)

    await recycle.complete(source, with: [source: url("one-trashed.png")])
    let result = try await attempt.value

    #expect(result == .removed)
    #expect(catalog.count == 1)
    #expect(catalog.selectedURL == url("two.png"))
  }

  @Test func thrownRecycleFailureLeavesCatalogAndSelectionUnchanged() async throws {
    var catalog = try catalog(["one.png", "two.png"], selectedIndex: 0)
    let service = TrashService()

    do {
      _ = try await service.move(
        url("one.png"),
        recycle: { _ in throw RecycleFailure.failed },
        removeFromCatalog: { _ = catalog.remove($0) }
      )
      Issue.record("Expected recycle failure")
    } catch {
      #expect(error is RecycleFailure)
    }

    #expect(catalog.count == 2)
    #expect(catalog.selectedURL == url("one.png"))
  }

  @Test func missingRecycleMappingLeavesCatalogAndSelectionUnchanged() async throws {
    var catalog = try catalog(["one.png", "two.png"], selectedIndex: 0)
    let service = TrashService()

    do {
      _ = try await service.move(
        url("one.png"),
        recycle: { _ in [:] },
        removeFromCatalog: { _ = catalog.remove($0) }
      )
      Issue.record("Expected missing recycle mapping")
    } catch {
      #expect(error as? TrashError == .missingRecycleMapping(url("one.png")))
    }

    #expect(catalog.count == 2)
    #expect(catalog.selectedURL == url("one.png"))
  }

  @Test func duplicateAttemptForSameURLIsRefusedWhileInFlight() async throws {
    var catalog = try catalog(["one.png"], selectedIndex: 0)
    let service = TrashService()
    let recycle = RecycleHarness()
    let source = url("one.png")
    let attempt = Task {
      try await service.move(
        source,
        recycle: { try await recycle.recycle($0) },
        removeFromCatalog: { _ = catalog.remove($0) }
      )
    }

    await recycle.waitUntilStarted()
    let duplicate = try await service.move(
      source,
      recycle: { _ in Issue.record("Duplicate recycle closure must not run"); return [:] },
      removeFromCatalog: { _ = catalog.remove($0) }
    )
    await recycle.complete(source, with: [source: url("one-trashed.png")])
    let result = try await attempt.value

    #expect(duplicate == .alreadyInProgress)
    #expect(result == .removed)
    #expect(catalog.isEmpty)
  }

  @Test func removingSelectedItemsKeepsNextThenPreviousSelection() async throws {
    var catalog = try catalog(["one.png", "two.png", "three.png"], selectedIndex: 1)
    let service = TrashService()
    let recycle: TrashService.Recycle = { source in [source: url("trashed-\(source.lastPathComponent)")] }

    let first = try await service.move(
      url("two.png"),
      recycle: recycle,
      removeFromCatalog: { _ = catalog.remove($0) }
    )
    let second = try await service.move(
      url("three.png"),
      recycle: recycle,
      removeFromCatalog: { _ = catalog.remove($0) }
    )

    #expect(first == .removed)
    #expect(second == .removed)
    #expect(catalog.count == 1)
    #expect(catalog.selectedURL == url("one.png"))
  }

  @Test func removingLastCatalogItemLeavesNoSelection() async throws {
    var catalog = try catalog(["one.png"], selectedIndex: 0)
    let service = TrashService()

    let result = try await service.move(
      url("one.png"),
      recycle: { source in [source: url("one-trashed.png")] },
      removeFromCatalog: { _ = catalog.remove($0) }
    )

    #expect(result == .removed)
    #expect(catalog.isEmpty)
    #expect(catalog.selectedURL == nil)
  }

  @Test func unusualFilenameSurvivesPreviewAndTrashIdentity() async throws {
    let filename = "line\nbreak 'quote' \"double\" 🧀 e\u{301}.png"
    let source = url(filename)
    var catalog = Catalog()
    catalog.append([MediaItem(url: source, kind: .image, modificationDate: nil, fileSize: nil)])
    try catalog.select(source)
    var preview = PreviewSession()
    let began = preview.begin(with: catalog)
    #expect(began)
    #expect(preview.previewItem(at: 0) == source)
    let service = TrashService()

    let result = try await service.move(
      source,
      recycle: { moved in
        #expect(moved == source)
        return [moved: self.url("trashed-\(filename)")]
      },
      removeFromCatalog: { moved in _ = catalog.remove(moved) }
    )

    #expect(result == .removed)
    #expect(catalog.isEmpty)
  }

  private func catalog(_ names: [String], selectedIndex: Int) throws -> Catalog {
    var catalog = Catalog()
    catalog.append(names.map { name in
      MediaItem(url: url(name), kind: .image, modificationDate: nil, fileSize: nil)
    })
    guard let item = catalog.item(at: selectedIndex) else {
      Issue.record("Selected fixture index must exist")
      return catalog
    }
    try catalog.select(item.url)
    return catalog
  }

  private func url(_ name: String) -> URL {
    URL(filePath: "/trash/\(name)")
  }
}

private enum RecycleFailure: Error {
  case failed
}

private actor RecycleHarness {
  private var continuations: [URL: CheckedContinuation<[URL: URL], any Error>] = [:]
  private var startedURLs: [URL] = []
  private var startWaiters: [CheckedContinuation<Void, Never>] = []

  func recycle(_ url: URL) async throws -> [URL: URL] {
    try await withCheckedThrowingContinuation { continuation in
      continuations[url] = continuation
      startedURLs.append(url)
      let waiters = startWaiters
      startWaiters.removeAll()
      waiters.forEach { $0.resume() }
    }
  }

  func waitUntilStarted() async {
    guard startedURLs.isEmpty else { return }
    await withCheckedContinuation { startWaiters.append($0) }
  }

  func complete(_ url: URL, with mapping: [URL: URL]) {
    continuations.removeValue(forKey: url)?.resume(returning: mapping)
  }
}
