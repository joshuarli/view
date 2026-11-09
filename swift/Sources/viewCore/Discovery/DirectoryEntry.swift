import Foundation
import UniformTypeIdentifiers

struct DirectoryReadError: Error {
  let reason: String
}

struct DirectoryEntry: Sendable {
  static let resourceKeys: Set<URLResourceKey> = [
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
    name = try Self.required(values.name, key: .nameKey, url: url)
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

enum DirectoryEntryReader {
  static func read(
    at url: URL,
    readResourceValues: @Sendable (URL, Set<URLResourceKey>) throws -> URLResourceValues
  ) throws -> [DirectoryEntry] {
    let urls: [URL]
    do {
      urls = try FileManager.default.contentsOfDirectory(
        at: url,
        includingPropertiesForKeys: Array(DirectoryEntry.resourceKeys)
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
        values = try readResourceValues(entryURL, DirectoryEntry.resourceKeys)
      } catch let error as CancellationError {
        throw error
      } catch {
        continue
      }
      do {
        entries.append(try DirectoryEntry(url: entryURL, values: values))
      } catch is DirectoryScannerError {
        continue
      }
    }
    entries.sort { PathOrder.comesBefore($0.name, $1.name) }
    return entries
  }

  static func readRoot(
    at url: URL,
    readResourceValues: @Sendable (URL, Set<URLResourceKey>) throws -> URLResourceValues
  ) throws -> [DirectoryEntry] {
    do {
      return try read(at: url, readResourceValues: readResourceValues)
    } catch let error as DirectoryReadError {
      throw DirectoryScannerError.rootUnreadable(path: url.path, reason: error.reason)
    }
  }
}
