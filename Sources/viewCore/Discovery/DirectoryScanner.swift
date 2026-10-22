import Foundation
import UniformTypeIdentifiers

public struct DirectoryScanSummary: Sendable, Equatable {
  public let itemCount: Int
  public let batchCount: Int
  public let skippedDirectoryCount: Int
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
  private static let resourceKeys: Set<URLResourceKey> = [
    .isDirectoryKey,
    .isRegularFileKey,
    .isSymbolicLinkKey,
    .isPackageKey,
    .isHiddenKey,
    .nameKey,
    .contentTypeKey,
    .contentModificationDateKey,
    .fileSizeKey,
  ]

  @concurrent
  public static func scan(
    root: RootDirectory,
    batchSink: @escaping @Sendable ([MediaItem]) async throws -> Void
  ) async throws -> DirectoryScanSummary {
    var pendingItems: [MediaItem] = []
    pendingItems.reserveCapacity(Self.batchSize)
    var itemCount = 0
    var batchCount = 0
    var skippedDirectoryCount = 0
    try Task.checkCancellation()
    var stack = [DirectoryFrame(entries: try Self.readRootEntries(at: root.url), nextEntry: 0)]

    while !stack.isEmpty {
      try Task.checkCancellation()
      var frame = stack.removeLast()
      guard frame.nextEntry < frame.entries.count else { continue }

      let entry = frame.entries[frame.nextEntry]
      frame.nextEntry += 1
      stack.append(frame)
      try Task.checkCancellation()

      guard !entry.isSymbolicLink, !entry.isHidden, !entry.isPackage else { continue }

      if entry.isDirectory {
        do {
          let children = try Self.readEntries(at: entry.url)
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

  private static func readRootEntries(at url: URL) throws -> [DirectoryEntry] {
    do {
      return try readEntries(at: url)
    } catch let error as DirectoryReadError {
      throw DirectoryScannerError.rootUnreadable(path: url.path, reason: error.reason)
    }
  }

  private static func readEntries(at url: URL) throws -> [DirectoryEntry] {
    let urls: [URL]
    do {
      urls = try FileManager.default.contentsOfDirectory(
        at: url,
        includingPropertiesForKeys: Array(resourceKeys)
      )
    } catch {
      throw DirectoryReadError(reason: error.localizedDescription)
    }

    var entries: [DirectoryEntry] = []
    entries.reserveCapacity(urls.count)
    for entryURL in urls {
      try Task.checkCancellation()
      let values: URLResourceValues
      do {
        values = try entryURL.resourceValues(forKeys: resourceKeys)
      } catch {
        throw DirectoryScannerError.entryMetadataUnreadable(
          path: entryURL.path,
          reason: error.localizedDescription
        )
      }
      entries.append(try DirectoryEntry(url: entryURL, values: values))
    }
    entries.sort { PathOrder.comesBefore($0.name, $1.name) }
    return entries
  }
}

private struct DirectoryFrame {
  let entries: [DirectoryEntry]
  var nextEntry: Int
}

private struct DirectoryEntry {
  let url: URL
  let name: String
  let isDirectory: Bool
  let isRegularFile: Bool
  let isSymbolicLink: Bool
  let isPackage: Bool
  let isHidden: Bool
  let contentType: UTType?
  let modificationDate: Date?
  let fileSize: Int?

  init(url: URL, values: URLResourceValues) throws {
    self.url = url
    name = try Self.required(values.name, key: URLResourceKey.nameKey, url: url)
    isDirectory = try Self.required(values.isDirectory, key: .isDirectoryKey, url: url)
    isRegularFile = try Self.required(values.isRegularFile, key: .isRegularFileKey, url: url)
    isSymbolicLink = try Self.required(values.isSymbolicLink, key: .isSymbolicLinkKey, url: url)
    isPackage = try Self.required(values.isPackage, key: .isPackageKey, url: url)
    isHidden = try Self.required(values.isHidden, key: .isHiddenKey, url: url)
    contentType = values.contentType
    modificationDate = values.contentModificationDate
    fileSize = values.fileSize
  }

  private static func required<Value>(
    _ value: Value?,
    key: URLResourceKey,
    url: URL
  ) throws -> Value {
    guard let value else {
      throw DirectoryScannerError.metadataUnavailable(path: url.path, key: key.rawValue)
    }
    return value
  }
}

private struct DirectoryReadError: Error {
  let reason: String
}
