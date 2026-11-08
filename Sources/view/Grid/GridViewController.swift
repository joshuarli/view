import AppKit
import CoreGraphics
import QuickLookThumbnailing
import QuickLookUI
import viewCore

@MainActor
final class GridViewController: NSViewController, NSCollectionViewDataSource, NSCollectionViewDelegate, NSCollectionViewPrefetching {
  private static let prefetchViewportCount = 2

  private let mode: BrowserMode
  private let metrics: ViewMetrics
  private(set) var root: RootDirectory
  private(set) var catalog = Catalog()
  private(set) var viewMode: ViewMode
  var catalogDidChange: ((Catalog) -> Void)?
  var selectionDidChange: (() -> Void)?
  weak var previewHost: PreviewPanelController?
  var anchorIndex: Int?
  private var scanTask: Task<Void, Never>?
  private var scanGeneration = 0
  private var thumbnailCoordinator: ThumbnailCoordinator?
  private var prefetchIndexPaths: Set<IndexPath> = []
  private var lastScrollOriginY: CGFloat = 0
  private var scrollDirection: ScrollDirection = .down
  private var isObservingScrollBounds = false
  private var isApplyingCatalogMutation = false
  private var isApplyingViewMode = false
  private var backStack: [HistoryEntry] = []
  private var forwardStack: [HistoryEntry] = []

  private struct HistoryEntry {
    let url: URL
    let selectedURL: URL?
  }
  private var rootContainer: NSView?
  private var carouselPreviewRegion: NSView?
  private var carouselPreviewImageView: NSImageView?
  private var carouselPreviewBadgeView: NSImageView?
  private var carouselPreviewAspectConstraint: NSLayoutConstraint?
  private var carouselPreviewGeneration: UInt64 = 0
  private var carouselContainer: NSView?
  private var gridContainer: NSView?
  private var scrollView: NSScrollView!
  private(set) var collectionView: GridCollectionView!
  private var emptyStateLabel: NSTextField!

