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
      var failed: [URL] = []
      var trashed: [URL] = []
      trashed.reserveCapacity(urls.count)
      var cancelled = false
      for url in urls {
        do {
          _ = try await service.move(
            url,
            recycle: { source in try await NSWorkspace.shared.recycle([source]) },
            removeFromCatalog: { _ in }
          )
          trashed.append(url)
        } catch is CancellationError {
          cancelled = true
          break
        } catch {
          if firstError == nil {
            firstError = error
          }
          failed.append(url)
        }
      }
      if !trashed.isEmpty {
        grid.removeCatalogItems(trashed)
        TrashSound.play()
      }
      if cancelled {
        return
      }
      if let firstError, !failed.isEmpty {
        presentError(firstError, trashed: trashed.count, total: trashed.count + failed.count, failed: failed)
      }
    }
  }

  func waitForPendingOperation() async {
    await task?.value
  }

  private func presentError(_ error: any Error, trashed: Int, total: Int, failed: [URL]) {
    let alert = NSAlert()
    alert.messageText = total > 1 ? "Couldn’t move some items to Trash" : "Couldn’t move item to Trash"
    alert.informativeText =
      "\(Self.summary(trashed: trashed, total: total, failed: failed)) \(error.localizedDescription)"
    guard let window = grid.view.window else {
      NSApp.presentError(error)
      return
    }
    alert.beginSheetModal(for: window) { _ in }
  }

  static func summary(trashed: Int, total: Int, failed: [URL]) -> String {
    var text = "\(trashed) of \(total) item\(total == 1 ? "" : "s") moved to Trash."
    if let first = failed.first {
      text += " First item that could not be moved: \(first.lastPathComponent)."
    }
    return text
  }
}

@MainActor
private enum TrashSound {
  private static let candidates = [
    "/System/Library/Components/CoreAudio.component/Contents/SharedSupport/SystemSounds/dock/drag to trash.aif",
  ]

  private static var cachedSound: NSSound?

  static func play() {
    if cachedSound == nil {
      for path in candidates where FileManager.default.fileExists(atPath: path) {
        if let sound = NSSound(contentsOfFile: path, byReference: true) {
          cachedSound = sound
          break
        }
      }
    }
    cachedSound?.play()
  }
}
