import AppKit
import CoreGraphics
import viewCore

enum ThumbnailCaptionStyle {
  case none
  case nameAndSize
}

@MainActor
final class ThumbnailCell: NSCollectionViewItem {
  static let reuseIdentifier = NSUserInterfaceItemIdentifier("ThumbnailCell")

  private let thumbnailView = NSImageView()
  private let videoBadgeView = NSImageView()
  private let nameLabel = NSTextField(labelWithString: "")
  private let detailLabel = NSTextField(labelWithString: "")
  private var thumbnailToCaptions: NSLayoutConstraint!
  private var thumbnailToBottom: NSLayoutConstraint!
  private var captionStyle = ThumbnailCaptionStyle.none
  private(set) var representedURL: URL?
  private var catalogSelection = false

  override func loadView() {
    let cellView = NSView()
    cellView.wantsLayer = true
    cellView.layer?.cornerRadius = 10
    cellView.layer?.masksToBounds = true

    thumbnailView.image = Self.placeholderImage(for: .image)
    thumbnailView.imageScaling = .scaleProportionallyUpOrDown
    thumbnailView.imageAlignment = .alignCenter
    thumbnailView.translatesAutoresizingMaskIntoConstraints = false
    thumbnailView.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
    thumbnailView.setContentCompressionResistancePriority(.defaultLow, for: .vertical)
    thumbnailView.setContentHuggingPriority(.defaultLow, for: .horizontal)
    thumbnailView.setContentHuggingPriority(.defaultLow, for: .vertical)

    videoBadgeView.image = NSImage(systemSymbolName: "play.fill", accessibilityDescription: nil)?
      .withSymbolConfiguration(NSImage.SymbolConfiguration(pointSize: 11, weight: .regular))
    videoBadgeView.contentTintColor = .secondaryLabelColor
    videoBadgeView.imageScaling = .scaleProportionallyUpOrDown
    videoBadgeView.translatesAutoresizingMaskIntoConstraints = false
    videoBadgeView.isHidden = true

    nameLabel.font = .systemFont(ofSize: 12)
    nameLabel.textColor = .labelColor
    nameLabel.alignment = .center
    nameLabel.lineBreakMode = .byTruncatingMiddle
    nameLabel.maximumNumberOfLines = 1
    nameLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
    nameLabel.translatesAutoresizingMaskIntoConstraints = false
    nameLabel.isHidden = true

    detailLabel.font = .systemFont(ofSize: 10)
    detailLabel.textColor = .secondaryLabelColor
    detailLabel.alignment = .center
    detailLabel.lineBreakMode = .byTruncatingTail
    detailLabel.maximumNumberOfLines = 1
    detailLabel.translatesAutoresizingMaskIntoConstraints = false
    detailLabel.isHidden = true

    cellView.addSubview(thumbnailView)
    cellView.addSubview(videoBadgeView)
    cellView.addSubview(nameLabel)
    cellView.addSubview(detailLabel)

    thumbnailToCaptions = thumbnailView.bottomAnchor.constraint(equalTo: nameLabel.topAnchor, constant: -4)
    thumbnailToBottom = thumbnailView.bottomAnchor.constraint(equalTo: cellView.bottomAnchor, constant: -6)
    NSLayoutConstraint.activate([
      thumbnailView.leadingAnchor.constraint(equalTo: cellView.leadingAnchor, constant: 6),
      thumbnailView.trailingAnchor.constraint(equalTo: cellView.trailingAnchor, constant: -6),
      thumbnailView.topAnchor.constraint(equalTo: cellView.topAnchor, constant: 6),
      nameLabel.leadingAnchor.constraint(equalTo: cellView.leadingAnchor, constant: 6),
      nameLabel.trailingAnchor.constraint(equalTo: cellView.trailingAnchor, constant: -6),
      nameLabel.bottomAnchor.constraint(equalTo: detailLabel.topAnchor, constant: -1),
      detailLabel.leadingAnchor.constraint(equalTo: cellView.leadingAnchor, constant: 6),
      detailLabel.trailingAnchor.constraint(equalTo: cellView.trailingAnchor, constant: -6),
      detailLabel.bottomAnchor.constraint(equalTo: cellView.bottomAnchor, constant: -4),
      videoBadgeView.trailingAnchor.constraint(equalTo: cellView.trailingAnchor, constant: -10),
      videoBadgeView.bottomAnchor.constraint(equalTo: cellView.bottomAnchor, constant: -10),
      videoBadgeView.widthAnchor.constraint(equalToConstant: 11),
      videoBadgeView.heightAnchor.constraint(equalToConstant: 11),
    ])
    thumbnailToBottom.isActive = true

    view = cellView
    updateSelectionAppearance()
  }

