import Testing
import viewCore

@Suite("GridCommand")
struct GridCommandTests {
  @Test func plainSpaceTogglesPreview() {
    #expect(GridCommand.resolve(keyEquivalent: " ", modifiers: []) == .togglePreview)
  }

  @Test func modifiedSpaceHasNoCommand() {
    #expect(GridCommand.resolve(keyEquivalent: " ", modifiers: [.command]) == nil)
    #expect(GridCommand.resolve(keyEquivalent: " ", modifiers: [.control]) == nil)
    #expect(GridCommand.resolve(keyEquivalent: " ", modifiers: [.option]) == nil)
    #expect(GridCommand.resolve(keyEquivalent: " ", modifiers: [.shift]) == nil)
  }

  @Test func controlDeleteMovesSelectionToTrash() {
    #expect(GridCommand.resolve(keyEquivalent: "\u{8}", modifiers: [.control]) == .trashSelectedItem)
  }

  @Test func commandDeleteAndUnmodifiedDeleteHaveNoCommand() {
    #expect(GridCommand.resolve(keyEquivalent: "\u{8}", modifiers: [.command]) == nil)
    #expect(GridCommand.resolve(keyEquivalent: "\u{8}", modifiers: []) == nil)
  }
}