  init(root: RootDirectory, mode: BrowserMode, metrics: ViewMetrics, viewMode: ViewMode = .carousel) {
    self.root = root
    self.mode = mode
    self.metrics = metrics
    self.viewMode = viewMode
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
    rootContainer = rootView

    collectionView = GridCollectionView(frame: .zero)
    collectionView.keyHandler = self
    let layout = NSCollectionViewFlowLayout()
    layout.scrollDirection = .vertical
    layout.itemSize = mode == .browser ? NSSize(width: 160, height: 180) : NSSize(width: 160, height: 160)
    layout.minimumInteritemSpacing = 10
    layout.minimumLineSpacing = 10
    layout.sectionInset = NSEdgeInsets(top: 16, left: 16, bottom: 16, right: 16)
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
    scrollView.contentView.postsBoundsChangedNotifications = true
    scrollView.documentView = collectionView

    emptyStateLabel = NSTextField(labelWithString: emptyStateText)
    emptyStateLabel.textColor = .secondaryLabelColor
    emptyStateLabel.alignment = .center
    emptyStateLabel.maximumNumberOfLines = mode == .browser ? 2 : 1
    emptyStateLabel.lineBreakMode = mode == .browser ? .byTruncatingTail : .byTruncatingMiddle
    emptyStateLabel.translatesAutoresizingMaskIntoConstraints = false

    let previewRegion = NSView()
    previewRegion.translatesAutoresizingMaskIntoConstraints = false
    carouselPreviewRegion = previewRegion

    let previewImage = NSImageView()
    previewImage.imageScaling = .scaleProportionallyUpOrDown
    previewImage.imageAlignment = .alignCenter
    previewImage.translatesAutoresizingMaskIntoConstraints = false
    previewImage.setContentHuggingPriority(.init(1), for: .horizontal)
    previewImage.setContentHuggingPriority(.init(1), for: .vertical)
    previewImage.setContentCompressionResistancePriority(.init(1), for: .horizontal)
    previewImage.setContentCompressionResistancePriority(.init(1), for: .vertical)
    previewRegion.addSubview(previewImage)
    let aspect = previewImage.widthAnchor.constraint(equalTo: previewImage.heightAnchor, multiplier: 1)
    let fillWidth = previewImage.widthAnchor.constraint(equalTo: previewRegion.widthAnchor)
    fillWidth.priority = .defaultLow
    let fillHeight = previewImage.heightAnchor.constraint(equalTo: previewRegion.heightAnchor)
    fillHeight.priority = .defaultLow
    NSLayoutConstraint.activate([
      previewImage.centerXAnchor.constraint(equalTo: previewRegion.centerXAnchor),
      previewImage.centerYAnchor.constraint(equalTo: previewRegion.centerYAnchor),
      previewImage.leadingAnchor.constraint(greaterThanOrEqualTo: previewRegion.leadingAnchor),
      previewImage.trailingAnchor.constraint(lessThanOrEqualTo: previewRegion.trailingAnchor),
      previewImage.topAnchor.constraint(greaterThanOrEqualTo: previewRegion.topAnchor),
      previewImage.bottomAnchor.constraint(lessThanOrEqualTo: previewRegion.bottomAnchor),
      aspect,
      fillWidth,
      fillHeight,
    ])
    carouselPreviewAspectConstraint = aspect
    carouselPreviewImageView = previewImage

    let previewBadge = NSImageView()
    previewBadge.image = NSImage(systemSymbolName: "play.fill", accessibilityDescription: nil)?
      .withSymbolConfiguration(NSImage.SymbolConfiguration(pointSize: 16, weight: .regular))
    previewBadge.contentTintColor = .secondaryLabelColor
    previewBadge.imageScaling = .scaleProportionallyUpOrDown
    previewBadge.translatesAutoresizingMaskIntoConstraints = false
    previewBadge.isHidden = true
    previewRegion.addSubview(previewBadge)
    NSLayoutConstraint.activate([
      previewBadge.trailingAnchor.constraint(equalTo: previewRegion.trailingAnchor, constant: -12),
      previewBadge.bottomAnchor.constraint(equalTo: previewRegion.bottomAnchor, constant: -12),
      previewBadge.widthAnchor.constraint(equalToConstant: 16),
      previewBadge.heightAnchor.constraint(equalToConstant: 16),
    ])
    carouselPreviewBadgeView = previewBadge

    let gridContainer = NSView()
    gridContainer.translatesAutoresizingMaskIntoConstraints = false
    self.gridContainer = gridContainer

    let carouselContainer = NSView()
    carouselContainer.translatesAutoresizingMaskIntoConstraints = false
    self.carouselContainer = carouselContainer

    rootView.addSubview(emptyStateLabel)
    NSLayoutConstraint.activate([
      emptyStateLabel.centerXAnchor.constraint(equalTo: rootView.centerXAnchor),
      emptyStateLabel.centerYAnchor.constraint(equalTo: rootView.centerYAnchor),
      emptyStateLabel.leadingAnchor.constraint(greaterThanOrEqualTo: rootView.leadingAnchor, constant: 24),
      emptyStateLabel.trailingAnchor.constraint(lessThanOrEqualTo: rootView.trailingAnchor, constant: -24),
    ])

    self.view = rootView
    applyViewMode()
    updateEmptyState()
  }