  override func prepareForReuse() {
    super.prepareForReuse()
    representedURL = nil
    thumbnailView.image = Self.placeholderImage(for: .image)
    videoBadgeView.isHidden = true
    nameLabel.stringValue = ""
    nameLabel.font = .systemFont(ofSize: 12)
    detailLabel.stringValue = ""
    catalogSelection = false
    isShowingThumbnailFailure = false
    updateSelectionAppearance()
  }

  func setCaptionStyle(_ style: ThumbnailCaptionStyle) {
    guard style != captionStyle else { return }
    captionStyle = style
    let showsCaptions = style != .none
    nameLabel.isHidden = !showsCaptions
    detailLabel.isHidden = !showsCaptions
    if showsCaptions {
      thumbnailToBottom.isActive = false
      thumbnailToCaptions.isActive = true
    } else {
      thumbnailToCaptions.isActive = false
      thumbnailToBottom.isActive = true
    }
  }

  func represent(_ item: MediaItem, isSelected: Bool, image: CGImage?, isThumbnailFailed: Bool = false) {
    representedURL = item.url
    if let image {
      thumbnailView.image = NSImage(cgImage: image, size: .zero)
      isShowingThumbnailFailure = false
    } else if isThumbnailFailed, image == nil {
      showThumbnailFailure()
    } else {
      thumbnailView.image = Self.placeholderImage(for: item.kind)
      isShowingThumbnailFailure = false
    }
    videoBadgeView.isHidden = item.kind != .movie
    nameLabel.stringValue = item.url.lastPathComponent
    nameLabel.font = item.kind == .directory ? .systemFont(ofSize: 12, weight: .medium) : .systemFont(ofSize: 12)
    detailLabel.stringValue = Self.detailText(for: item)
    catalogSelection = isSelected
    updateSelectionAppearance()
  }

  func setCatalogSelection(_ isSelected: Bool) {
    catalogSelection = isSelected
    updateSelectionAppearance()
  }

  func display(_ image: CGImage) {
    thumbnailView.image = NSImage(cgImage: image, size: .zero)
    isShowingThumbnailFailure = false
  }

  func showThumbnailFailure() {
    guard !isShowingThumbnailFailure else { return }
    thumbnailView.image = Self.failureImage()
    isShowingThumbnailFailure = true
  }

  private(set) var isShowingThumbnailFailure = false

  private func updateSelectionAppearance() {
    view.wantsLayer = true
    view.layer?.backgroundColor = catalogSelection
      ? NSColor.selectedContentBackgroundColor.withAlphaComponent(0.35).cgColor
      : NSColor.clear.cgColor
  }

  private static func detailText(for item: MediaItem) -> String {
    switch item.kind {
    case .directory:
      return ""
    case .image, .movie:
      guard let fileSize = item.fileSize else { return "" }
      return byteFormatter.string(fromByteCount: fileSize)
    }
  }

  private static let byteFormatter: ByteCountFormatter = {
    let formatter = ByteCountFormatter()
    formatter.countStyle = .file
    return formatter
  }()

  private static func placeholderImage(for kind: MediaKind) -> NSImage? {
    let symbolName = kind == .directory ? "folder" : "photo"
    return NSImage(systemSymbolName: symbolName, accessibilityDescription: nil)?
      .withSymbolConfiguration(NSImage.SymbolConfiguration(pointSize: 40, weight: .regular))
  }

  private static func failureImage() -> NSImage? {
    NSImage(systemSymbolName: "exclamationmark.triangle", accessibilityDescription: nil)?
      .withSymbolConfiguration(NSImage.SymbolConfiguration(pointSize: 40, weight: .regular))
  }
}
