import Foundation
import UniformTypeIdentifiers

public struct DirectoryScanSummary: Sendable, Equatable {
  public let itemCount: Int
  public let batchCount: Int
  public let skippedDirectoryCount: Int

  public init(itemCount: Int, batchCount: Int, skippedDirectoryCount: Int) {
    self.itemCount = itemCount
    self.batchCount = batchCount
    self.skippedDirectoryCount = skippedDirectoryCount
  }
}

public enum DirectoryScannerError: Error, Equatable, Sendable {
  case rootUnreadable(path: String, reason: String)
  case entryMetadataUnreadable(path: String, reason: String)
  case metadataUnavailable(path: String, key: String)
}

extension DirectoryScannerError: LocalizedError {
  public var errorDescription: String? {
    switch self {
    case .rootUnreadable(let path, let reason):
      "cannot read directory \(path): \(reason)"
    case .entryMetadataUnreadable(let path, let reason):
      "cannot read metadata for \(path): \(reason)"
    case .metadataUnavailable(let path, let key):
      "metadata \(key) unavailable for \(path)"
    }
  }
}

public enum DirectoryScanner {
  private static let batchSize = 128

  @concurrent
  public static func scan(
    root: RootDirectory,
    batchSink: @escaping @Sendable ([MediaItem]) async throws -> Void
  ) async throws -> DirectoryScanSummary {
    try await Self.scan(
      root: root,
      batchSink: batchSink,
      readResourceValues: { url, keys in
        try url.resourceValues(forKeys: keys)
      }
    )
  }

  @concurrent
  static func scan(
    root: RootDirectory,
    batchSink: @escaping @Sendable ([MediaItem]) async throws -> Void,
    readResourceValues: @escaping @Sendable (URL, Set<URLResourceKey>) throws -> URLResourceValues
  ) async throws -> DirectoryScanSummary {
    var pendingItems: [MediaItem] = []
    pendingItems.reserveCapacity(Self.batchSize)
    var itemCount = 0
    var batchCount = 0
    var skippedDirectoryCount = 0
    try Task.checkCancellation()
    var stack = [
      DirectoryFrame(
        entries: try DirectoryEntryReader.readRoot(
          at: root.url,
          readResourceValues: readResourceValues
        ),
        nextEntry: 0
      ),
    ]

    while !stack.isEmpty {
      try Task.checkCancellation()
      var frame = stack.removeLast()
      guard frame.nextEntry < frame.entries.count else { continue }

      let entry = frame.entries[frame.nextEntry]
      frame.nextEntry += 1
      stack.append(frame)
      try Task.checkCancellation()

      guard !entry.isSymbolicLink, !entry.isHidden, !entry.isPackage, !entry.name.hasPrefix(".") else { continue }

      if entry.isDirectory {
        do {
          let children = try DirectoryEntryReader.read(
            at: entry.url,
            readResourceValues: readResourceValues
          )
          stack.append(DirectoryFrame(entries: children, nextEntry: 0))
        } catch is DirectoryReadError {
          skippedDirectoryCount += 1
        }
        continue
      }

      guard entry.isRegularFile, let contentType = entry.contentType else { continue }
      let kind: MediaKind
      if contentType.conforms(to: .image) {
        kind = .image
      } else if contentType.conforms(to: .movie) {
        kind = .movie
      } else {
        continue
      }

      pendingItems.append(
        MediaItem(
          url: entry.url,
          kind: kind,
          modificationDate: entry.modificationDate,
          fileSize: entry.fileSize.map(Int64.init)
        )
      )
      itemCount += 1

      if pendingItems.count == Self.batchSize {
        try await batchSink(pendingItems)
        batchCount += 1
        pendingItems.removeAll(keepingCapacity: true)
      }
    }

    try Task.checkCancellation()
    if !pendingItems.isEmpty {
      try await batchSink(pendingItems)
      batchCount += 1
      try Task.checkCancellation()
    }

    return DirectoryScanSummary(
      itemCount: itemCount,
      batchCount: batchCount,
      skippedDirectoryCount: skippedDirectoryCount
    )
  }
}

private struct DirectoryFrame {
  let entries: [DirectoryEntry]
  var nextEntry: Int
}
