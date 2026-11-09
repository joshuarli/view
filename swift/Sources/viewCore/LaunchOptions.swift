import Foundation

public enum BrowserMode: Sendable, Equatable {
  case browser
  case recursive
}

public struct LaunchOptions: Sendable, Equatable {
  public let root: RootDirectory
  public let mode: BrowserMode

  public init(root: RootDirectory, mode: BrowserMode) {
    self.root = root
    self.mode = mode
  }

  public static func resolve(
    arguments: [String],
    currentDirectory: String
  ) throws -> LaunchOptions {
    var mode = BrowserMode.browser
    var directoryArgument: String?
    var parsesOptions = true

    for argument in arguments {
      if parsesOptions {
        switch argument {
        case "-r", "--recursive":
          mode = .recursive
          continue
        case "--":
          parsesOptions = false
          continue
        default:
          if argument.hasPrefix("-"), argument != "-" {
            throw LaunchError.unknownOption(argument)
          }
        }
      }
      guard directoryArgument == nil else {
        throw LaunchError.unexpectedArgument(argument)
      }
      directoryArgument = argument
    }

    return LaunchOptions(
      root: try RootDirectory.resolve(
        argument: directoryArgument,
        currentDirectory: currentDirectory
      ),
      mode: mode
    )
  }
}
