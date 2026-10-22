import Foundation

public enum MediaKind: Sendable, Equatable {
  case image
  case movie
}

public struct MediaItem: Sendable, Equatable, Identifiable {
  public let url: URL
  public let kind: MediaKind
  public let modificationDate: Date?
  public let fileSize: Int64?

  public var id: URL { url }

  public init(url: URL, kind: MediaKind, modificationDate: Date?, fileSize: Int64?) {
    self.url = url
    self.kind = kind
    self.modificationDate = modificationDate
    self.fileSize = fileSize
  }
}
