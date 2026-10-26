import AppKit
import QuickLookUI
import viewCore

@MainActor
final class PreviewPanelController: NSViewController, NSMenuItemValidation {
  private let grid: GridViewController
  private let trashController: TrashController
  private var session = PreviewSession()
  private var isControllingPanel = false
  private var isUpdatingPanel = false
  private var currentIndexObservation: NSKeyValueObservation?

  init(root: RootDirectory, metrics: ViewMetrics) {
    let grid = GridViewController(root: root, metrics: metrics)
    self.grid = grid
    trashController = TrashController(grid: grid)
    super.init(nibName: nil, bundle: nil)
    grid.previewHost = self
    grid.catalogItemDidRemove = { [weak self] catalog in
      self?.catalogItemDidRemove(catalog)
    }
    grid.selectionDidChange = { [weak self] in
      self?.previewSelectionDidChange()
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
      currentIndexObservation = panel.observe(\.currentPreviewItemIndex, options: [.new]) { [weak self] panel, _ in
        MainActor.assumeIsolated {
          self?.synchronizeSelection(from: panel)
        }
      }
      isUpdatingPanel = true
      panel.reloadData()
      if let currentIndex = session.currentIndex {
        panel.currentPreviewItemIndex = currentIndex
      }
      isUpdatingPanel = false
    }
  }

  nonisolated override func endPreviewPanelControl(_ panel: QLPreviewPanel!) {
    MainActor.assumeIsolated {
      synchronizeSelection(from: panel)
      currentIndexObservation = nil
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
      guard isCurrentController(panel) else { return }
      synchronizeSelection(from: panel)
      panel.orderOut(sender)
    } else {
      panel.updateController()
      panel.makeKeyAndOrderFront(sender)
    }
  }

  @objc func trashSelectedItem(_ sender: Any?) {
    if isControllingPanel, let panel = QLPreviewPanel.shared() {
      synchronizeSelection(from: panel)
    }
    trashController.trashSelectedItem()
  }

  func cancelWork() async {
    await grid.cancelWork()
    await trashController.waitForPendingOperation()
  }

  func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
    guard
      menuItem.action == #selector(togglePreview(_:)) || menuItem.action == #selector(trashSelectedItem(_:))
    else { return true }
    return grid.catalog.selectedURL != nil
  }

  func previewSelectionDidChange() {
    guard QLPreviewPanel.sharedPreviewPanelExists(), let panel = QLPreviewPanel.shared() else { return }
    guard !isUpdatingPanel else { return }
    guard isControllingPanel, isCurrentController(panel) else {
      panel.updateController()
      return
    }
    guard let selectedURL = grid.catalog.selectedURL else { return }
    if let index = session.index(of: selectedURL) {
      guard session.currentIndex != index else { return }
      _ = session.selectPreviewItem(at: index)
      isUpdatingPanel = true
      panel.currentPreviewItemIndex = index
      isUpdatingPanel = false
    } else {
      guard session.begin(with: grid.catalog) else { return }
      isUpdatingPanel = true
      panel.reloadData()
      if let currentIndex = session.currentIndex {
        panel.currentPreviewItemIndex = currentIndex
      }
      isUpdatingPanel = false
    }
  }

  private func catalogItemDidRemove(_ catalog: Catalog) {
    guard
      isControllingPanel,
      let panel = QLPreviewPanel.shared(),
      isCurrentController(panel)
    else { return }
    let previousSession = session
    session.update(with: catalog)
    guard session != previousSession else { return }
    isUpdatingPanel = true
    defer { isUpdatingPanel = false }
    guard session.count > 0 else {
      panel.orderOut(nil)
      return
    }
    panel.reloadData()
    if let currentIndex = session.currentIndex {
      panel.currentPreviewItemIndex = currentIndex
      if let url = session.previewItem(at: currentIndex) {
        grid.selectFromPreview(url)
      }
    }
  }

  private func synchronizeSelection(from panel: QLPreviewPanel) {
    guard
      isControllingPanel,
      !isUpdatingPanel,
      let url = session.selectPreviewItem(at: panel.currentPreviewItemIndex)
    else { return }
    grid.selectFromPreview(url)
  }

  private func isCurrentController(_ panel: QLPreviewPanel) -> Bool {
    let controller = panel.currentController as AnyObject?
    return controller === self || controller === grid
  }
}

extension PreviewPanelController: @MainActor QLPreviewPanelDataSource {
  func numberOfPreviewItems(in panel: QLPreviewPanel) -> Int {
    session.count
  }

  func previewPanel(_ panel: QLPreviewPanel, previewItemAt index: Int) -> QLPreviewItem? {
    session.previewItem(at: index).map { $0 as NSURL }
  }
}

extension PreviewPanelController: @MainActor QLPreviewPanelDelegate {
  func windowWillClose(_ notification: Notification) {
    guard let panel = notification.object as? QLPreviewPanel else { return }
    synchronizeSelection(from: panel)
  }

  func previewPanel(_ panel: QLPreviewPanel, handle event: NSEvent) -> Bool {
    guard isCurrentController(panel) else { return false }
    guard event.type == .keyDown else { return false }
    let flags = event.modifierFlags.intersection([.command, .control, .option])
    if event.keyCode == 53 {
      panel.orderOut(nil)
      return true
    }
    if event.charactersIgnoringModifiers == " ", flags.isEmpty {
      togglePreview(event)
      return true
    }
    if event.keyCode == 51, flags.isEmpty || flags == .control {
      trashSelectedItem(event)
      return true
    }
    return false
  }
}
