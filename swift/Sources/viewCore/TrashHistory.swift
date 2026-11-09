import Foundation

public struct TrashedFile: Sendable, Equatable {
  public let item: MediaItem
  public let trashedURL: URL
  public let index: Int

  public init(item: MediaItem, trashedURL: URL, index: Int) {
    self.item = item
    self.trashedURL = trashedURL
    self.index = index
  }
}

public struct TrashTransaction: Sendable, Equatable {
  public let files: [TrashedFile]
  public let selectionBeforeOp: [URL]

  public init(files: [TrashedFile], selectionBeforeOp: [URL]) {
    self.files = files
    self.selectionBeforeOp = selectionBeforeOp
  }
}

public struct TrashHistory: Sendable, Equatable {
  private var undoStack: [TrashTransaction] = []
  private var redoStack: [TrashTransaction] = []

  public init() {}

  public var canUndo: Bool { !undoStack.isEmpty }
  public var canRedo: Bool { !redoStack.isEmpty }

  public mutating func recordTrash(_ transaction: TrashTransaction) {
    guard !transaction.files.isEmpty else { return }
    undoStack.append(transaction)
    redoStack.removeAll()
  }

  public mutating func pushUndo(_ transaction: TrashTransaction) {
    guard !transaction.files.isEmpty else { return }
    undoStack.append(transaction)
  }

  public mutating func pushRedo(_ transaction: TrashTransaction) {
    guard !transaction.files.isEmpty else { return }
    redoStack.append(transaction)
  }

  public mutating func popUndo() -> TrashTransaction? {
    undoStack.popLast()
  }

  public mutating func popRedo() -> TrashTransaction? {
    redoStack.popLast()
  }
}
