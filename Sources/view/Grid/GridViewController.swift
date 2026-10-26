import AppKit
import CoreGraphics
import QuickLookUI
import viewCore

@MainActor
final class GridViewController: NSViewController, NSCollectionViewDataSource, NSCollectionViewDelegate, NSCollectionViewPrefetching {
  private static let prefetchViewportCount = 2

  private let root: RootDirectory
  private let metrics: ViewMetrics
  private(set) var catalog = Catalog()
  var catalogItemDidRemove: ((Catalog) -> Void)?
  var selectionDidChange: (() -> Void)?
  weak var previewHost: PreviewPanelController?
  var anchorIndex: Int?
  private var scanTask: Task<Void, Never>?
  private var thumbnailCoordinator: ThumbnailCoordinator?
  private var prefetchIndexPaths: Set<IndexPath> = []
  private var lastScrollOriginY: CGFloat = 0
  private var scrollDirection: ScrollDirection = .down
  private var isObservingScrollBounds = false
  private var isApplyingCatalogMutation = false
  private var scrollView: NSScrollView!
  private(set) var collectionView: GridCollectionView!
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

    collectionView = GridCollectionView(frame: .zero)
    collectionView.keyHandler = self
    collectionView.collectionViewLayout = layout
    collectionView.dataSource = self
    collectionView.delegate = self
    collectionView.prefetchDataSource = self
    collectionView.isSelectable = true
    collectionView.allowsEmptySelection = true
    collectionView.allowsMultipleSelection = true
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
    cell.represent(item, isSelected: catalog.selectedURLs.contains(item.url), image: thumbnailCoordinator?.cachedImage(for: item))
    return cell
  }

  func collectionView(_ collectionView: NSCollectionView, didSelectItemsAt indexPaths: Set<IndexPath>) {
    guard !isApplyingCatalogMutation else { return }
    let urls = indexPaths.compactMap { catalog.item(at: $0.item)?.url }.sorted { $0.path < $1.path }
    guard !urls.isEmpty else { return }
    let flags = NSEvent.modifierFlags
    do {
      if flags.contains(.command) || flags.contains(.shift) {
        for url in urls {
          try catalog.selectAdditional(url)
        }
      } else {
        try catalog.select(urls[0])
        for url in urls.dropFirst() {
          try catalog.selectAdditional(url)
        }
      }
    } catch {
      preconditionFailure(error.localizedDescription)
    }
    syncAnchor()
    updateVisibleSelection()
    selectionDidChange?()
  }

  func collectionView(_ collectionView: NSCollectionView, didDeselectItemsAt indexPaths: Set<IndexPath>) {
    guard !isApplyingCatalogMutation else { return }
    if collectionView.selectionIndexPaths.isEmpty {
      catalog.clearSelection()
    } else {
      for indexPath in indexPaths {
        if let item = catalog.item(at: indexPath.item) {
          catalog.deselect(item.url)
        }
      }
    }
    syncAnchor()
    updateVisibleSelection()
    selectionDidChange?()
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

  func selectRange(from anchor: Int, to clicked: Int) {
    guard let anchorURL = catalog.item(at: anchor)?.url,
      let clickedURL = catalog.item(at: clicked)?.url
    else { return }
    do {
      try catalog.selectRange(anchor: anchorURL, clicked: clickedURL)
    } catch {
      preconditionFailure(error.localizedDescription)
    }
    anchorIndex = anchor
    isApplyingCatalogMutation = true
    collectionView.deselectAll(nil)
    let lower = min(anchor, clicked)
    let upper = max(anchor, clicked)
    collectionView.selectItems(
      at: Set((lower...upper).map { IndexPath(item: $0, section: 0) }),
      scrollPosition: []
    )
    isApplyingCatalogMutation = false
    updateVisibleSelection()
    selectionDidChange?()
  }

  func selectFromPreview(_ url: URL) {
    guard let index = catalog.items.firstIndex(where: { $0.url == url }) else { return }
    do {
      try catalog.select(url)
    } catch {
      preconditionFailure(error.localizedDescription)
    }
    anchorIndex = index
    isApplyingCatalogMutation = true
    collectionView.deselectAll(nil)
    collectionView.selectItems(at: [IndexPath(item: index, section: 0)], scrollPosition: [])
    isApplyingCatalogMutation = false
    updateVisibleSelection()
    scrollSelectionIntoView()
    selectionDidChange?()
  }

  func removeCatalogItems(_ urls: [URL]) {
    guard !urls.isEmpty else { return }
    let urlSet = Set(urls)
    let removedIndexPaths = Set(catalog.items.indices
      .filter { urlSet.contains(catalog.items[$0].url) }
      .map { IndexPath(item: $0, section: 0) })
    guard !removedIndexPaths.isEmpty else { return }
    for url in urls {
      thumbnailCoordinator?.removeItem(at: url)
    }
    prefetchIndexPaths.removeAll(keepingCapacity: false)
    isApplyingCatalogMutation = true
    for url in urls {
      catalog.remove(url)
    }
    collectionView.deleteItems(at: removedIndexPaths)
    collectionView.deselectAll(nil)
    let selectedIndexPaths = Set(catalog.selectedURLs.compactMap { selectedURL in
      catalog.items.firstIndex(where: { $0.url == selectedURL })
        .map { IndexPath(item: $0, section: 0) }
    })
    if !selectedIndexPaths.isEmpty {
      collectionView.selectItems(at: selectedIndexPaths, scrollPosition: [])
    }
    isApplyingCatalogMutation = false
    syncAnchor()
    updateEmptyState()
    updateVisibleSelection()
    scrollSelectionIntoView()
    catalogItemDidRemove?(catalog)
    selectionDidChange?()
    updateThumbnailDemand()
  }

  func cancelWork() async {
    scanTask?.cancel()
    await scanTask?.value
    await thumbnailCoordinator?.cancelAll()
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

  private func syncAnchor() {
    anchorIndex = catalog.selectedURL.flatMap { url in
      catalog.items.firstIndex(where: { $0.url == url })
    }
  }

  private func scrollSelectionIntoView() {
    guard let selectedURL = catalog.selectedURL,
      let index = catalog.items.firstIndex(where: { $0.url == selectedURL })
    else { return }
    let path = IndexPath(item: index, section: 0)
    guard !collectionView.indexPathsForVisibleItems().contains(path) else { return }
    collectionView.scrollToItems(at: [path], scrollPosition: [.centeredVertically])
  }

  private func updateVisibleSelection() {
    for indexPath in collectionView.indexPathsForVisibleItems() {
      guard
        let item = catalog.item(at: indexPath.item),
        let cell = collectionView.item(at: indexPath) as? ThumbnailCell
      else { continue }
      cell.setCatalogSelection(catalog.selectedURLs.contains(item.url))
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
    NotificationCenter.default.addObserver(
      self,
      selector: #selector(windowBackingPropertiesDidChange(_:)),
      name: NSWindow.didChangeBackingPropertiesNotification,
      object: view.window
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
    NotificationCenter.default.removeObserver(
      self,
      name: NSWindow.didChangeBackingPropertiesNotification,
      object: view.window
    )
    isObservingScrollBounds = false
  }

  @objc private func windowBackingPropertiesDidChange(_ notification: Notification) {
    for indexPath in collectionView.indexPathsForVisibleItems() {
      guard
        let item = catalog.item(at: indexPath.item),
        let cell = collectionView.item(at: indexPath) as? ThumbnailCell
      else { continue }
      cell.represent(item, isSelected: catalog.selectedURLs.contains(item.url), image: thumbnailCoordinator?.cachedImage(for: item))
    }
    updateThumbnailDemand()
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
    let prefetchDepth = visibleIndices.count * Self.prefetchViewportCount
    switch scrollDirection {
    case .down:
      let start = visibleIndices[visibleIndices.count - 1] + 1
      let end = min(catalog.count, start + prefetchDepth)
      prefetchIndices = start < end ? Array(start..<end) : []
    case .up:
      let end = visibleIndices[0]
      let start = max(0, end - prefetchDepth)
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

  nonisolated override func acceptsPreviewPanelControl(_ panel: QLPreviewPanel!) -> Bool {
    MainActor.assumeIsolated {
      if let host = previewHost {
        return host.acceptsPreviewPanelControl(panel)
      }
      return catalog.selectedURL != nil
    }
  }

  nonisolated override func beginPreviewPanelControl(_ panel: QLPreviewPanel!) {
    MainActor.assumeIsolated {
      if let host = previewHost {
        host.beginPreviewPanelControl(panel)
      } else {
        panel.orderOut(nil)
      }
    }
  }

  nonisolated override func endPreviewPanelControl(_ panel: QLPreviewPanel!) {
    MainActor.assumeIsolated {
      previewHost?.endPreviewPanelControl(panel)
    }
  }

  override func keyDown(with event: NSEvent) {
    if handleCollectionKeyDown(event) {
      return
    }
    super.keyDown(with: event)
  }

  func handleCollectionKeyDown(_ event: NSEvent) -> Bool {
    let flags = event.modifierFlags.intersection([.command, .control, .option])
    if event.keyCode == 53 {
      return true
    }
    if event.charactersIgnoringModifiers == " ", flags.isEmpty {
      guard catalog.selectedURL != nil else { return false }
      previewHost?.togglePreview(event)
      return true
    }
    if event.keyCode == 51, flags.isEmpty || flags == .control {
      guard catalog.selectedURL != nil else { return false }
      previewHost?.trashSelectedItem(event)
      return true
    }
    return false
  }

  private enum ScrollDirection {
    case down
    case up
  }
}

@MainActor
final class GridCollectionView: NSCollectionView {
  weak var keyHandler: GridViewController?

  override func mouseDown(with event: NSEvent) {
    let location = convert(event.locationInWindow, from: nil)
    let clicked = indexPathForItem(at: location)?.item
    if event.clickCount > 1, clicked != nil, let handler = keyHandler {
      handler.previewHost?.togglePreview(event)
      return
    }
    if event.modifierFlags.contains(.shift),
      let handler = keyHandler,
      let anchor = handler.anchorIndex,
      anchor < handler.catalog.count,
      let clicked
    {
      handler.selectRange(from: anchor, to: clicked)
      return
    }
    super.mouseDown(with: event)
  }

  override func keyDown(with event: NSEvent) {
    if let handler = keyHandler, handler.handleCollectionKeyDown(event) {
      return
    }
    super.keyDown(with: event)
  }
}
