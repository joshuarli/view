import Foundation

public struct ThumbnailKey: Hashable, Sendable {
  public let url: URL
  public let kind: MediaKind
  public let pixelSize: Int
  public let scale: CGFloat
  public let modificationDate: Date?
  public let fileSize: Int64?

  public init?(item: MediaItem, pixelSize: Int, scale: CGFloat) {
    guard pixelSize > 0, scale > 0, scale.isFinite else { return nil }
    url = item.url
    kind = item.kind
    self.pixelSize = pixelSize
    self.scale = scale
    modificationDate = item.modificationDate
    fileSize = item.fileSize
  }
}
