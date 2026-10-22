import AppKit
import CoreGraphics
import viewCore

@MainActor
final class ThumbnailCell: NSCollectionViewItem {
  static let reuseIdentifier = NSUserInterfaceItemIdentifier("ThumbnailCell")

  private let thumbnailView = NSImageView()
  private let videoBadgeView = NSImageView()
  private(set) var representedURL: URL?
  private var catalogSelection = false

  override func loadView() {
    let cellView = NSView()
    cellView.wantsLayer = true
    cellView.layer?.cornerRadius = 10
    cellView.layer?.masksToBounds = true

    thumbnailView.image = Self.placeholderImage
    thumbnailView.imageScaling = .scaleProportionallyUpOrDown
    thumbnailView.imageAlignment = .alignCenter
    thumbnailView.translatesAutoresizingMaskIntoConstraints = false

    videoBadgeView.image = NSImage(systemSymbolName: "play.fill", accessibilityDescription: nil)?
      .withSymbolConfiguration(NSImage.SymbolConfiguration(pointSize: 11, weight: .regular))
    videoBadgeView.contentTintColor = .secondaryLabelColor
    videoBadgeView.imageScaling = .scaleProportionallyUpOrDown
    videoBadgeView.translatesAutoresizingMaskIntoConstraints = false
    videoBadgeView.isHidden = true

    cellView.addSubview(thumbnailView)
    cellView.addSubview(videoBadgeView)
    NSLayoutConstraint.activate([
      thumbnailView.leadingAnchor.constraint(equalTo: cellView.leadingAnchor, constant: 6),
      thumbnailView.trailingAnchor.constraint(equalTo: cellView.trailingAnchor, constant: -6),
      thumbnailView.topAnchor.constraint(equalTo: cellView.topAnchor, constant: 6),
      thumbnailView.bottomAnchor.constraint(equalTo: cellView.bottomAnchor, constant: -6),
      videoBadgeView.trailingAnchor.constraint(equalTo: cellView.trailingAnchor, constant: -10),
      videoBadgeView.bottomAnchor.constraint(equalTo: cellView.bottomAnchor, constant: -10),
      videoBadgeView.widthAnchor.constraint(equalToConstant: 11),
      videoBadgeView.heightAnchor.constraint(equalToConstant: 11),
    ])
    view = cellView
    updateSelectionAppearance()
  }

  override func prepareForReuse() {
    super.prepareForReuse()
    representedURL = nil
    thumbnailView.image = Self.placeholderImage
    videoBadgeView.isHidden = true
    catalogSelection = false
    updateSelectionAppearance()
  }

  func represent(_ item: MediaItem, isSelected: Bool) {
    representedURL = item.url
    thumbnailView.image = Self.placeholderImage
    videoBadgeView.isHidden = item.kind != .movie
    catalogSelection = isSelected
    updateSelectionAppearance()
  }

  func setCatalogSelection(_ isSelected: Bool) {
    catalogSelection = isSelected
    updateSelectionAppearance()
  }

  func display(_ image: CGImage) {
    thumbnailView.image = NSImage(cgImage: image, size: NSSize(width: 160, height: 160))
  }

  private func updateSelectionAppearance() {
    view.wantsLayer = true
    view.layer?.backgroundColor = catalogSelection
      ? NSColor.selectedContentBackgroundColor.withAlphaComponent(0.35).cgColor
      : NSColor.clear.cgColor
  }

  private static var placeholderImage: NSImage? {
    NSImage(systemSymbolName: "photo", accessibilityDescription: nil)?
      .withSymbolConfiguration(NSImage.SymbolConfiguration(pointSize: 40, weight: .regular))
  }
}
