import Foundation

public struct PreviewSession: Sendable, Equatable {
  private var urls: [URL] = []
  private var selectedIndex: Int?

  public init() {}

  public var count: Int { urls.count }
  public var currentIndex: Int? { selectedIndex }

  @discardableResult
  public mutating func begin(with catalog: Catalog) -> Bool {
    let nextURLs = catalog.items.map(\.url)
    guard let selectedURL = catalog.selectedURL, let index = nextURLs.firstIndex(of: selectedURL) else {
      end()
      return false
    }
    urls = nextURLs
    selectedIndex = index
    return true
  }

  public mutating func previewItem(at index: Int) -> URL? {
    guard urls.indices.contains(index) else { return nil }
    selectedIndex = index
    return urls[index]
  }

  public mutating func update(with catalog: Catalog) {
    guard let previousURL = selectedIndex.flatMap({ urls.indices.contains($0) ? urls[$0] : nil }) else {
      return
    }

    let nextURLs = catalog.items.map(\.url)
    guard !nextURLs.isEmpty else {
      urls = []
      selectedIndex = nil
      return
    }

    if let index = nextURLs.firstIndex(of: previousURL) {
      urls = nextURLs
      selectedIndex = index
    } else if let selectedURL = catalog.selectedURL, let index = nextURLs.firstIndex(of: selectedURL) {
      urls = nextURLs
      selectedIndex = index
    } else {
      urls = nextURLs
      selectedIndex = min(selectedIndex ?? 0, nextURLs.count - 1)
    }
  }

  public mutating func end() {
    urls = []
    selectedIndex = nil
  }
}
