import Foundation

public enum TrashOutcome: Sendable, Equatable {
  case removed(trashedURL: URL)
  case alreadyInProgress
}

public enum TrashError: Error, Equatable, Sendable {
  case missingRecycleMapping(URL)
  case alreadyInProgress(URL)
  case destinationExists(URL)
  case missingTrashedFile(URL)
}

extension TrashError: LocalizedError {
  public var errorDescription: String? {
    switch self {
    case .missingRecycleMapping(let url): "Trash did not return a recycled location for \(url.path)"
    case .alreadyInProgress(let url): "A trash operation for \(url.path) is already in progress"
    case .destinationExists(let url): "A file already exists at \(url.path)"
    case .missingTrashedFile(let url): "The trashed file \(url.path) could not be found"
    }
  }
}

@MainActor
public final class TrashService {
  public typealias Recycle = @MainActor @Sendable (URL) async throws -> [URL: URL]

  private var inFlightURLs: Set<URL> = []

  public init() {}

  public func move(
    _ url: URL,
    recycle: @escaping Recycle,
    removeFromCatalog: (URL) -> Void
  ) async throws -> TrashOutcome {
    guard inFlightURLs.insert(url).inserted else { return .alreadyInProgress }
    defer { inFlightURLs.remove(url) }

    let mapping = try await recycle(url)
    guard let trashedURL = mapping[url] else { throw TrashError.missingRecycleMapping(url) }
    removeFromCatalog(url)
    return .removed(trashedURL: trashedURL)
  }

  public typealias MoveFile = @MainActor @Sendable (URL, URL) async throws -> Void

  public func restore(
    trashed trashedURL: URL,
    to original: URL,
    move: @escaping MoveFile
  ) async throws {
    guard inFlightURLs.insert(original).inserted else {
      throw TrashError.alreadyInProgress(original)
    }
    defer { inFlightURLs.remove(original) }
    try await move(trashedURL, original)
  }
}
