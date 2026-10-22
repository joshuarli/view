import AppKit
import CoreGraphics
import viewCore

@MainActor
final class GridViewController: NSViewController, NSCollectionViewDataSource, NSCollectionViewDelegate, NSCollectionViewPrefetching {
  private let root: RootDirectory
  private let metrics: ViewMetrics
  private(set) var catalog = Catalog()
  private var scanTask: Task<Void, Never>?
  private var thumbnailCoordinator: ThumbnailCoordinator?
  private var prefetchIndexPaths: Set<IndexPath> = []
  private var lastScrollOriginY: CGFloat = 0
  private var scrollDirection: ScrollDirection = .down
  private var isObservingScrollBounds = false
  private var scrollView: NSScrollView!
  private(set) var collectionView: NSCollectionView!
  private var emptyStateLabel: NSTextField!

  init(root: RootDirectory, metrics: ViewMetrics) {
    self.root = root
    self.metrics = metrics
    super.init(nibName: nil, bundle: nil)
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    fatalError("init(coder:) is unavailable")
  }

  override func loadView() {
    let rootView = NSView()
    rootView.wantsLayer = true
    rootView.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor

    let layout = NSCollectionViewFlowLayout()
    layout.itemSize = NSSize(width: 160, height: 160)
    layout.minimumInteritemSpacing = 10
    layout.minimumLineSpacing = 10
    layout.sectionInset = NSEdgeInsets(top: 16, left: 16, bottom: 16, right: 16)

    collectionView = NSCollectionView(frame: .zero)
    collectionView.collectionViewLayout = layout
    collectionView.dataSource = self
    collectionView.delegate = self
    collectionView.prefetchDataSource = self
    collectionView.isSelectable = true
    collectionView.allowsEmptySelection = true
    collectionView.allowsMultipleSelection = false
    collectionView.backgroundColors = [.windowBackgroundColor]
    collectionView.register(ThumbnailCell.self, forItemWithIdentifier: ThumbnailCell.reuseIdentifier)
    collectionView.translatesAutoresizingMaskIntoConstraints = true
    collectionView.autoresizingMask = [.width]

    scrollView = NSScrollView()
    scrollView.translatesAutoresizingMaskIntoConstraints = false
    scrollView.drawsBackground = false
    scrollView.hasVerticalScroller = true
    scrollView.hasHorizontalScroller = false
    scrollView.contentView.postsBoundsChangedNotifications = true
    scrollView.documentView = collectionView

    emptyStateLabel = NSTextField(labelWithString: root.url.path)
    emptyStateLabel.textColor = .secondaryLabelColor
    emptyStateLabel.alignment = .center
    emptyStateLabel.lineBreakMode = .byTruncatingMiddle
    emptyStateLabel.translatesAutoresizingMaskIntoConstraints = false

    rootView.addSubview(scrollView)
    rootView.addSubview(emptyStateLabel)
    NSLayoutConstraint.activate([
      scrollView.leadingAnchor.constraint(equalTo: rootView.leadingAnchor),
      scrollView.trailingAnchor.constraint(equalTo: rootView.trailingAnchor),
      scrollView.topAnchor.constraint(equalTo: rootView.topAnchor),
      scrollView.bottomAnchor.constraint(equalTo: rootView.bottomAnchor),
      emptyStateLabel.centerXAnchor.constraint(equalTo: rootView.centerXAnchor),
      emptyStateLabel.centerYAnchor.constraint(equalTo: rootView.centerYAnchor),
      emptyStateLabel.leadingAnchor.constraint(greaterThanOrEqualTo: rootView.leadingAnchor, constant: 24),
      emptyStateLabel.trailingAnchor.constraint(lessThanOrEqualTo: rootView.trailingAnchor, constant: -24),
    ])
    self.view = rootView
    updateEmptyState()
  }

  override func viewDidAppear() {
    super.viewDidAppear()
    if thumbnailCoordinator == nil {
      thumbnailCoordinator = ThumbnailCoordinator(grid: self, metrics: metrics)
    }
    beginObservingScrollBounds()
    guard scanTask == nil else { return }
    startScan()
  }

  override func viewWillDisappear() {
    super.viewWillDisappear()
    endObservingScrollBounds()
  }

  func collectionView(_ collectionView: NSCollectionView, numberOfItemsInSection section: Int) -> Int {
    catalog.count
  }

  func collectionView(
    _ collectionView: NSCollectionView,
    itemForRepresentedObjectAt indexPath: IndexPath
  ) -> NSCollectionViewItem {
    guard let item = catalog.item(at: indexPath.item) else {
      preconditionFailure("collection view requested an item outside the catalog")
    }
    let cell = collectionView.makeItem(withIdentifier: ThumbnailCell.reuseIdentifier, for: indexPath) as! ThumbnailCell
    cell.represent(item, isSelected: item.url == catalog.selectedURL)
    return cell
  }

  func collectionView(_ collectionView: NSCollectionView, didSelectItemsAt indexPaths: Set<IndexPath>) {
    guard let indexPath = indexPaths.first, let item = catalog.item(at: indexPath.item) else { return }
    do {
      try catalog.select(item.url)
    } catch {
      preconditionFailure(error.localizedDescription)
    }
    updateVisibleSelection()
  }

  func collectionView(_ collectionView: NSCollectionView, didDeselectItemsAt indexPaths: Set<IndexPath>) {
    if collectionView.selectionIndexPaths.isEmpty {
      catalog.clearSelection()
    }
    updateVisibleSelection()
  }

