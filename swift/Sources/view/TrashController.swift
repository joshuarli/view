import AppKit
import viewCore

@MainActor
final class TrashController {
  private let grid: GridViewController
  private let service = TrashService()
  private let recycle: TrashService.Recycle
  private var task: Task<Void, Never>?
  private var history = TrashHistory()

  var canUndo: Bool { history.canUndo }
  var canRedo: Bool { history.canRedo }

  init(grid: GridViewController, recycle: @escaping TrashService.Recycle = { source in try await NSWorkspace.shared.recycle([source]) }) {
    self.grid = grid
    self.recycle = recycle
  }

  func trashSelectedItem() {
    guard task == nil else { return }
    let urls = grid.catalog.selectedURLs
    guard !urls.isEmpty else { return }
    var snapshots: [URL: (MediaItem, Int)] = [:]
    for (index, item) in grid.catalog.items.enumerated() where urls.contains(item.url) {
      snapshots[item.url] = (item, index)
    }
    let selectionBeforeOp = urls
    let recycle = self.recycle
    task = Task {
      defer { task = nil }
      var firstError: (any Error)?
      var failed: [URL] = []
      var trashedFiles: [TrashedFile] = []
      trashedFiles.reserveCapacity(urls.count)
      var cancelled = false
      for url in urls {
        guard let (item, index) = snapshots[url] else { continue }
        do {
          let outcome = try await service.move(
            url,
            recycle: recycle,
            removeFromCatalog: { _ in }
          )
          switch outcome {
          case .removed(let trashedURL):
            trashedFiles.append(TrashedFile(item: item, trashedURL: trashedURL, index: index))
          case .alreadyInProgress:
            if firstError == nil {
              firstError = TrashError.alreadyInProgress(url)
            }
            failed.append(url)
          }
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
      if !trashedFiles.isEmpty {
        grid.removeCatalogItems(trashedFiles.map(\.item.url))
        TrashSound.play()
        history.recordTrash(TrashTransaction(files: trashedFiles, selectionBeforeOp: selectionBeforeOp))
      }
      if cancelled {
        return
      }
      if let firstError, !failed.isEmpty {
        presentError(firstError, trashed: trashedFiles.count, total: trashedFiles.count + failed.count, failed: failed)
      }
    }
  }

  func undo() {
    guard task == nil, let transaction = history.popUndo() else { return }
    let selectionBeforeUndo = grid.catalog.selectedURLs
    task = Task {
      defer { task = nil }
      var succeeded: [TrashedFile] = []
      succeeded.reserveCapacity(transaction.files.count)
      var residual: [TrashedFile] = []
      var failed: [URL] = []
      var firstError: (any Error)?
      var cancelled = false
      for (offset, file) in transaction.files.enumerated() {
        do {
          try await service.restore(trashed: file.trashedURL, to: file.item.url) { trashed, original in
            let manager = FileManager.default
            guard manager.fileExists(atPath: trashed.path) else {
              throw TrashError.missingTrashedFile(trashed)
            }
            guard !manager.fileExists(atPath: original.path) else {
              throw TrashError.destinationExists(original)
            }
            try manager.moveItem(at: trashed, to: original)
          }
          succeeded.append(file)
        } catch is CancellationError {
          cancelled = true
          residual.append(file)
          if offset + 1 < transaction.files.count {
            residual.append(contentsOf: transaction.files[(offset + 1)...])
          }
          break
        } catch {
          if firstError == nil {
            firstError = error
          }
          failed.append(file.item.url)
          if let trashError = error as? TrashError, case .missingTrashedFile = trashError {
            // The trashed file is permanently gone; do not keep it for retry.
          } else {
            residual.append(file)
          }
        }
      }
      if !succeeded.isEmpty {
        grid.restoreCatalogItems(succeeded)
        grid.restoreSelection(transaction.selectionBeforeOp)
        history.pushRedo(TrashTransaction(files: succeeded, selectionBeforeOp: selectionBeforeUndo))
      }
      if !residual.isEmpty {
        history.pushUndo(TrashTransaction(files: residual, selectionBeforeOp: transaction.selectionBeforeOp))
      }
      if cancelled {
        return
      }
      if let firstError, !failed.isEmpty {
        presentRestoreError(firstError, restored: succeeded.count, total: transaction.files.count, failed: failed)
      }
    }
  }

  func redo() {
    guard task == nil, let transaction = history.popRedo() else { return }
    let selectionBeforeRedo = grid.catalog.selectedURLs
    let recycle = self.recycle
    task = Task {
      defer { task = nil }
      var succeeded: [TrashedFile] = []
      succeeded.reserveCapacity(transaction.files.count)
      var residual: [TrashedFile] = []
      var failed: [URL] = []
      var firstError: (any Error)?
      var cancelled = false
      for (offset, file) in transaction.files.enumerated() {
        do {
          let outcome = try await service.move(
            file.item.url,
            recycle: recycle,
            removeFromCatalog: { _ in }
          )
          switch outcome {
          case .removed(let trashedURL):
            succeeded.append(TrashedFile(item: file.item, trashedURL: trashedURL, index: file.index))
          case .alreadyInProgress:
            if firstError == nil {
              firstError = TrashError.alreadyInProgress(file.item.url)
            }
            failed.append(file.item.url)
            residual.append(file)
          }
        } catch is CancellationError {
          cancelled = true
          residual.append(file)
          if offset + 1 < transaction.files.count {
            residual.append(contentsOf: transaction.files[(offset + 1)...])
          }
          break
        } catch {
          if firstError == nil {
            firstError = error
          }
          failed.append(file.item.url)
          residual.append(file)
        }
      }
      if !succeeded.isEmpty {
        grid.removeCatalogItems(succeeded.map(\.item.url))
        TrashSound.play()
        history.pushUndo(TrashTransaction(files: succeeded, selectionBeforeOp: selectionBeforeRedo))
      }
      if !residual.isEmpty {
        history.pushRedo(TrashTransaction(files: residual, selectionBeforeOp: transaction.selectionBeforeOp))
      }
      if cancelled {
        return
      }
      if let firstError, !failed.isEmpty {
        presentError(firstError, trashed: succeeded.count, total: succeeded.count + failed.count, failed: failed)
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

  private func presentRestoreError(_ error: any Error, restored: Int, total: Int, failed: [URL]) {
    let alert = NSAlert()
    alert.messageText = total > 1 ? "Couldn’t restore some items" : "Couldn’t restore item"
    alert.informativeText =
      "\(Self.restoreSummary(restored: restored, total: total, failed: failed)) \(error.localizedDescription)"
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

  static func restoreSummary(restored: Int, total: Int, failed: [URL]) -> String {
    var text = "\(restored) of \(total) item\(total == 1 ? "" : "s") restored."
    if let first = failed.first {
      text += " First item that could not be restored: \(first.lastPathComponent)."
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
