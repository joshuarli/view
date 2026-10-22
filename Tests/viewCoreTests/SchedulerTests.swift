import CoreGraphics
import Foundation
import Synchronization
import Testing
@testable import viewCore

@Suite("Scheduler")
@MainActor
struct SchedulerTests {
  @Test func capsConcurrentRequests() async {
    let cache = ThumbnailCache(totalCostLimit: 1024)
    let requests = RequestHarness()
    let results = ResultRecorder()
    let scheduler = ThumbnailScheduler(
      concurrencyLimit: 2,
      cache: cache,
      request: { key, generation in try await requests.request(key, generation: generation) },
      resultSink: { results.record(key: $0, generation: $1, result: $2) }
    )
    let keys = [key("one.png"), key("two.png"), key("three.png")]

    scheduler.updateDemand(visible: keys, prefetch: [])
    await requests.waitForStartCount(2)

    #expect(scheduler.activeRequestCount == 2)
    #expect(await requests.maximumConcurrentCount == 2)
    let started = await requests.startedRequests
    #expect(started.map(\.key) == Array(keys.prefix(2)))
    await finishSchedulerRequests(scheduler, requests: requests)
  }

  @Test func newlyVisibleDemandPreemptsOneRelevantPrefetchWithoutReleasingItsSlot() async {
    let cache = ThumbnailCache(totalCostLimit: 1024)
    let requests = RequestHarness()
    let results = ResultRecorder()
    let scheduler = ThumbnailScheduler(
      concurrencyLimit: 3,
      cache: cache,
      request: { key, generation in try await requests.requestIgnoringCancellation(key, generation: generation) },
      resultSink: { results.record(key: $0, generation: $1, result: $2) }
    )
    let firstPrefetch = key("first-prefetch.png")
    let secondPrefetch = key("second-prefetch.png")
    let thirdPrefetch = key("third-prefetch.png")
    let firstVisible = key("first-visible.png")
    let secondVisible = key("second-visible.png")
    let prefetch = [firstPrefetch, secondPrefetch, thirdPrefetch]

    scheduler.updateDemand(visible: [], prefetch: prefetch)
    await requests.waitForStartCount(3)
    let startedPrefetch = await requests.startedRequests
    scheduler.updateDemand(visible: [firstVisible, secondVisible], prefetch: prefetch)
    #expect(await requests.cancellationRequests == [startedPrefetch[2]])
    #expect(scheduler.activeRequestCount == 3)
    #expect(scheduler.requestCount == 3)

    scheduler.updateDemand(visible: [firstVisible, secondVisible], prefetch: prefetch)
    #expect(await requests.cancellationRequests.count == 1)
    #expect(scheduler.activeRequestCount == 3)
    #expect(scheduler.requestCount == 3)

    await requests.complete(startedPrefetch[2], with: makeImage())
    await requests.waitForStartCount(4)
    #expect(await requests.startedRequests.map(\.key) == [
      firstPrefetch, secondPrefetch, thirdPrefetch, firstVisible,
    ])
    #expect(scheduler.activeRequestCount == 3)
    #expect(await requests.maximumConcurrentCount == 3)

    await finishSchedulerRequests(scheduler, requests: requests)
  }

  @Test func deduplicatesKeysAcrossVisibleAndPrefetchDemand() async {
    let cache = ThumbnailCache(totalCostLimit: 1024)
    let requests = RequestHarness()
    let results = ResultRecorder()
    let scheduler = ThumbnailScheduler(
      concurrencyLimit: 1,
      cache: cache,
      request: { key, generation in try await requests.request(key, generation: generation) },
      resultSink: { results.record(key: $0, generation: $1, result: $2) }
    )
    let requested = key("same.png")

    scheduler.updateDemand(visible: [requested], prefetch: [requested, requested])
    await requests.waitForStartCount(1)
    let started = await requests.startedRequests
    #expect(started.map(\.key) == [requested])
    await finishSchedulerRequests(scheduler, requests: requests)
  }

  @Test func visibleDemandPrecedesPrefetchAndDirectionChangesReorderTheQueue() async {
    let cache = ThumbnailCache(totalCostLimit: 1024)
    let requests = RequestHarness()
    let results = ResultRecorder()
    let scheduler = ThumbnailScheduler(
      concurrencyLimit: 1,
      cache: cache,
      request: { key, generation in try await requests.request(key, generation: generation) },
      resultSink: { results.record(key: $0, generation: $1, result: $2) }
    )
    let visible = key("visible.png")
    let forward = key("forward.png")
    let backward = key("backward.png")
    let image = makeImage()

    scheduler.updateDemand(visible: [visible], prefetch: [forward, backward])
    await requests.waitForStartCount(1)
    scheduler.updateDemand(visible: [visible], prefetch: [backward, forward])
    let first = await requests.startedRequests[0]
    await requests.complete(first, with: image)
    await results.waitForCount(1)
    await requests.waitForStartCount(2)
    let second = await requests.startedRequests[1]
    #expect(second.key == backward)
    await requests.complete(second, with: image)
    await results.waitForCount(2)
    await requests.waitForStartCount(3)
    #expect(await requests.startedRequests[2].key == forward)
    await finishSchedulerRequests(scheduler, requests: requests)
  }

  @Test func cancellationReleasesTheSlotForCurrentDemand() async {
    let cache = ThumbnailCache(totalCostLimit: 1024)
    let requests = RequestHarness(respondsToCancellation: true)
    let results = ResultRecorder()
    let scheduler = ThumbnailScheduler(
      concurrencyLimit: 1,
      cache: cache,
      request: { key, generation in try await requests.request(key, generation: generation) },
      resultSink: { results.record(key: $0, generation: $1, result: $2) }
    )
    let abandoned = key("abandoned.png")
    let current = key("current.png")

    scheduler.updateDemand(visible: [abandoned], prefetch: [])
    await requests.waitForStartCount(1)
    scheduler.updateDemand(visible: [current], prefetch: [])
    await requests.waitForStartCount(2)
    await requests.waitForCompletionCount(1)

    #expect(scheduler.activeRequestCount == 1)
    #expect(await requests.cancelledCount == 1)
    #expect(await requests.startedRequests[1].key == current)
    await finishSchedulerRequests(scheduler, requests: requests)
  }

  @Test func cancellationKeepsTheSlotUntilUnderlyingWorkFinishes() async {
    let cache = ThumbnailCache(totalCostLimit: 1024)
    let requests = RequestHarness()
    let results = ResultRecorder()
    let scheduler = ThumbnailScheduler(
      concurrencyLimit: 1,
      cache: cache,
      request: { key, generation in try await requests.requestIgnoringCancellation(key, generation: generation) },
      resultSink: { results.record(key: $0, generation: $1, result: $2) }
    )
    let abandoned = key("abandoned.png")
    let current = key("current.png")

    scheduler.updateDemand(visible: [abandoned], prefetch: [])
    await requests.waitForStartCount(1)
    let abandonedRequest = await requests.startedRequests[0]

    scheduler.updateDemand(visible: [current], prefetch: [])
    #expect(scheduler.requestCount == 1)

    await requests.complete(abandonedRequest, with: makeImage())
    await requests.waitForCompletionCount(1)
    await requests.waitForStartCount(2)
    let currentRequest = await requests.startedRequests[1]
    #expect(currentRequest.key == current)

    let shutdown = Task { await scheduler.cancelAll() }
    await requests.complete(currentRequest, with: makeImage())
    await shutdown.value
    await finishSchedulerRequests(scheduler, requests: requests)
  }

  @Test func cancelAllCancelsAndAwaitsEveryOwnedRequest() async {
    let cache = ThumbnailCache(totalCostLimit: 1024)
    let requests = RequestHarness(respondsToCancellation: true)
    let results = ResultRecorder()
    let scheduler = ThumbnailScheduler(
      concurrencyLimit: 2,
      cache: cache,
      request: { key, generation in try await requests.request(key, generation: generation) },
      resultSink: { results.record(key: $0, generation: $1, result: $2) }
    )
    scheduler.updateDemand(visible: [key("first.png"), key("second.png")], prefetch: [])
    await requests.waitForStartCount(2)

    let shutdown = Task { await scheduler.cancelAll() }
    await requests.waitForCompletionCount(2)
    await shutdown.value

    #expect(scheduler.activeRequestCount == 0)
    #expect(await requests.cancelledCount == 2)
    #expect(results.results.isEmpty)
    await finishSchedulerRequests(scheduler, requests: requests)
  }

  @Test func rapidDirectionChangesKeepOnlyTheLatestBoundedDemand() async {
    let cache = ThumbnailCache(totalCostLimit: 1024)
    let requests = RequestHarness()
    let results = ResultRecorder()
    let scheduler = ThumbnailScheduler(
      concurrencyLimit: 1,
      cache: cache,
      request: { key, generation in try await requests.request(key, generation: generation) },
      resultSink: { results.record(key: $0, generation: $1, result: $2) }
    )
    let blocker = key("blocker.png")
    let first = key("first.png")
    let second = key("second.png")
    let image = makeImage()

    scheduler.updateDemand(visible: [blocker], prefetch: [])
    await requests.waitForStartCount(1)
    for direction in 0..<100 {
      let prefetch = direction.isMultiple(of: 2) ? [first, second] : [second, first]
      scheduler.updateDemand(visible: [blocker], prefetch: prefetch)
    }

    let active = await requests.startedRequests[0]
    await requests.complete(active, with: image)
    await results.waitForCount(1)
    await requests.waitForStartCount(2)
    #expect(await requests.startedRequests.map(\.key) == [blocker, second])
    let next = await requests.startedRequests[1]
    await requests.complete(next, with: image)
    await results.waitForCount(2)
    await requests.waitForStartCount(3)
    #expect(await requests.startedRequests[2].key == first)
    await finishSchedulerRequests(scheduler, requests: requests)
  }

  @Test func staleGenerationCannotPopulateCacheOrReachTheCell() async {
    let cache = ThumbnailCache(totalCostLimit: 1024)
    let requests = RequestHarness()
    let results = ResultRecorder()
    let scheduler = ThumbnailScheduler(
      concurrencyLimit: 1,
      cache: cache,
      request: { key, generation in try await requests.requestIgnoringCancellation(key, generation: generation) },
      resultSink: { results.record(key: $0, generation: $1, result: $2) }
    )
    let stale = key("stale.png")

    scheduler.updateDemand(visible: [stale], prefetch: [])
    await requests.waitForStartCount(1)
    let request = await requests.startedRequests[0]
    scheduler.updateDemand(visible: [], prefetch: [])
    await requests.complete(request, with: makeImage())
    await scheduler.cancelAll()

    #expect(results.results.isEmpty)
    #expect(cache.image(for: stale) == nil)
    await finishSchedulerRequests(scheduler, requests: requests)
  }

  @Test func removedItemCannotBeRequestedAgain() async {
    let cache = ThumbnailCache(totalCostLimit: 1024)
    let requests = RequestHarness(respondsToCancellation: true)
    let results = ResultRecorder()
    let scheduler = ThumbnailScheduler(
      concurrencyLimit: 1,
      cache: cache,
      request: { key, generation in try await requests.request(key, generation: generation) },
      resultSink: { results.record(key: $0, generation: $1, result: $2) }
    )
    let removed = key("removed.png")

    scheduler.updateDemand(visible: [removed], prefetch: [])
    await requests.waitForStartCount(1)
    scheduler.removeItem(at: removed.url)
    scheduler.updateDemand(visible: [removed], prefetch: [])
    await requests.waitForCompletionCount(1)

    #expect(await requests.startedRequests.count == 1)
    #expect(scheduler.activeRequestCount == 0)
    #expect(results.results.isEmpty)
    await finishSchedulerRequests(scheduler, requests: requests)
  }

  @Test func cacheHitAvoidsAQuickLookRequest() async {
    let cache = ThumbnailCache(totalCostLimit: 1024)
    let requests = RequestHarness()
    let results = ResultRecorder()
    let requested = key("cached.png")
    cache.insert(makeImage(), for: requested)
    let scheduler = ThumbnailScheduler(
      concurrencyLimit: 1,
      cache: cache,
      request: { key, generation in try await requests.request(key, generation: generation) },
      resultSink: { results.record(key: $0, generation: $1, result: $2) }
    )

    scheduler.updateDemand(visible: [requested], prefetch: [])

    #expect(await requests.startedRequests.isEmpty)
    #expect(results.results.count == 1)
    #expect(cache.totalCostBytes == 4)
    await finishSchedulerRequests(scheduler, requests: requests)
  }

  @Test func completedPrefetchCanServeVisibleCellRebindingFromCache() async {
    let cache = ThumbnailCache(totalCostLimit: 1024)
    let requests = RequestHarness()
    let results = ResultRecorder()
    let scheduler = ThumbnailScheduler(
      concurrencyLimit: 1,
      cache: cache,
      request: { key, generation in try await requests.request(key, generation: generation) },
      resultSink: { results.record(key: $0, generation: $1, result: $2) }
    )
    let prefetched = key("prefetched.png")

    scheduler.updateDemand(visible: [], prefetch: [prefetched])
    await requests.waitForStartCount(1)
    let started = await requests.startedRequests
    let request = started[0]
    let image = makeImage()
    await requests.complete(request, with: image)
    await results.waitForCount(1)

    scheduler.updateDemand(visible: [prefetched], prefetch: [])

    #expect(scheduler.requestCount == 1)
    #expect(results.results.count == 1)
    #expect(cache.image(for: prefetched)?.width == image.width)
    await finishSchedulerRequests(scheduler, requests: requests)
  }

  @Test func failureIsLatchedForTheSession() async {
    let cache = ThumbnailCache(totalCostLimit: 1024)
    let requests = RequestHarness(failure: RequestFailure.unavailable)
    let results = ResultRecorder()
    let failed = key("corrupt.png")
    let scheduler = ThumbnailScheduler(
      concurrencyLimit: 1,
      cache: cache,
      request: { key, generation in try await requests.request(key, generation: generation) },
      resultSink: { results.record(key: $0, generation: $1, result: $2) }
    )

    scheduler.updateDemand(visible: [failed], prefetch: [])
    await requests.waitForCompletionCount(1)
    await results.waitForCount(1)
    scheduler.updateDemand(visible: [], prefetch: [])
    scheduler.updateDemand(visible: [failed], prefetch: [])

    #expect(await requests.startedRequests.count == 1)
    #expect(results.results.count == 1)
    if case .failure = results.results[0].result {
      #expect(true)
    } else {
      Issue.record("Expected a latched thumbnail failure")
    }
    await finishSchedulerRequests(scheduler, requests: requests)
  }

  @Test func cacheEvictionDropsOldestEntriesAndRemovalDropsEveryVersion() {
    let cache = ThumbnailCache(totalCostLimit: 8)
    let first = key("first.png")
    let second = key("second.png")
    let third = key("third.png")
    let image = makeImage()

    cache.insert(image, for: first)
    cache.insert(image, for: second)
    cache.insert(image, for: third)

    #expect(cache.image(for: first) == nil)
    #expect(cache.image(for: second) != nil)
    #expect(cache.image(for: third) != nil)
    #expect(cache.totalCostBytes == 8)

    let secondRetina = ThumbnailKey(item: mediaItem("second.png"), pixelSize: 2, scale: 2)!
    cache.insert(image, for: secondRetina)
    cache.removeItems(at: second.url)

    #expect(cache.image(for: second) == nil)
    #expect(cache.image(for: secondRetina) == nil)
    #expect(cache.image(for: third) != nil)
  }

  @Test func rejectsInvalidThumbnailDimensions() {
    #expect(ThumbnailKey(item: mediaItem("bad.png"), pixelSize: 0, scale: 1) == nil)
    #expect(ThumbnailKey(item: mediaItem("bad.png"), pixelSize: 1, scale: 0) == nil)
  }

  private func key(_ name: String) -> ThumbnailKey {
    ThumbnailKey(item: mediaItem(name), pixelSize: 1, scale: 1)!
  }

  private func finishSchedulerRequests(_ scheduler: ThumbnailScheduler, requests: RequestHarness) async {
    scheduler.updateDemand(visible: [], prefetch: [])
    let requestCount = scheduler.requestCount
    await requests.waitForStartCount(requestCount)
    await requests.completeOutstandingRequests(with: makeImage())
    await requests.waitForCompletionCount(requestCount)
    await requests.awaitCancellationTasks()
    await scheduler.cancelAll()
    #expect(scheduler.activeRequestCount == 0)
  }

  private func mediaItem(_ name: String) -> MediaItem {
    MediaItem(
      url: URL(filePath: "/thumbnails/\(name)"),
      kind: .image,
      modificationDate: nil,
      fileSize: nil
    )
  }

  private func makeImage() -> CGImage {
    let context = CGContext(
      data: nil,
      width: 1,
      height: 1,
      bitsPerComponent: 8,
      bytesPerRow: 4,
      space: CGColorSpaceCreateDeviceRGB(),
      bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    )!
    return context.makeImage()!
  }
}

