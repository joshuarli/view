import Foundation
import Testing
import viewCore

@Suite("Catalog")
struct CatalogTests {
  @Test func appendingBatchesPreservesDeliveredOrderAndSelection() throws {
    var catalog = Catalog()
    let first = item("first.png")
    let second = item("second.png")
    let third = item("third.png")

    let firstRange = catalog.append([first, second])
    try catalog.select(second.url)
    let secondRange = catalog.append([third])

    #expect(firstRange == 0..<2)
    #expect(secondRange == 2..<3)
    #expect(catalog.items == [first, second, third])
    #expect(catalog.selectedURL == second.url)
  }

  @Test func insertingRestoresOrderAndKeepsSelection() throws {
    var catalog = Catalog()
    let first = item("first.png")
    let third = item("third.png")
    catalog.append([first, third])
    try catalog.select(third.url)

    let second = item("second.png")
    #expect(catalog.insert(second, at: 1) == 1)
    #expect(catalog.items == [first, second, third])
    #expect(catalog.selectedURL == third.url)
  }

  @Test func insertingClampsToBounds() {
    var catalog = Catalog()
    let first = item("first.png")
    catalog.append([first])

    #expect(catalog.insert(item("before.png"), at: -10) == 0)
    #expect(catalog.insert(item("after.png"), at: 100) == 2)
    #expect(catalog.items.map(\.url.lastPathComponent) == ["before.png", "first.png", "after.png"])
  }

  @Test func removingSelectedItemChoosesTheNextThenPreviousItem() throws {
    var catalog = Catalog()
    let items = [item("first.png"), item("second.png"), item("third.png")]
    catalog.append(items)

    try catalog.select(items[1].url)
    #expect(catalog.remove(items[1].url) == 1)
    #expect(catalog.selectedURL == items[2].url)

    #expect(catalog.remove(items[2].url) == 1)
    #expect(catalog.selectedURL == items[0].url)
  }

  @Test func removingLastItemLeavesAnEmptyCatalog() throws {
    var catalog = Catalog()
    let onlyItem = item("only.png")
    catalog.append([onlyItem])
    try catalog.select(onlyItem.url)

    #expect(catalog.remove(onlyItem.url) == 0)
    #expect(catalog.isEmpty)
    #expect(catalog.selectedURL == nil)
  }

  @Test func removingAnUnrelatedItemKeepsSelectionIdentity() throws {
    var catalog = Catalog()
    let items = [item("first.png"), item("second.png"), item("third.png")]
    catalog.append(items)
    try catalog.select(items[2].url)

    #expect(catalog.remove(items[0].url) == 0)
    #expect(catalog.selectedURL == items[2].url)
    #expect(catalog.items == [items[1], items[2]])
  }

  @Test func rejectsSelectionOutsideCatalog() {
    var catalog = Catalog()
    let missing = item("missing.png")

    #expect(throws: CatalogError.itemNotFound(missing.url)) {
      try catalog.select(missing.url)
    }
    #expect(catalog.selectedURL == nil)
  }

  @Test func additionalSelectionKeepsTheOriginalAnchor() throws {
    var catalog = Catalog()
    let items = [item("first.png"), item("second.png"), item("third.png")]
    catalog.append(items)

    try catalog.select(items[0].url)
    try catalog.selectAdditional(items[1].url)
    try catalog.selectAdditional(items[2].url)
    try catalog.selectAdditional(items[1].url)

    #expect(catalog.selectedURL == items[0].url)
    #expect(catalog.selectedURLs == [items[0].url, items[1].url, items[2].url])
  }

  @Test func rejectsAdditionalSelectionOutsideCatalog() throws {
    var catalog = Catalog()
    catalog.append([item("first.png")])
    try catalog.select(item("first.png").url)
    let missing = item("missing.png")

    #expect(throws: CatalogError.itemNotFound(missing.url)) {
      try catalog.selectAdditional(missing.url)
    }
    #expect(catalog.selectedURLs == [item("first.png").url])
  }

  @Test func deselectingTheAnchorPromotesTheNextSelection() throws {
    var catalog = Catalog()
    let items = [item("first.png"), item("second.png"), item("third.png")]
    catalog.append(items)
    try catalog.select(items[0].url)
    try catalog.selectAdditional(items[1].url)

    catalog.deselect(items[0].url)

    #expect(catalog.selectedURL == items[1].url)
    #expect(catalog.selectedURLs == [items[1].url])
  }

  @Test func removingARangeLeavesTheNeighborSelected() throws {
    var catalog = Catalog()
    let items = [item("first.png"), item("second.png"), item("third.png"), item("fourth.png")]
    catalog.append(items)
    try catalog.select(items[1].url)
    try catalog.selectAdditional(items[2].url)

    #expect(catalog.remove(items[1].url) == 1)
    #expect(catalog.selectedURL == items[2].url)

    #expect(catalog.remove(items[2].url) == 1)
    #expect(catalog.selectedURL == items[3].url)
    #expect(catalog.items == [items[0], items[3]])
  }

  @Test func rangeSelectionKeepsTheAnchorFirst() throws {
    var catalog = Catalog()
    let items = [item("first.png"), item("second.png"), item("third.png"), item("fourth.png")]
    catalog.append(items)

    try catalog.selectRange(anchor: items[1].url, clicked: items[3].url)

    #expect(catalog.selectedURL == items[1].url)
    #expect(catalog.selectedURLs == [items[1].url, items[2].url, items[3].url])
  }

  @Test func rangeSelectionExtendsUpwardWithTheAnchorFirst() throws {
    var catalog = Catalog()
    let items = [item("first.png"), item("second.png"), item("third.png")]
    catalog.append(items)

    try catalog.selectRange(anchor: items[2].url, clicked: items[0].url)

    #expect(catalog.selectedURL == items[2].url)
    #expect(catalog.selectedURLs == [items[2].url, items[0].url, items[1].url])
  }

  @Test func rangeSelectionOutsideCatalogThrows() {
    var catalog = Catalog()
    catalog.append([item("first.png")])

    #expect(throws: CatalogError.itemNotFound(item("missing.png").url)) {
      try catalog.selectRange(anchor: item("missing.png").url, clicked: item("first.png").url)
    }
    #expect(throws: CatalogError.itemNotFound(item("missing.png").url)) {
      try catalog.selectRange(anchor: item("first.png").url, clicked: item("missing.png").url)
    }
    #expect(catalog.selectedURL == nil)
  }

  private func item(_ name: String) -> MediaItem {
    MediaItem(
      url: URL(filePath: "/catalog/\(name)"),
      kind: .image,
      modificationDate: nil,
      fileSize: nil
    )
  }
}
