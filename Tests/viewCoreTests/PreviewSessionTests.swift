import Foundation
import Testing
@testable import viewCore

@Suite("PreviewSession")
struct PreviewSessionTests {
  @Test func beginsAtTheCatalogSelection() throws {
    let catalog = try catalog(["one.png", "two.png", "three.png"], selectedIndex: 1)
    var session = PreviewSession()

    let began = session.begin(with: catalog)
    #expect(began)
    #expect(session.count == 3)
    #expect(session.currentIndex == 1)
    #expect(session.previewItem(at: 1) == url("two.png"))
  }

  @Test func advancingRecordsOnlyAnIndexInsideTheSnapshot() throws {
    let catalog = try catalog(["one.png", "two.png"], selectedIndex: 0)
    var session = PreviewSession()
    let began = session.begin(with: catalog)
    #expect(began)

    #expect(session.previewItem(at: 1) == url("two.png"))
    #expect(session.currentIndex == 1)
    #expect(session.previewItem(at: 2) == nil)
    #expect(session.currentIndex == 1)
  }

  @Test func removingBeforeCurrentItemShiftsItsIndexByIdentity() throws {
    var catalog = try catalog(["one.png", "two.png", "three.png"], selectedIndex: 2)
    var session = PreviewSession()
    let began = session.begin(with: catalog)
    #expect(began)
    #expect(session.previewItem(at: 2) == url("three.png"))

    catalog.remove(url("two.png"))
    session.update(with: catalog)

    #expect(session.count == 2)
    #expect(session.currentIndex == 1)
    #expect(session.previewItem(at: 1) == url("three.png"))
    #expect(session.previewItem(at: 2) == nil)
  }

  @Test func removingCurrentItemUsesCatalogSelectionWithinTheNewBounds() throws {
    var catalog = try catalog(["one.png", "two.png", "three.png"], selectedIndex: 2)
    var session = PreviewSession()
    let began = session.begin(with: catalog)
    #expect(began)
    #expect(session.previewItem(at: 2) == url("three.png"))

    catalog.remove(url("three.png"))
    session.update(with: catalog)

    #expect(session.count == 2)
    #expect(session.currentIndex == 1)
    #expect(session.previewItem(at: 1) == url("two.png"))
    #expect(session.previewItem(at: 2) == nil)
  }

  @Test func emptyCatalogHasNoPreviewItem() throws {
    let catalog = try catalog(["one.png"], selectedIndex: 0)
    var session = PreviewSession()
    let began = session.begin(with: catalog)
    #expect(began)
    var emptyCatalog = catalog
    emptyCatalog.remove(url("one.png"))

    session.update(with: emptyCatalog)

    #expect(session.count == 0)
    #expect(session.currentIndex == nil)
    #expect(session.previewItem(at: 0) == nil)
  }

  @Test func noSelectionCannotStartAPreviewSession() throws {
    let catalog = try catalog(["one.png"])
    var session = PreviewSession()

    let began = session.begin(with: catalog)
    #expect(!began)
    #expect(session.count == 0)
    #expect(session.currentIndex == nil)
  }

  private func catalog(_ names: [String], selectedIndex: Int? = nil) throws -> Catalog {
    var catalog = Catalog()
    catalog.append(names.map { name in
      MediaItem(url: url(name), kind: .image, modificationDate: nil, fileSize: nil)
    })
    if let selectedIndex, let item = catalog.item(at: selectedIndex) {
      try catalog.select(item.url)
    }
    return catalog
  }

  private func url(_ name: String) -> URL {
    URL(filePath: "/preview/\(name)")
  }
}