private struct StartedRequest: Sendable, Hashable {
  let key: ThumbnailKey
  let generation: UInt64
  let attempt: Int
}

private actor RequestHarness {
  private var continuations: [StartedRequest: CheckedContinuation<CGImage, any Error>] = [:]
  private var cancelledBeforeContinuation: Set<StartedRequest> = []
  private var resolvedRequests: Set<StartedRequest> = []
  private var startWaiters: [(Int, CheckedContinuation<Void, Never>)] = []
  private var completionWaiters: [(Int, CheckedContinuation<Void, Never>)] = []
  private var starts: [StartedRequest] = []
  private var currentConcurrentCount = 0
  private var maximumCount = 0
  private var completedCount = 0
  private var cancellationCount = 0
  private let cancellationRecorder = CancellationRecorder()
  private let cancellationTasks = CancellationTaskRegistry()

  private let respondsToCancellation: Bool
  private let failure: RequestFailure?

  init(respondsToCancellation: Bool = false, failure: RequestFailure? = nil) {
    self.respondsToCancellation = respondsToCancellation
    self.failure = failure
  }

  var startedRequests: [StartedRequest] { starts }
  var maximumConcurrentCount: Int { maximumCount }
  var cancelledCount: Int { cancellationCount }
  var cancellationRequests: [StartedRequest] { cancellationRecorder.requests }

  func request(_ key: ThumbnailKey, generation: UInt64) async throws -> CGImage {
    try await perform(key, generation: generation, ignoresCancellation: false)
  }

  func requestIgnoringCancellation(_ key: ThumbnailKey, generation: UInt64) async throws -> CGImage {
    try await perform(key, generation: generation, ignoresCancellation: true)
  }

  func waitForStartCount(_ count: Int) async {
    guard starts.count < count else { return }
    await withCheckedContinuation { startWaiters.append((count, $0)) }
  }

  func waitForCompletionCount(_ count: Int) async {
    guard completedCount < count else { return }
    await withCheckedContinuation { completionWaiters.append((count, $0)) }
  }

  func complete(_ request: StartedRequest, with image: CGImage) {
    guard let continuation = continuations.removeValue(forKey: request) else { return }
    resolvedRequests.insert(request)
    continuation.resume(returning: image)
  }

  func completeOutstandingRequests(with image: CGImage) {
    let pending = continuations
    continuations.removeAll(keepingCapacity: false)
    for (request, continuation) in pending {
      resolvedRequests.insert(request)
      continuation.resume(returning: image)
    }
  }

  func awaitCancellationTasks() async {
    for task in cancellationTasks.take() {
      await task.value
    }
  }

  private func perform(
    _ key: ThumbnailKey,
    generation: UInt64,
    ignoresCancellation: Bool
  ) async throws -> CGImage {
    let request = StartedRequest(key: key, generation: generation, attempt: starts.count)
    let cancellationRecorder = self.cancellationRecorder
    let cancellationTasks = self.cancellationTasks
    starts.append(request)
    currentConcurrentCount += 1
    maximumCount = max(maximumCount, currentConcurrentCount)
    resumeStartWaiters()
    defer {
      resolvedRequests.insert(request)
      currentConcurrentCount -= 1
      completedCount += 1
      resumeCompletionWaiters()
    }

    if let failure {
      throw failure
    }

    return try await withTaskCancellationHandler {
      try await withCheckedThrowingContinuation { continuation in
        if cancelledBeforeContinuation.remove(request) != nil {
          resolvedRequests.insert(request)
          continuation.resume(throwing: CancellationError())
        } else {
          continuations[request] = continuation
        }
      }
    } onCancel: {
      cancellationRecorder.record(request)
      guard respondsToCancellation, !ignoresCancellation else { return }
      cancellationTasks.append(Task { await self.cancel(request) })
    }
  }

  private func cancel(_ request: StartedRequest) {
    guard !resolvedRequests.contains(request), !cancelledBeforeContinuation.contains(request) else { return }
    cancellationCount += 1
    if let continuation = continuations.removeValue(forKey: request) {
      resolvedRequests.insert(request)
      continuation.resume(throwing: CancellationError())
    } else {
      cancelledBeforeContinuation.insert(request)
    }
  }

  private func resumeStartWaiters() {
    let ready = startWaiters.filter { starts.count >= $0.0 }
    startWaiters.removeAll { starts.count >= $0.0 }
    ready.forEach { $0.1.resume() }
  }

  private func resumeCompletionWaiters() {
    let ready = completionWaiters.filter { completedCount >= $0.0 }
    completionWaiters.removeAll { completedCount >= $0.0 }
    ready.forEach { $0.1.resume() }
  }
}

