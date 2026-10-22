import Foundation

public struct Catalog: Sendable, Equatable {
  private var storage: [MediaItem] = []
  private var selection: URL?

  public init() {}

  public var items: [MediaItem] { storage }
  public var selectedURL: URL? { selection }
  public var count: Int { storage.count }
  public var isEmpty: Bool { storage.isEmpty }

  public func item(at index: Int) -> MediaItem? {
    guard storage.indices.contains(index) else { return nil }
    return storage[index]
  }

  @discardableResult
  public mutating func append(_ items: [MediaItem]) -> Range<Int> {
    let start = storage.count
    storage.append(contentsOf: items)
    return start..<storage.count
  }

  public mutating func select(_ url: URL) throws {
    guard storage.contains(where: { $0.url == url }) else {
      throw CatalogError.itemNotFound(url)
    }
    selection = url
  }

  public mutating func clearSelection() {
    selection = nil
  }

  @discardableResult
  public mutating func remove(_ url: URL) -> Int? {
    guard let index = storage.firstIndex(where: { $0.url == url }) else { return nil }
    storage.remove(at: index)
    if selection == url {
      selection = index < storage.count ? storage[index].url : storage.last?.url
    }
    return index
  }
}

public enum CatalogError: Error, Equatable, Sendable {
  case itemNotFound(URL)
}

extension CatalogError: LocalizedError {
  public var errorDescription: String? {
    switch self {
    case .itemNotFound(let url): "item is not in the catalog: \(url.path)"
    }
  }
}
