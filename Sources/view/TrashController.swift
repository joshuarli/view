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
    guard task == nil else { return }
    let urls = grid.catalog.selectedURLs
    guard !urls.isEmpty else { return }
    let grid = self.grid
    task = Task {
      defer { task = nil }
      var firstError: (any Error)?
      var trashedCount = 0
      for url in urls {
        do {
          _ = try await service.move(
            url,
            recycle: { source in try await NSWorkspace.shared.recycle([source]) },
            removeFromCatalog: { source in grid.removeCatalogItem(source) }
          )
          trashedCount += 1
        } catch is CancellationError {
          return
        } catch {
          if firstError == nil {
            firstError = error
          }
        }
      }
      if trashedCount > 0 {
        TrashSound.play()
      }
      if let firstError {
        presentError(firstError)
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

@MainActor
private enum TrashSound {
  private static let candidates = [
    "/System/Library/Components/CoreAudio.component/Contents/SharedSupport/SystemSounds/dock/drag to trash.aif",
  ]

  private static var player: NSSound?

  static func play() {
    for path in candidates {
      guard FileManager.default.fileExists(atPath: path) else { continue }
      guard let sound = NSSound(contentsOfFile: path, byReference: true) else { continue }
      player = sound
      sound.play()
      return
    }
  }
}
