import Foundation
import UniformTypeIdentifiers

public enum DirectoryLister {
  @concurrent
  public static func list(directory: URL) async throws -> [MediaItem] {
    try await Self.list(
      directory: directory,
      readResourceValues: { url, keys in
        try url.resourceValues(forKeys: keys)
      }
    )
  }

  @concurrent
  static func list(
    directory: URL,
    readResourceValues: @Sendable (URL, Set<URLResourceKey>) throws -> URLResourceValues
  ) async throws -> [MediaItem] {
    try Task.checkCancellation()
    let entries = try DirectoryEntryReader.readRoot(
      at: directory,
      readResourceValues: readResourceValues
    )

    var items: [MediaItem] = []
    items.reserveCapacity(entries.count)
    for entry in entries {
      try Task.checkCancellation()
      guard !entry.isSymbolicLink, !entry.isHidden, !entry.isPackage, !entry.name.hasPrefix(".") else {
        continue
      }

      if entry.isDirectory {
        items.append(
          MediaItem(
            url: entry.url,
            kind: .directory,
            modificationDate: entry.modificationDate,
            fileSize: nil
          )
        )
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
      items.append(
        MediaItem(
          url: entry.url,
          kind: kind,
          modificationDate: entry.modificationDate,
          fileSize: entry.fileSize.map(Int64.init)
        )
      )
    }

    items.sort(by: Self.comesBefore)
    return items
  }

  static func comesBefore(_ lhs: MediaItem, _ rhs: MediaItem) -> Bool {
    let lhsIsDirectory = lhs.kind == .directory
    let rhsIsDirectory = rhs.kind == .directory
    if lhsIsDirectory != rhsIsDirectory {
      return lhsIsDirectory
    }
    if !lhsIsDirectory {
      switch (lhs.fileSize, rhs.fileSize) {
      case let (left?, right?) where left != right:
        return left > right
      case (_?, nil):
        return true
      case (nil, _?):
        return false
      default:
        break
      }
    }
    return PathOrder.comesBefore(lhs.url.lastPathComponent, rhs.url.lastPathComponent)
  }
}