private final class CancellationRecorder: Sendable {
  private let state = Mutex<[StartedRequest]>([])

  var requests: [StartedRequest] {
    state.withLock { $0 }
  }

  func record(_ request: StartedRequest) {
    state.withLock { $0.append(request) }
  }
}

private final class CancellationTaskRegistry: Sendable {
  private let state = Mutex<[Task<Void, Never>]>([])

  func append(_ task: Task<Void, Never>) {
    state.withLock { $0.append(task) }
  }

  func take() -> [Task<Void, Never>] {
    state.withLock { state in
      defer { state.removeAll(keepingCapacity: false) }
      return state
    }
  }
}

@MainActor
private final class ResultRecorder {
  struct Entry {
    let key: ThumbnailKey
    let generation: UInt64
    let result: ThumbnailResult
  }

  private(set) var results: [Entry] = []
  private var waiters: [(Int, CheckedContinuation<Void, Never>)] = []

  func record(key: ThumbnailKey, generation: UInt64, result: ThumbnailResult) {
    results.append(Entry(key: key, generation: generation, result: result))
    let ready = waiters.filter { results.count >= $0.0 }
    waiters.removeAll { results.count >= $0.0 }
    ready.forEach { $0.1.resume() }
  }

  func waitForCount(_ count: Int) async {
    guard results.count < count else { return }
    await withCheckedContinuation { waiters.append((count, $0)) }
  }
}

private enum RequestFailure: Error {
  case unavailable
}
