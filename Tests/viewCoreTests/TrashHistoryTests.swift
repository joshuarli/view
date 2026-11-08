import Foundation
import Testing
import viewCore

@Suite("Trash history")
struct TrashHistoryTests {
  @Test func newTrashClearsRedo() {
    var history = TrashHistory()
    history.pushUndo(transaction(named: "a"))
    history.pushRedo(transaction(named: "b"))
    #expect(history.canUndo)
    #expect(history.canRedo)

    history.recordTrash(transaction(named: "c"))
    #expect(history.canUndo)
    #expect(history.canRedo == false)
    #expect(history.popUndo()?.files.first?.item.url.lastPathComponent == "c.png")
    #expect(history.popUndo()?.files.first?.item.url.lastPathComponent == "a.png")
  }

  @Test func popOnEmptyStacksReturnsNil() {
    var history = TrashHistory()
    #expect(history.popUndo() == nil)
    #expect(history.popRedo() == nil)
    #expect(history.canUndo == false)
    #expect(history.canRedo == false)
  }

  @Test func undoRedoRoundTripKeepsResidual() {
    var history = TrashHistory()
    history.pushUndo(transaction(named: "a"))
    let popped = history.popUndo()
    #expect(popped?.files.first?.item.url.lastPathComponent == "a.png")
    history.pushRedo(transaction(named: "a"))
    #expect(history.canRedo)
    let redone = history.popRedo()
    #expect(redone?.files.first?.item.url.lastPathComponent == "a.png")
    #expect(history.canUndo == false)
    #expect(history.canRedo == false)
  }

  private func transaction(named name: String) -> TrashTransaction {
    let url = URL(filePath: "/history/\(name).png")
    return TrashTransaction(
      files: [TrashedFile(
        item: MediaItem(url: url, kind: .image, modificationDate: nil, fileSize: nil),
        trashedURL: URL(filePath: "/trash/\(name).png"),
        index: 0
      )],
      selectionBeforeOp: [url]
    )
  }
}