  func collectionView(_ collectionView: NSCollectionView, prefetchItemsAt indexPaths: [IndexPath]) {
    prefetchIndexPaths.formUnion(indexPaths)
    updateThumbnailDemand()
  }

  func collectionView(_ collectionView: NSCollectionView, cancelPrefetchingForItemsAt indexPaths: [IndexPath]) {
    prefetchIndexPaths.subtract(indexPaths)
    updateThumbnailDemand()
  }

  func displayThumbnail(_ image: CGImage, for url: URL) -> Bool {
    for indexPath in collectionView.indexPathsForVisibleItems() {
      guard
        catalog.item(at: indexPath.item)?.url == url,
        let cell = collectionView.item(at: indexPath) as? ThumbnailCell,
        cell.representedURL == url
      else { continue }
      cell.display(image)
      return true
    }
    return false
  }

  private func startScan() {
    let root = self.root
    metrics.scanStarted()
    scanTask = Task { [weak self, root] in
      do {
        let volume = try await ThumbnailCoordinator.volume(for: root)
        guard let self else { return }
        guard !Task.isCancelled else { throw CancellationError() }
        self.thumbnailCoordinator?.configure(volume: volume)
        let summary = try await DirectoryScanner.scan(root: root) { [weak self] batch in
          await self?.append(batch)
        }
        self.metrics.scanCompleted(summary)
      } catch is CancellationError {
        self?.metrics.scanFailed()
      } catch {
        self?.metrics.scanFailed()
        self?.presentScanError(error)
      }
    }
  }

  private func append(_ batch: [MediaItem]) {
    let insertedIndices = catalog.append(batch)
    guard !insertedIndices.isEmpty else { return }
    let indexPaths = Set(insertedIndices.map { IndexPath(item: $0, section: 0) })
    collectionView.insertItems(at: indexPaths)
    metrics.catalogBatchApplied()
    updateEmptyState()
    collectionView.layoutSubtreeIfNeeded()
    updateThumbnailDemand()
  }

  private func updateEmptyState() {
    let isEmpty = catalog.isEmpty
    scrollView?.isHidden = isEmpty
    emptyStateLabel?.isHidden = !isEmpty
  }

  private func updateVisibleSelection() {
    for indexPath in collectionView.indexPathsForVisibleItems() {
      guard
        let item = catalog.item(at: indexPath.item),
        let cell = collectionView.item(at: indexPath) as? ThumbnailCell
      else { continue }
      cell.setCatalogSelection(item.url == catalog.selectedURL)
    }
  }

  private func beginObservingScrollBounds() {
    guard !isObservingScrollBounds else { return }
    lastScrollOriginY = scrollView.contentView.bounds.origin.y
    NotificationCenter.default.addObserver(
      self,
      selector: #selector(clipViewBoundsDidChange(_:)),
      name: NSView.boundsDidChangeNotification,
      object: scrollView.contentView
    )
    isObservingScrollBounds = true
  }

  private func endObservingScrollBounds() {
    guard isObservingScrollBounds else { return }
    NotificationCenter.default.removeObserver(
      self,
      name: NSView.boundsDidChangeNotification,
      object: scrollView.contentView
    )
    isObservingScrollBounds = false
  }

  @objc private func clipViewBoundsDidChange(_ notification: Notification) {
    let currentOriginY = scrollView.contentView.bounds.origin.y
    if currentOriginY > lastScrollOriginY {
      scrollDirection = .down
    } else if currentOriginY < lastScrollOriginY {
      scrollDirection = .up
    }
    lastScrollOriginY = currentOriginY
    updateThumbnailDemand()
  }

  private func updateThumbnailDemand() {
    let visiblePaths = collectionView.indexPathsForVisibleItems().sorted { $0.item < $1.item }
    let visibleItems = visiblePaths.compactMap { catalog.item(at: $0.item) }
    guard !visiblePaths.isEmpty else {
      thumbnailCoordinator?.updateDemand(visible: [], prefetch: [])
      return
    }

    let visibleIndices = visiblePaths.map(\.item)
    let prefetchIndices: [Int]
    switch scrollDirection {
    case .down:
      let start = visibleIndices[visibleIndices.count - 1] + 1
      let end = min(catalog.count, start + visibleIndices.count)
      prefetchIndices = start < end ? Array(start..<end) : []
    case .up:
      let end = visibleIndices[0]
      let start = max(0, end - visibleIndices.count)
      prefetchIndices = end > start ? Array((start..<end).reversed()) : []
    }
    let callbackIndices = Set(prefetchIndexPaths.map(\.item))
    let callbackPrefetch = prefetchIndices.filter { callbackIndices.contains($0) }
    let selectedPrefetch = callbackPrefetch.isEmpty ? prefetchIndices : callbackPrefetch
    let prefetchItems = selectedPrefetch.compactMap { catalog.item(at: $0) }
    thumbnailCoordinator?.updateDemand(visible: visibleItems, prefetch: prefetchItems)
  }

  private func presentScanError(_ error: any Error) {
    let alert = NSAlert(error: error)
    guard let window = view.window else {
      NSApp.presentError(error)
      return
    }
    alert.beginSheetModal(for: window) { _ in }
  }

  private enum ScrollDirection {
    case down
    case up
  }
}
