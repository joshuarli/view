import AppKit
import QuickLookUI
import viewCore

@MainActor
final class PreviewPanelController: NSViewController, NSMenuItemValidation {
  private let grid: GridViewController
  private let trashController: TrashController
  private var session = PreviewSession()
  private var isControllingPanel = false

  init(root: RootDirectory, metrics: ViewMetrics) {
    let grid = GridViewController(root: root, metrics: metrics)
    self.grid = grid
    trashController = TrashController(grid: grid)
    super.init(nibName: nil, bundle: nil)
    grid.catalogDidChange = { [weak self] catalog in
      self?.catalogDidChange(catalog)
    }
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    fatalError("init(coder:) is unavailable")
  }

  override func loadView() {
    addChild(grid)
    let container = NSView()
    grid.view.translatesAutoresizingMaskIntoConstraints = false
    container.addSubview(grid.view)
    NSLayoutConstraint.activate([
      grid.view.leadingAnchor.constraint(equalTo: container.leadingAnchor),
      grid.view.trailingAnchor.constraint(equalTo: container.trailingAnchor),
      grid.view.topAnchor.constraint(equalTo: container.topAnchor),
      grid.view.bottomAnchor.constraint(equalTo: container.bottomAnchor),
    ])
    view = container
  }

  nonisolated override func acceptsPreviewPanelControl(_ panel: QLPreviewPanel!) -> Bool {
    MainActor.assumeIsolated { grid.catalog.selectedURL != nil }
  }

  nonisolated override func beginPreviewPanelControl(_ panel: QLPreviewPanel!) {
    MainActor.assumeIsolated {
      guard session.begin(with: grid.catalog) else {
        panel.orderOut(nil)
        return
      }
      isControllingPanel = true
      panel.dataSource = self
      panel.delegate = self
      panel.reloadData()
      if let currentIndex = session.currentIndex {
        panel.currentPreviewItemIndex = currentIndex
      }
    }
  }

  nonisolated override func endPreviewPanelControl(_ panel: QLPreviewPanel!) {
    MainActor.assumeIsolated {
      if (panel.dataSource as AnyObject?) === self {
        panel.dataSource = nil
      }
      if (panel.delegate as AnyObject?) === self {
        panel.delegate = nil
      }
      isControllingPanel = false
      session.end()
    }
  }

  @objc func togglePreview(_ sender: Any?) {
    guard grid.catalog.selectedURL != nil, let panel = QLPreviewPanel.shared() else { return }
    if panel.isVisible {
      guard (panel.currentController as AnyObject?) === self else { return }
      panel.orderOut(sender)
    } else {
      panel.makeKeyAndOrderFront(sender)
    }
  }

  @objc func trashSelectedItem(_ sender: Any?) {
    trashController.trashSelectedItem()
  }

  func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
    guard
      menuItem.action == #selector(togglePreview(_:)) || menuItem.action == #selector(trashSelectedItem(_:))
    else { return true }
    return grid.catalog.selectedURL != nil
  }

  private func catalogDidChange(_ catalog: Catalog) {
    guard
      isControllingPanel,
      let panel = QLPreviewPanel.shared(),
      (panel.currentController as AnyObject?) === self
    else { return }
    session.update(with: catalog)
    guard session.count > 0 else {
      panel.orderOut(nil)
      return
    }
    panel.reloadData()
    if let currentIndex = session.currentIndex {
      panel.currentPreviewItemIndex = currentIndex
    }
  }
}

extension PreviewPanelController: @MainActor QLPreviewPanelDataSource {
  func numberOfPreviewItems(in panel: QLPreviewPanel) -> Int {
    session.count
  }

  func previewPanel(_ panel: QLPreviewPanel, previewItemAt index: Int) -> QLPreviewItem? {
    guard let url = session.previewItem(at: index) else { return nil }
    grid.selectFromPreview(url)
    return url as NSURL
  }
}

extension PreviewPanelController: @MainActor QLPreviewPanelDelegate {}
