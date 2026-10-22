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

  private func item(_ name: String) -> MediaItem {
    MediaItem(
      url: URL(filePath: "/catalog/\(name)"),
      kind: .image,
      modificationDate: nil,
      fileSize: nil
    )
  }
}
