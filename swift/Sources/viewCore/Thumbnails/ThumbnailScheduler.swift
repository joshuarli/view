import CoreGraphics
import Foundation

public enum ThumbnailVolume: Sendable, Equatable {
  case local
  case other

  var concurrencyLimit: Int {
    switch self {
    case .local: 4
    case .other: 2
    }
  }
}

public enum ThumbnailResult: Sendable {
  case image(CGImage)
  case failure(String)
  case cancelled
}

@MainActor
public final class ThumbnailScheduler {
  public typealias Request = @Sendable @concurrent (ThumbnailKey, UInt64) async throws -> CGImage
  public typealias ResultSink = @MainActor @Sendable (ThumbnailKey, UInt64, ThumbnailResult) -> Void

  private let concurrencyLimit: Int
  private let cache: ThumbnailCache
  private let request: Request
  private let resultSink: ResultSink
  private var visibleDemand: [ThumbnailKey] = []
  private var prefetchDemand: [ThumbnailKey] = []
  private var demandGenerations: [ThumbnailKey: UInt64] = [:]
  private var settledGenerations: [ThumbnailKey: UInt64] = [:]
  private var activeRequests: [ThumbnailKey: ActiveThumbnailRequest] = [:]
  private var failedKeys: Set<ThumbnailKey> = []
  private var removedURLs: Set<URL> = []
  private var nextGeneration: UInt64 = 0
  private var stopped = false
  private var isPumping = false

  public private(set) var requestCount = 0
  public private(set) var maximumConcurrentRequestCount = 0

  public convenience init(
    volume: ThumbnailVolume,
    cache: ThumbnailCache,
    request: @escaping Request,
    resultSink: @escaping ResultSink
  ) {
    self.init(
      concurrencyLimit: volume.concurrencyLimit,
      cache: cache,
      request: request,
      resultSink: resultSink
    )
  }

  init(
    concurrencyLimit: Int,
    cache: ThumbnailCache,
    request: @escaping Request,
    resultSink: @escaping ResultSink
  ) {
    precondition(concurrencyLimit > 0)
    self.concurrencyLimit = concurrencyLimit
    self.cache = cache
    self.request = request
    self.resultSink = resultSink
  }

  public var activeRequestCount: Int { activeRequests.count }

  public func updateDemand(visible: [ThumbnailKey], prefetch: [ThumbnailKey]) {
    guard !stopped else { return }
    let nextVisible = uniqueKeys(visible)
    let previouslyVisible = Set(visibleDemand)
    let newlyVisible = nextVisible.filter { !previouslyVisible.contains($0) }
    let visibleKeys = Set(nextVisible)
    let nextPrefetch = uniqueKeys(prefetch).filter { !visibleKeys.contains($0) }
    let nextDemand = Set(nextVisible).union(nextPrefetch)

    for key in Array(demandGenerations.keys) where !nextDemand.contains(key) {
      demandGenerations.removeValue(forKey: key)
      settledGenerations.removeValue(forKey: key)
      cancelActiveRequest(for: key)
    }

    for key in nextVisible + nextPrefetch where demandGenerations[key] == nil {
      nextGeneration += 1
      demandGenerations[key] = nextGeneration
    }

    visibleDemand = nextVisible
    prefetchDemand = nextPrefetch
    preemptPrefetchForVisibleDemand(newlyVisible)
    pump()
  }

  public func removeItem(at url: URL) {
    removedURLs.insert(url)
    let matchingKeys = demandGenerations.keys.filter { $0.url == url }
    for key in matchingKeys {
      demandGenerations.removeValue(forKey: key)
      settledGenerations.removeValue(forKey: key)
      cancelActiveRequest(for: key)
    }
    visibleDemand.removeAll { $0.url == url }
    prefetchDemand.removeAll { $0.url == url }
    cache.removeItems(at: url)
    pump()
  }

