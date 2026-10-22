import CoreGraphics
import Foundation

@MainActor
public final class ThumbnailCache {
  public static let defaultTotalCostLimitBytes = 96 * 1024 * 1024

  private let storage = NSCache<ThumbnailCacheObjectKey, CGImage>()
  private let totalCostLimit: Int
  private var entries: [ThumbnailKey: CacheEntry] = [:]
  private var insertionOrder: [CacheOrderEntry] = []
  private var nextEvictionIndex = 0
  private var nextSequence: UInt64 = 0
  private var trackedCostBytes = 0

  public convenience init() {
    self.init(totalCostLimit: Self.defaultTotalCostLimitBytes)
  }

  init(totalCostLimit: Int) {
    precondition(totalCostLimit > 0)
    self.totalCostLimit = totalCostLimit
    storage.totalCostLimit = totalCostLimit
  }

  public var totalCostBytes: Int { trackedCostBytes }

  public func image(for key: ThumbnailKey) -> CGImage? {
    let objectKey = ThumbnailCacheObjectKey(key)
    guard let image = storage.object(forKey: objectKey) else {
      removeEntry(for: key)
      return nil
    }
    return image
  }

  public func insert(_ image: CGImage, for key: ThumbnailKey) {
    let cost = image.bytesPerRow * image.height
    removeEntry(for: key)
    let sequence = nextSequence
    nextSequence += 1
    entries[key] = CacheEntry(cost: cost, sequence: sequence)
    insertionOrder.append(CacheOrderEntry(key: key, sequence: sequence))
    trackedCostBytes += cost
    storage.setObject(image, forKey: ThumbnailCacheObjectKey(key), cost: cost)
    evictToLimit()
  }

  public func removeItems(at url: URL) {
    let matchingKeys = entries.keys.filter { $0.url == url }
    for key in matchingKeys {
      removeEntry(for: key)
    }
    compactOrderIfNeeded()
  }

  public func removeAll() {
    storage.removeAllObjects()
    entries.removeAll(keepingCapacity: false)
    insertionOrder.removeAll(keepingCapacity: false)
    nextEvictionIndex = 0
    trackedCostBytes = 0
  }

  private func removeEntry(for key: ThumbnailKey) {
    guard let entry = entries.removeValue(forKey: key) else { return }
    trackedCostBytes -= entry.cost
    storage.removeObject(forKey: ThumbnailCacheObjectKey(key))
  }

  private func evictToLimit() {
    while trackedCostBytes > totalCostLimit, nextEvictionIndex < insertionOrder.count {
      let candidate = insertionOrder[nextEvictionIndex]
      nextEvictionIndex += 1
      guard entries[candidate.key]?.sequence == candidate.sequence else { continue }
      removeEntry(for: candidate.key)
    }
    compactOrderIfNeeded()
  }

  private func compactOrderIfNeeded() {
    guard nextEvictionIndex > 128, nextEvictionIndex * 2 > insertionOrder.count else { return }
    insertionOrder.removeFirst(nextEvictionIndex)
    nextEvictionIndex = 0
  }
}

private struct CacheEntry {
  let cost: Int
  let sequence: UInt64
}

private struct CacheOrderEntry {
  let key: ThumbnailKey
  let sequence: UInt64
}

private final class ThumbnailCacheObjectKey: NSObject {
  let key: ThumbnailKey

  init(_ key: ThumbnailKey) {
    self.key = key
  }

  override var hash: Int { key.hashValue }

  override func isEqual(_ object: Any?) -> Bool {
    guard let other = object as? ThumbnailCacheObjectKey else { return false }
    return key == other.key
  }
}
