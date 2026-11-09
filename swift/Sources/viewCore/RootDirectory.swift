import Foundation

public enum LaunchError: Error, Equatable, Sendable {
  case notFound(String)
  case notDirectory(String)
  case unknownOption(String)
  case unexpectedArgument(String)
}

extension LaunchError: LocalizedError {
  public var errorDescription: String? {
    switch self {
    case .notFound(let path): "no such directory: \(path)"
    case .notDirectory(let path): "not a directory: \(path)"
    case .unknownOption(let option): "unknown option: \(option)"
    case .unexpectedArgument(let argument): "unexpected argument: \(argument)"
    }
  }
}

public struct RootDirectory: Sendable, Equatable {
  public let url: URL

  public init(url: URL) {
    self.url = url
  }

  public static func resolve(argument: String?, currentDirectory: String) throws -> RootDirectory {
    let current = URL(filePath: currentDirectory).standardizedFileURL
    let requested = argument.map { ($0 as NSString).expandingTildeInPath } ?? current.path
    let candidate =
      requested.hasPrefix("/")
      ? URL(filePath: requested)
      : current.appending(path: requested)
    let url = candidate.standardizedFileURL

    var isDirectory: ObjCBool = false
    guard FileManager.default.fileExists(atPath: url.path, isDirectory: &isDirectory) else {
      throw LaunchError.notFound(url.path)
    }
    guard isDirectory.boolValue else {
      throw LaunchError.notDirectory(url.path)
    }
    return RootDirectory(url: url)
  }
}