  override func viewDidAppear() {
    super.viewDidAppear()
    if thumbnailCoordinator == nil {
      thumbnailCoordinator = ThumbnailCoordinator(grid: self, metrics: metrics)
    }
    beginObservingScrollBounds()
    updateLocationPresentation()
    _ = view.window?.makeFirstResponder(collectionView)
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
    cell.setCaptionStyle(captionStyle)
    cell.represent(
      item,
      isSelected: catalog.selectedURLs.contains(item.url),
      image: thumbnailCoordinator?.cachedImage(for: item),
      isThumbnailFailed: thumbnailCoordinator?.isThumbnailFailed(for: item) ?? false
    )
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
    updateCarouselPreview()
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
    updateCarouselPreview()
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

  func displayThumbnailFailure(for url: URL) {
    for indexPath in collectionView.indexPathsForVisibleItems() {
      guard
        catalog.item(at: indexPath.item)?.url == url,
        let cell = collectionView.item(at: indexPath) as? ThumbnailCell,
        cell.representedURL == url
      else { continue }
      cell.showThumbnailFailure()
      return
    }
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
    updateCarouselPreview()
    selectionDidChange?()
  }

  func selectFromPreview(_ url: URL) {
    selectCatalogItem(url)
  }

  func activateItem(at index: Int) {
    guard let item = catalog.item(at: index) else { return }
    if mode == .browser, item.kind == .directory {
      navigate(to: item.url)
    } else {
      previewHost?.togglePreview(nil)
    }
  }

  func navigate(to directory: URL, selecting selection: URL? = nil) {
    navigate(to: directory, selecting: selection, pushingHistory: true)
  }

  private func navigate(to directory: URL, selecting selection: URL?, pushingHistory: Bool) {
    guard mode == .browser else { return }
    let target = directory.standardizedFileURL
    guard target.path != root.url.path else { return }

    if pushingHistory {
      backStack.append(HistoryEntry(url: root.url, selectedURL: catalog.selectedURL))
      forwardStack.removeAll()
    }

    scanGeneration += 1
    let previous = scanTask
    previous?.cancel()
    scanTask = nil
    root = RootDirectory(url: target)

    catalog = Catalog()
    anchorIndex = nil
    prefetchIndexPaths.removeAll(keepingCapacity: false)
    isApplyingCatalogMutation = true
    collectionView.deselectAll(nil)
    isApplyingCatalogMutation = false
    collectionView.reloadData()
    scrollToTop()
    updateEmptyState()
    updateCarouselPreview()
    updateLocationPresentation()
    updateThumbnailDemand()
    catalogDidChange?(catalog)
    selectionDidChange?()
    startScan(selecting: selection, after: previous)
  }

  @discardableResult
  func navigateToParent() -> Bool {
    guard mode == .browser else { return false }
    let parent = root.url.deletingLastPathComponent().standardizedFileURL
    guard parent.path != root.url.path else { return false }
    navigate(to: parent, selecting: root.url)
    return true
  }

  @discardableResult
  func navigateBack() -> Bool {
    guard mode == .browser, let target = backStack.last, target.url.path != root.url.path else { return false }
    backStack.removeLast()
    forwardStack.append(HistoryEntry(url: root.url, selectedURL: catalog.selectedURL))
    navigate(to: target.url, selecting: target.selectedURL, pushingHistory: false)
    return true
  }

  @discardableResult
  func navigateForward() -> Bool {
    guard mode == .browser, let target = forwardStack.last, target.url.path != root.url.path else { return false }
    forwardStack.removeLast()
    backStack.append(HistoryEntry(url: root.url, selectedURL: catalog.selectedURL))
    navigate(to: target.url, selecting: target.selectedURL, pushingHistory: false)
    return true
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
    updateCarouselPreview()
    scrollSelectionIntoView()
    catalogDidChange?(catalog)
    selectionDidChange?()
    updateThumbnailDemand()
  }

  func restoreCatalogItems(_ files: [TrashedFile]) {
    let applicable = files.filter {
      switch mode {
      case .browser:
        return ($0.item.url.path as NSString).deletingLastPathComponent == root.url.path
      case .recursive:
        let rootPath = root.url.path.hasSuffix("/") ? root.url.path : root.url.path + "/"
        return $0.item.url.path.hasPrefix(rootPath) || $0.item.url.path == root.url.path
      }
    }
    guard !applicable.isEmpty else { return }
    let ordered = applicable.sorted { $0.index > $1.index }
    var insertedURLs: [URL] = []
    isApplyingCatalogMutation = true
    for file in ordered {
      guard !catalog.items.contains(where: { $0.url == file.item.url }) else { continue }
      catalog.insert(file.item, at: file.index)
      insertedURLs.append(file.item.url)
    }
    let inserted = Set(catalog.items.indices
      .filter { insertedURLs.contains(catalog.items[$0].url) }
      .map { IndexPath(item: $0, section: 0) })
    if !inserted.isEmpty {
      collectionView.insertItems(at: inserted)
    }
    isApplyingCatalogMutation = false
    syncAnchor()
    updateEmptyState()
    updateVisibleSelection()
    updateCarouselPreview()
    scrollSelectionIntoView()
    catalogDidChange?(catalog)
    selectionDidChange?()
    updateThumbnailDemand()
  }

  func restoreSelection(_ urls: [URL]) {
    let present = urls.filter { url in catalog.items.contains(where: { $0.url == url }) }
    guard !present.isEmpty else { return }
    do {
      try catalog.select(present[0])
      for url in present.dropFirst() {
        try catalog.selectAdditional(url)
      }
    } catch {
      preconditionFailure(error.localizedDescription)
    }
    let indexPaths = Set(present.compactMap { url in
      catalog.items.firstIndex(where: { $0.url == url }).map { IndexPath(item: $0, section: 0) }
    })
    isApplyingCatalogMutation = true
    collectionView.deselectAll(nil)
    collectionView.selectItems(at: indexPaths, scrollPosition: [])
    isApplyingCatalogMutation = false
    syncAnchor()
    updateVisibleSelection()
    updateCarouselPreview()
    scrollSelectionIntoView()
    selectionDidChange?()
  }

  func cancelWork() async {
    scanGeneration += 1
    scanTask?.cancel()
    await scanTask?.value
    await thumbnailCoordinator?.cancelAll()
  }

  var carouselPreviewImageForTesting: NSImage? { carouselPreviewImageView?.image }
  var carouselPreviewBadgeHiddenForTesting: Bool? { carouselPreviewBadgeView?.isHidden }
  var canNavigateBackForTesting: Bool { !backStack.isEmpty }
  var canNavigateForwardForTesting: Bool { !forwardStack.isEmpty }
  var columnCountForTesting: Int { columnCount() }

  func isShowingThumbnailFailureForTesting(url: URL) -> Bool {
    for indexPath in collectionView.indexPathsForVisibleItems() {
      guard
        catalog.item(at: indexPath.item)?.url == url,
        let cell = collectionView.item(at: indexPath) as? ThumbnailCell
      else { continue }
      return cell.isShowingThumbnailFailure
    }
    return false
  }

  private var captionStyle: ThumbnailCaptionStyle {
    mode == .browser ? .nameAndSize : .none
  }

  private var selectedIndex: Int? {
    guard let selectedURL = catalog.selectedURL else { return nil }
    return catalog.items.firstIndex(where: { $0.url == selectedURL })
  }

  private var emptyStateText: String {
    switch mode {
    case .browser:
      "\(root.url.path)\nNo images or videos here. Press ⌫ to go up."
    case .recursive:
      root.url.path
    }
  }

  private func startScan(selecting selection: URL? = nil, after previous: Task<Void, Never>? = nil) {
    let generation = scanGeneration
    let root = self.root
    scanTask = Task { [weak self] in
      await previous?.value
      guard let self, generation == self.scanGeneration else { return }
      self.metrics.scanStarted()
      do {
        let volume = try await ThumbnailCoordinator.volume(for: root)
        guard !Task.isCancelled, generation == self.scanGeneration else {
          throw CancellationError()
        }
        await self.thumbnailCoordinator?.setVolume(volume)
        guard !Task.isCancelled, generation == self.scanGeneration else {
          throw CancellationError()
        }
        switch self.mode {
        case .browser:
          let entries = try await DirectoryLister.list(directory: root.url)
          guard generation == self.scanGeneration else { return }
          self.applyListing(entries, selecting: selection)
          self.metrics.scanCompleted(
            DirectoryScanSummary(
              itemCount: entries.count,
              batchCount: entries.isEmpty ? 0 : 1,
              skippedDirectoryCount: 0
            )
          )
        case .recursive:
          let summary = try await DirectoryScanner.scan(root: root) { [weak self] batch in
            await self?.append(batch, generation: generation)
          }
          guard generation == self.scanGeneration else { return }
          self.metrics.scanCompleted(summary)
        }
      } catch is CancellationError {
        self.metrics.scanFailed()
      } catch {
        self.metrics.scanFailed()
        self.presentScanError(error)
      }
    }
  }

  private func append(_ batch: [MediaItem], generation: Int) {
    guard generation == scanGeneration else { return }
    let insertedIndices = catalog.append(batch)
    guard !insertedIndices.isEmpty else { return }
    let indexPaths = Set(insertedIndices.map { IndexPath(item: $0, section: 0) })
    collectionView.insertItems(at: indexPaths)
    metrics.catalogBatchApplied()
    updateEmptyState()
    collectionView.layoutSubtreeIfNeeded()
    updateThumbnailDemand()
  }

  private func applyListing(_ entries: [MediaItem], selecting selection: URL?) {
    guard !entries.isEmpty else {
      updateEmptyState()
      updateThumbnailDemand()
      return
    }
    catalog.append(entries)
    collectionView.reloadData()
    metrics.catalogBatchApplied()
    updateEmptyState()
    collectionView.layoutSubtreeIfNeeded()
    if let selection, catalog.items.contains(where: { $0.url == selection }) {
      selectCatalogItem(selection)
    } else if viewMode == .carousel, let first = catalog.items.first {
      selectCatalogItem(first.url)
    }
    updateThumbnailDemand()
  }

  private func selectCatalogItem(_ url: URL) {
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
    updateCarouselPreview()
    scrollSelectionIntoView()
    selectionDidChange?()
  }

  func setViewMode(_ mode: ViewMode) {
    guard mode != viewMode else { return }
    let window = view.window
    let windowFrame = window?.frame
    let contentSize = window.map { $0.contentRect(forFrameRect: $0.frame).size }
    viewMode = mode
    applyViewMode()
    if isViewLoaded {
      collectionView.reloadData()
      updateThumbnailDemand()
      updateCarouselPreview()
      view.layoutSubtreeIfNeeded()
      if let window, let contentSize {
        window.setContentSize(contentSize)
        DispatchQueue.main.async {
          window.setContentSize(contentSize)
        }
      } else if let window, let windowFrame {
        window.setFrame(windowFrame, display: false)
        DispatchQueue.main.async {
          window.setFrame(windowFrame, display: false)
        }
      }
      _ = window?.makeFirstResponder(collectionView)
    }
  }

  private func applyViewMode() {
    guard let rootContainer else { return }
    isApplyingViewMode = true
    defer { isApplyingViewMode = false }
    gridContainer?.subviews.forEach { $0.removeFromSuperview() }
    carouselContainer?.subviews.forEach { $0.removeFromSuperview() }
    gridContainer?.removeFromSuperview()
    carouselContainer?.removeFromSuperview()
    scrollView.removeFromSuperview()

    configureCollectionLayout(for: viewMode)

    switch viewMode {
    case .grid:
      guard let gridContainer else { return }
      scrollView.hasVerticalScroller = true
      scrollView.hasHorizontalScroller = false
      collectionView.translatesAutoresizingMaskIntoConstraints = true
      collectionView.autoresizingMask = [.width]
      gridContainer.addSubview(scrollView)
      scrollView.translatesAutoresizingMaskIntoConstraints = false
      NSLayoutConstraint.activate([
        scrollView.leadingAnchor.constraint(equalTo: gridContainer.leadingAnchor),
        scrollView.trailingAnchor.constraint(equalTo: gridContainer.trailingAnchor),
        scrollView.topAnchor.constraint(equalTo: gridContainer.topAnchor),
        scrollView.bottomAnchor.constraint(equalTo: gridContainer.bottomAnchor),
      ])
      rootContainer.addSubview(gridContainer, positioned: .below, relativeTo: emptyStateLabel)
      NSLayoutConstraint.activate([
        gridContainer.leadingAnchor.constraint(equalTo: rootContainer.leadingAnchor),
        gridContainer.trailingAnchor.constraint(equalTo: rootContainer.trailingAnchor),
        gridContainer.topAnchor.constraint(equalTo: rootContainer.topAnchor),
        gridContainer.bottomAnchor.constraint(equalTo: rootContainer.bottomAnchor),
      ])
      carouselPreviewImageView?.image = nil
    case .carousel:
      guard let carouselContainer, let previewRegion = carouselPreviewRegion else { return }
      scrollView.hasVerticalScroller = false
      scrollView.hasHorizontalScroller = true
      collectionView.translatesAutoresizingMaskIntoConstraints = true
      collectionView.autoresizingMask = [.height]

      let divider = NSBox()
      divider.boxType = .separator
      divider.translatesAutoresizingMaskIntoConstraints = false

      carouselContainer.addSubview(previewRegion)
      carouselContainer.addSubview(divider)
      carouselContainer.addSubview(scrollView)
      scrollView.translatesAutoresizingMaskIntoConstraints = false

      let stripHeight: CGFloat = captionStyle == .nameAndSize ? 184 : 148
      NSLayoutConstraint.activate([
        previewRegion.leadingAnchor.constraint(equalTo: carouselContainer.leadingAnchor),
        previewRegion.trailingAnchor.constraint(equalTo: carouselContainer.trailingAnchor),
        previewRegion.topAnchor.constraint(equalTo: carouselContainer.topAnchor),
        previewRegion.bottomAnchor.constraint(equalTo: divider.topAnchor),
        divider.leadingAnchor.constraint(equalTo: carouselContainer.leadingAnchor),
        divider.trailingAnchor.constraint(equalTo: carouselContainer.trailingAnchor),
        divider.bottomAnchor.constraint(equalTo: scrollView.topAnchor),
        scrollView.leadingAnchor.constraint(equalTo: carouselContainer.leadingAnchor),
        scrollView.trailingAnchor.constraint(equalTo: carouselContainer.trailingAnchor),
        scrollView.bottomAnchor.constraint(equalTo: carouselContainer.bottomAnchor),
        scrollView.heightAnchor.constraint(equalToConstant: stripHeight),
      ])

      rootContainer.addSubview(carouselContainer, positioned: .below, relativeTo: emptyStateLabel)
      NSLayoutConstraint.activate([
        carouselContainer.leadingAnchor.constraint(equalTo: rootContainer.leadingAnchor),
        carouselContainer.trailingAnchor.constraint(equalTo: rootContainer.trailingAnchor),
        carouselContainer.topAnchor.constraint(equalTo: rootContainer.topAnchor),
        carouselContainer.bottomAnchor.constraint(equalTo: rootContainer.bottomAnchor),
      ])
      updateCarouselPreview()
    }
    if isViewLoaded {
      _ = view.window?.makeFirstResponder(collectionView)
    }
    view.needsLayout = true
  }

  private func configureCollectionLayout(for mode: ViewMode) {
    let layout = (collectionView.collectionViewLayout as? NSCollectionViewFlowLayout) ?? NSCollectionViewFlowLayout()
    switch mode {
    case .grid:
      layout.scrollDirection = .vertical
      layout.itemSize =
        self.mode == .browser
        ? NSSize(width: 160, height: 180)
        : NSSize(width: 160, height: 160)
      layout.minimumInteritemSpacing = 10
      layout.minimumLineSpacing = 10
      layout.sectionInset = NSEdgeInsets(top: 16, left: 16, bottom: 16, right: 16)
    case .carousel:
      layout.scrollDirection = .horizontal
      layout.itemSize =
        self.mode == .browser
        ? NSSize(width: 120, height: 156)
        : NSSize(width: 120, height: 120)
      layout.minimumInteritemSpacing = 8
      layout.minimumLineSpacing = 8
      layout.sectionInset = NSEdgeInsets(top: 8, left: 12, bottom: 8, right: 12)
    }
    if collectionView.collectionViewLayout !== layout {
      collectionView.collectionViewLayout = layout
      collectionView.register(ThumbnailCell.self, forItemWithIdentifier: ThumbnailCell.reuseIdentifier)
    }
  }

  private func updateCarouselPreview() {
    guard viewMode == .carousel else { return }
    carouselPreviewGeneration &+= 1
    let generation = carouselPreviewGeneration
    guard let selectedURL = catalog.selectedURL,
      let item = catalog.items.first(where: { $0.url == selectedURL })
    else {
      carouselPreviewImageView?.image = nil
      carouselPreviewBadgeView?.isHidden = true
      setPreviewAspectRatio(1)
      return
    }
    carouselPreviewBadgeView?.isHidden = item.kind != .movie
    carouselPreviewImageView?.imageScaling =
      item.kind == .directory ? .scaleProportionallyDown : .scaleProportionallyUpOrDown
    let scale = view.window?.backingScaleFactor ?? 2
    Task { @MainActor [weak self] in
      let image = await Self.loadPreviewImage(for: item, scale: scale)
      guard let self, generation == self.carouselPreviewGeneration else { return }
      if let image {
        let size = image.size
        if size.width > 0, size.height > 0 {
          self.setPreviewAspectRatio(size.width / size.height)
        }
      }
      self.carouselPreviewImageView?.image = image
    }
  }

  private func setPreviewAspectRatio(_ ratio: CGFloat) {
    guard let imageView = carouselPreviewImageView else { return }
    let safe = ratio.isFinite && ratio > 0 ? ratio : 1
    guard let old = carouselPreviewAspectConstraint else {
      let aspect = imageView.widthAnchor.constraint(equalTo: imageView.heightAnchor, multiplier: safe)
      aspect.isActive = true
      carouselPreviewAspectConstraint = aspect
      return
    }
    old.isActive = false
    let aspect = imageView.widthAnchor.constraint(equalTo: imageView.heightAnchor, multiplier: safe)
    aspect.isActive = true
    carouselPreviewAspectConstraint = aspect
  }

  private static func loadPreviewImage(for item: MediaItem, scale: CGFloat) async -> NSImage? {
    switch item.kind {
    case .image, .movie:
      for pointSize: CGFloat in [1200, 800, 480, 320] {
        if let image = await loadThumbnailImage(for: item.url, pointSize: pointSize, scale: scale) {
          return image
        }
      }
      return nil
    case .directory:
      let pointSize: CGFloat = 256
      let request = QLThumbnailGenerator.Request(
        fileAt: item.url,
        size: NSSize(width: pointSize, height: pointSize),
        scale: scale,
        representationTypes: [.thumbnail, .icon]
      )
      request.iconMode = true
      return await withCheckedContinuation { continuation in
        QLThumbnailGenerator.shared.generateBestRepresentation(for: request) { representation, _ in
          if let cg = representation?.cgImage {
            continuation.resume(returning: NSImage(cgImage: cg, size: NSSize(width: pointSize, height: pointSize)))
          } else {
            continuation.resume(returning: nil)
          }
        }
      }
    }
  }

  private static func loadThumbnailImage(for url: URL, pointSize: CGFloat, scale: CGFloat) async -> NSImage? {
    let request = QLThumbnailGenerator.Request(
      fileAt: url,
      size: NSSize(width: pointSize, height: pointSize),
      scale: scale,
      representationTypes: .thumbnail
    )
    return await withCheckedContinuation { continuation in
      QLThumbnailGenerator.shared.generateBestRepresentation(for: request) { representation, _ in
        if let cg = representation?.cgImage {
          continuation.resume(
            returning: NSImage(
              cgImage: cg,
              size: NSSize(width: CGFloat(cg.width) / scale, height: CGFloat(cg.height) / scale)
            )
          )
        } else {
          continuation.resume(returning: nil)
        }
      }
    }
  }

  private func updateLocationPresentation() {
    view.window?.title = root.url.lastPathComponent
    view.window?.representedURL = root.url
  }

  private func updateEmptyState() {
    let isEmpty = catalog.isEmpty
    emptyStateLabel?.stringValue = emptyStateText
    gridContainer?.isHidden = isEmpty
    carouselContainer?.isHidden = isEmpty
    scrollView?.isHidden = isEmpty
    emptyStateLabel?.isHidden = !isEmpty
    if isEmpty {
      carouselPreviewGeneration &+= 1
      carouselPreviewImageView?.image = nil
    }
    restoreKeyResponder()
  }

  private func restoreKeyResponder() {
    _ = view.window?.makeFirstResponder(collectionView)
  }

  private func syncAnchor() {
    anchorIndex = catalog.selectedURL.flatMap { url in
      catalog.items.firstIndex(where: { $0.url == url })
    }
  }

  private func scrollToTop() {
    scrollDirection = .down
    lastScrollOriginY = 0
    scrollView.contentView.scroll(to: .zero)
    scrollView.reflectScrolledClipView(scrollView.contentView)
  }

  private func scrollSelectionIntoView() {
    guard let selectedURL = catalog.selectedURL,
      let index = catalog.items.firstIndex(where: { $0.url == selectedURL })
    else { return }
    let path = IndexPath(item: index, section: 0)
    collectionView.layoutSubtreeIfNeeded()
    guard let frame = collectionView.layoutAttributesForItem(at: path)?.frame else { return }
    let visible = collectionView.visibleRect
    let current = scrollView.contentView.bounds.origin
    let epsilon: CGFloat = 0.5
    switch viewMode {
    case .carousel:
      let targetX: CGFloat
      if frame.minX < visible.minX + epsilon {
        targetX = frame.minX
      } else if frame.maxX > visible.maxX - epsilon {
        targetX = frame.maxX - visible.width
      } else {
        return
      }
      guard abs(targetX - current.x) > epsilon else { return }
      scrollView.contentView.scroll(to: NSPoint(x: targetX, y: current.y))
      scrollView.reflectScrolledClipView(scrollView.contentView)
    case .grid:
      let targetY: CGFloat
      if frame.minY < visible.minY + epsilon {
        targetY = frame.minY
      } else if frame.maxY > visible.maxY - epsilon {
        targetY = frame.maxY - visible.height
      } else {
        return
      }
      guard abs(targetY - current.y) > epsilon else { return }
      scrollView.contentView.scroll(to: NSPoint(x: current.x, y: targetY))
      scrollView.reflectScrolledClipView(scrollView.contentView)
    }
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
      cell.setCaptionStyle(captionStyle)
      cell.represent(
        item,
        isSelected: catalog.selectedURLs.contains(item.url),
        image: thumbnailCoordinator?.cachedImage(for: item),
        isThumbnailFailed: thumbnailCoordinator?.isThumbnailFailed(for: item) ?? false
      )
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
      if QLPreviewPanel.sharedPreviewPanelExists(), QLPreviewPanel.shared().isVisible {
        QLPreviewPanel.shared().orderOut(nil)
        return true
      }
      return false
    }
    if event.charactersIgnoringModifiers == " ", flags.isEmpty {
      guard catalog.selectedURL != nil else { return false }
      previewHost?.togglePreview(event)
      return true
    }
    if event.keyCode == 36, flags.isEmpty, let selectedIndex {
      activateItem(at: selectedIndex)
      return true
    }
    if mode == .browser {
      if event.keyCode == 123, flags == .control {
        return navigateBack()
      }
      if event.keyCode == 124, flags == .control {
        return navigateForward()
      }
      if event.keyCode == 123, flags.isEmpty, event.modifierFlags.contains(.shift) {
        return navigateBack()
      }
      if event.keyCode == 124, flags.isEmpty, event.modifierFlags.contains(.shift) {
        return navigateForward()
      }
      if event.keyCode == 33, flags == .command {
        return navigateBack()
      }
      if event.keyCode == 30, flags == .command {
        return navigateForward()
      }
      if event.keyCode == 51, flags.isEmpty {
        return navigateToParent()
      }
      if event.keyCode == 126, flags == .command {
        return navigateToParent()
      }
      if event.keyCode == 51, flags == .command {
        guard catalog.selectedURL != nil else { return false }
        previewHost?.trashSelectedItem(event)
        return true
      }
    }
    if event.keyCode == 51, flags.isEmpty || flags == .control {
      guard catalog.selectedURL != nil else { return false }
      previewHost?.trashSelectedItem(event)
      return true
    }
    if flags.isEmpty, catalog.count > 0 {
      let columns = columnCount()
      let delta: Int?
      switch event.keyCode {
      case 123:
        delta = -1
      case 124:
        delta = 1
      case 126:
        delta = -columns
      case 125:
        delta = columns
      default:
        delta = nil
      }
      if let delta {
        let current = catalog.selectedURL.flatMap { url in catalog.items.firstIndex(where: { $0.url == url }) }
        let next: Int
        if let current {
          next = ((current + delta) % catalog.count + catalog.count) % catalog.count
        } else {
          next = delta > 0 ? 0 : catalog.count - 1
        }
        selectCatalogItem(catalog.items[next].url)
        return true
      }
    }
    return false
  }

  private func columnCount() -> Int {
    guard viewMode == .grid else { return 1 }
    guard let layout = collectionView.collectionViewLayout as? NSCollectionViewFlowLayout else { return 1 }
    let spacing = layout.minimumInteritemSpacing
    let itemStride = layout.itemSize.width + spacing
    guard itemStride > 0 else { return 1 }
    let available = collectionView.bounds.width - layout.sectionInset.left - layout.sectionInset.right + spacing
    let columns = max(1, Int(available / itemStride))
    return columns >= catalog.count ? 1 : columns
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
    if event.clickCount > 1, let clicked, let handler = keyHandler {
      handler.activateItem(at: clicked)
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
