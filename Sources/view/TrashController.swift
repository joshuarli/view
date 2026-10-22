import AppKit
import viewCore

@MainActor
final class TrashController {
  private let grid: GridViewController
  private let service = TrashService()
  private var task: Task<Void, Never>?

  init(grid: GridViewController) {
    self.grid = grid
  }

  func trashSelectedItem() {
    guard task == nil, let url = grid.catalog.selectedURL else { return }
    let grid = self.grid
    task = Task {
      defer { task = nil }
      do {
        _ = try await service.move(
          url,
          recycle: { source in try await NSWorkspace.shared.recycle([source]) },
          removeFromCatalog: { source in grid.removeCatalogItem(source) }
        )
      } catch is CancellationError {
        return
      } catch {
        presentError(error)
      }
    }
  }

  func waitForPendingOperation() async {
    await task?.value
  }

  private func presentError(_ error: any Error) {
    let alert = NSAlert(error: error)
    guard let window = grid.view.window else {
      NSApp.presentError(error)
      return
    }
    alert.beginSheetModal(for: window) { _ in }
  }
}
