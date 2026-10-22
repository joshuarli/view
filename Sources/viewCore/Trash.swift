import Foundation

public enum TrashOutcome: Sendable, Equatable {
  case removed
  case alreadyInProgress
}

public enum TrashError: Error, Equatable, Sendable {
  case missingRecycleMapping(URL)
}

extension TrashError: LocalizedError {
  public var errorDescription: String? {
    switch self {
    case .missingRecycleMapping(let url): "Trash did not return a recycled location for \(url.path)"
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
    guard mapping[url] != nil else { throw TrashError.missingRecycleMapping(url) }
    removeFromCatalog(url)
    return .removed
  }
}