  public func cancelAll() async {
    stopped = true
    visibleDemand.removeAll(keepingCapacity: false)
    prefetchDemand.removeAll(keepingCapacity: false)
    demandGenerations.removeAll(keepingCapacity: false)
    settledGenerations.removeAll(keepingCapacity: false)
    let tasks = activeRequests.values.map(\.task)
    activeRequests.removeAll(keepingCapacity: false)
    tasks.forEach { $0.cancel() }
    for task in tasks {
      await task.value
    }
  }

  private func uniqueKeys(_ keys: [ThumbnailKey]) -> [ThumbnailKey] {
    var seen: Set<ThumbnailKey> = []
    return keys.filter { key in
      !removedURLs.contains(key.url) && seen.insert(key).inserted
    }
  }

  private func pump() {
    guard !stopped, !isPumping else { return }
    isPumping = true
    defer { isPumping = false }

    for key in visibleDemand + prefetchDemand {
      guard let generation = demandGenerations[key] else { continue }
      guard settledGenerations[key] != generation, !failedKeys.contains(key) else { continue }

      if let image = cache.image(for: key) {
        settledGenerations[key] = generation
        resultSink(key, generation, .image(image))
        continue
      }

      guard activeRequests[key] == nil else { continue }
      guard activeRequests.count < concurrencyLimit else { continue }
      startRequest(for: key, generation: generation)
    }
  }

  private func startRequest(for key: ThumbnailKey, generation: UInt64) {
    let request = self.request
    let task = Task { @MainActor [weak self] in
      let result: ThumbnailResult
      do {
        let image = try await request(key, generation)
        result = Task.isCancelled ? .cancelled : .image(image)
      } catch is CancellationError {
        result = .cancelled
      } catch {
        result = .failure(error.localizedDescription)
      }
      self?.finishRequest(for: key, generation: generation, result: result)
    }
    activeRequests[key] = ActiveThumbnailRequest(generation: generation, task: task)
    requestCount += 1
    maximumConcurrentRequestCount = max(maximumConcurrentRequestCount, activeRequests.count)
  }

  private func preemptPrefetchForVisibleDemand(_ newlyVisible: [ThumbnailKey]) {
    guard activeRequests.count >= concurrencyLimit else { return }
    let hasWaitingVisibleRequest = newlyVisible.contains { key in
      guard let generation = demandGenerations[key] else { return false }
      guard settledGenerations[key] != generation, !failedKeys.contains(key) else { return false }
      if activeRequests[key]?.generation == generation { return false }
      return cache.image(for: key) == nil
    }
    guard hasWaitingVisibleRequest else { return }

    guard let prefetchKey = prefetchDemand.reversed().first(where: { key in
      guard let request = activeRequests[key] else { return false }
      guard let generation = demandGenerations[key] else { return false }
      return request.generation == generation && !request.cancellationRequested
    }) else { return }
    cancelActiveRequest(for: prefetchKey)
  }

  private func cancelActiveRequest(for key: ThumbnailKey) {
    guard var request = activeRequests[key], !request.cancellationRequested else { return }
    request.cancellationRequested = true
    activeRequests[key] = request
    request.task.cancel()
  }

  private func finishRequest(for key: ThumbnailKey, generation: UInt64, result: ThumbnailResult) {
    guard activeRequests[key]?.generation == generation else { return }
    activeRequests.removeValue(forKey: key)
    guard !stopped, demandGenerations[key] == generation, !removedURLs.contains(key.url) else {
      pump()
      return
    }

    switch result {
    case .image(let image):
      cache.insert(image, for: key)
      settledGenerations[key] = generation
      resultSink(key, generation, result)
    case .failure:
      failedKeys.insert(key)
      settledGenerations[key] = generation
      resultSink(key, generation, result)
    case .cancelled:
      break
    }
    pump()
  }
}

private struct ActiveThumbnailRequest {
  let generation: UInt64
  let task: Task<Void, Never>
  var cancellationRequested = false
}
