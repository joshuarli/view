public enum GridKeyModifier: Hashable, Sendable {
  case command
  case control
  case option
  case shift
}

public struct GridShortcut: Sendable, Equatable {
  public let keyEquivalent: String
  public let modifiers: Set<GridKeyModifier>

  init(keyEquivalent: String, modifiers: Set<GridKeyModifier>) {
    self.keyEquivalent = keyEquivalent
    self.modifiers = modifiers
  }
}

public enum GridCommand: CaseIterable, Sendable, Equatable {
  case togglePreview

  public var shortcut: GridShortcut {
    switch self {
    case .togglePreview:
      GridShortcut(keyEquivalent: " ", modifiers: [])
    }
  }

  public static func resolve(
    keyEquivalent: String,
    modifiers: Set<GridKeyModifier>
  ) -> GridCommand? {
    allCases.first { command in
      command.shortcut.keyEquivalent == keyEquivalent && command.shortcut.modifiers == modifiers
    }
  }
}
