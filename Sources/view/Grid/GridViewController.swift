import AppKit
import viewCore

@MainActor
final class GridViewController: NSViewController, NSCollectionViewDataSource, NSCollectionViewDelegate {
  private let root: RootDirectory
  private var catalog = Catalog()
  private var scanTask: Task<Void, Never>?
  private var scrollView: NSScrollView!
  private var collectionView: NSCollectionView!
  private var emptyStateLabel: NSTextField!

  init(root: RootDirectory) {
    self.root = root
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
    guard scanTask == nil else { return }
    startScan()
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

  private func startScan() {
    let root = self.root
    scanTask = Task { [weak self, root] in
      do {
        _ = try await DirectoryScanner.scan(root: root) { [weak self] batch in
          await self?.append(batch)
        }
      } catch is CancellationError {
      } catch {
        self?.presentScanError(error)
      }
    }
  }

  private func append(_ batch: [MediaItem]) {
    let insertedIndices = catalog.append(batch)
    guard !insertedIndices.isEmpty else { return }
    let indexPaths = Set(insertedIndices.map { IndexPath(item: $0, section: 0) })
    collectionView.insertItems(at: indexPaths)
    updateEmptyState()
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

  private func presentScanError(_ error: any Error) {
    let alert = NSAlert(error: error)
    guard let window = view.window else {
      NSApp.presentError(error)
      return
    }
    alert.beginSheetModal(for: window) { _ in }
  }
}
