import AppKit
import Foundation
import QuickLookThumbnailing
import Synchronization
import os
import viewCore

@MainActor
final class ThumbnailCoordinator {
  private weak var grid: GridViewController?
  private let cache = ThumbnailCache()
  private let metrics: ViewMetrics
  private var scheduler: ThumbnailScheduler?
  private var visibleItems: [MediaItem] = []
  private var prefetchItems: [MediaItem] = []
  private var initialViewport: Set<ThumbnailKey>?
  private var settledInitialViewport: Set<ThumbnailKey> = []
  private var hasRecordedInitialViewport = false

  init(grid: GridViewController, metrics: ViewMetrics) {
    self.grid = grid
    self.metrics = metrics
  }

  func configure(volume: ThumbnailVolume) {
    guard scheduler == nil else { preconditionFailure("thumbnail scheduler already configured") }
    scheduler = ThumbnailScheduler(
      volume: volume,
      cache: cache,
      request: { [metrics] key, generation in
        await metrics.thumbnailRequestStarted(key, generation: generation)
        do {
          let image = try await generateQuickLookThumbnail(for: key)
          await metrics.thumbnailRequestFinished(key, generation: generation)
          return image
        } catch {
          await metrics.thumbnailRequestFinished(key, generation: generation)
          throw error
        }
      },
      resultSink: { [weak self] key, generation, result in
        self?.receive(key: key, generation: generation, result: result)
      }
    )
    updateDemand(visible: visibleItems, prefetch: prefetchItems)
  }

  func updateDemand(visible: [MediaItem], prefetch: [MediaItem]) {
    visibleItems = visible
    prefetchItems = prefetch
    guard let scheduler else { return }
    let scale = backingScale
    let visibleKeys = makeKeys(for: visible, scale: scale)
    let prefetchKeys = makeKeys(for: prefetch, scale: scale)
    if initialViewport == nil, !visibleKeys.isEmpty {
      initialViewport = Set(visibleKeys)
      metrics.initialViewportStarted()
    }
    scheduler.updateDemand(visible: visibleKeys, prefetch: prefetchKeys)
  }

  func cachedImage(for item: MediaItem) -> CGImage? {
    let image = cache.image(for: makeKey(for: item, scale: backingScale))
    if image != nil {
      metrics.firstThumbnailDisplayed()
    }
    return image
  }

  private var backingScale: CGFloat { grid?.view.window?.backingScaleFactor ?? 1 }

  func removeItem(at url: URL) {
    scheduler?.removeItem(at: url)
  }

  func cancelAll() async {
    await scheduler?.cancelAll()
  }

  @concurrent
  nonisolated static func volume(for root: RootDirectory) async throws -> ThumbnailVolume {
    let values = try root.url.resourceValues(forKeys: [.volumeIsLocalKey])
    return values.volumeIsLocal == true ? .local : .other
  }

  private func makeKeys(for items: [MediaItem], scale: CGFloat) -> [ThumbnailKey] {
    items.map { makeKey(for: $0, scale: scale) }
  }

  private func makeKey(for item: MediaItem, scale: CGFloat) -> ThumbnailKey {
    guard let key = ThumbnailKey(
      item: item,
      pixelSize: Int((160 * scale).rounded()),
      scale: scale
    ) else {
      preconditionFailure("invalid thumbnail dimensions")
    }
    return key
  }

  private func receive(key: ThumbnailKey, generation _: UInt64, result: ThumbnailResult) {
    switch result {
    case .image(let image):
      if key.scale == backingScale, grid?.displayThumbnail(image, for: key.url) == true {
        metrics.firstThumbnailDisplayed()
      }
    case .failure, .cancelled:
      break
    }

    guard let initialViewport, initialViewport.contains(key) else { return }
    settledInitialViewport.insert(key)
    guard !hasRecordedInitialViewport, settledInitialViewport.isSuperset(of: initialViewport) else { return }
    hasRecordedInitialViewport = true
    metrics.initialViewportSettled(
      cacheCostBytes: cache.totalCostBytes,
      requestCount: scheduler?.requestCount ?? 0,
      maximumConcurrentRequestCount: scheduler?.maximumConcurrentRequestCount ?? 0
    )
  }
}

@MainActor
final class ViewMetrics {
  private let enabled: Bool
  private let clock = ContinuousClock()
  private let processStart: ContinuousClock.Instant
  private let signposter: OSSignposter
  private var launchInterval: OSSignpostIntervalState?
  private var scanInterval: OSSignpostIntervalState?
  private var viewportInterval: OSSignpostIntervalState?
  private var thumbnailIntervals: [ThumbnailRequestIdentity: OSSignpostIntervalState] = [:]
  private var didRecordFirstBatch = false
  private var didRecordFirstThumbnail = false
  private var appliedBatchCount = 0

  init(enabled: Bool) {
    self.enabled = enabled
    processStart = clock.now
    signposter = enabled ? OSSignposter(subsystem: "com.view", category: "Performance") : .disabled
    if enabled {
      launchInterval = signposter.beginInterval("launch")
    }
  }

  func windowShown() {
    if let launchInterval {
      signposter.endInterval("launch", launchInterval)
      self.launchInterval = nil
    }
    record("window-shown-ms", elapsedMilliseconds)
  }

  func scanStarted() {
    guard enabled else { return }
    scanInterval = signposter.beginInterval("scan")
  }

  func catalogBatchApplied() {
    appliedBatchCount += 1
    guard enabled, !didRecordFirstBatch else { return }
    didRecordFirstBatch = true
    record("first-batch-ms", elapsedMilliseconds)
  }

  func scanCompleted(_ summary: DirectoryScanSummary) {
    if let scanInterval {
      signposter.endInterval("scan", scanInterval)
      self.scanInterval = nil
    }
    record("scan-complete-ms", elapsedMilliseconds)
    record("scan-items", String(summary.itemCount))
    record("scan-batches", String(appliedBatchCount))
  }

  func scanFailed() {
    if let scanInterval {
      signposter.endInterval("scan", scanInterval)
      self.scanInterval = nil
    }
  }

  func initialViewportStarted() {
    guard enabled else { return }
    viewportInterval = signposter.beginInterval("initial-viewport")
  }

  func initialViewportSettled(
    cacheCostBytes: Int,
    requestCount: Int,
    maximumConcurrentRequestCount: Int
  ) {
    if let viewportInterval {
      signposter.endInterval("initial-viewport", viewportInterval)
      self.viewportInterval = nil
    }
    record("visible-populated-ms", elapsedMilliseconds)
    record("thumbnail-requests", String(requestCount))
    record("max-concurrent-thumbnail-requests", String(maximumConcurrentRequestCount))
    record("thumbnail-cache-bytes", String(cacheCostBytes))
  }

  func thumbnailRequestStarted(_ key: ThumbnailKey, generation: UInt64) {
    guard enabled else { return }
    thumbnailIntervals[ThumbnailRequestIdentity(key: key, generation: generation)] =
      signposter.beginInterval("thumbnail-request")
  }

  func thumbnailRequestFinished(_ key: ThumbnailKey, generation: UInt64) {
    guard enabled else { return }
    let identity = ThumbnailRequestIdentity(key: key, generation: generation)
    guard let interval = thumbnailIntervals.removeValue(forKey: identity) else { return }
    signposter.endInterval("thumbnail-request", interval)
  }

  func firstThumbnailDisplayed() {
    guard enabled, !didRecordFirstThumbnail else { return }
    didRecordFirstThumbnail = true
    record("first-thumbnail-ms", elapsedMilliseconds)
  }

  private var elapsedMilliseconds: Double {
    let elapsed = processStart.duration(to: clock.now).components
    return Double(elapsed.seconds) * 1000 + Double(elapsed.attoseconds) / 1_000_000_000_000_000
  }

  private func record(_ name: String, _ value: String) {
    guard enabled else { return }
    FileHandle.standardError.write(Data("VIEW_METRIC \(name)=\(value)\n".utf8))
  }

  private func record(_ name: String, _ value: Double) {
    record(name, String(format: "%.1f", locale: Locale(identifier: "en_US_POSIX"), value))
  }
}

private struct ThumbnailRequestIdentity: Hashable {
  let key: ThumbnailKey
  let generation: UInt64
}

private enum ThumbnailRequestFailure: Error, LocalizedError, Sendable {
  case iconOnly(URL)
  case noRepresentation(URL)

  var errorDescription: String? {
    switch self {
    case .iconOnly(let url): "Quick Look returned only a file icon for \(url.path)"
    case .noRepresentation(let url): "Quick Look returned no thumbnail for \(url.path)"
    }
  }
}

@concurrent
private func generateQuickLookThumbnail(for key: ThumbnailKey) async throws -> CGImage {
  let request = QuickLookRequestBox(key: key)
  let cancellation = CancellationTaskOwner()

  let image: CGImage
  do {
    image = try await withTaskCancellationHandler {
      try await request.generate()
    } onCancel: {
      cancellation.cancel { await request.cancel() }
    }
  } catch {
    await cancellation.finish()
    throw error
  }
  await cancellation.finish()
  return image
}

private final class CancellationTaskOwner: Sendable {
  private struct State: Sendable {
    var isFinished = false
    var task: Task<Void, Never>?
  }

  nonisolated private let state = Mutex(State())

  nonisolated func cancel(_ action: @escaping @Sendable () async -> Void) {
    state.withLock { state in
      guard !state.isFinished, state.task == nil else { return }
      state.task = Task { await action() }
    }
  }

  nonisolated func finish() async {
    let task = state.withLock { state -> Task<Void, Never>? in
      state.isFinished = true
      return state.task
    }
    await task?.value
  }
}

private actor QuickLookRequestBox {
  private let generator = QLThumbnailGenerator.shared
  private let request: QLThumbnailGenerator.Request
  private let url: URL
  private var isCancelled = false

  init(key: ThumbnailKey) {
    let pointSize = CGFloat(key.pixelSize) / key.scale
    url = key.url
    request = QLThumbnailGenerator.Request(
      fileAt: key.url,
      size: NSSize(width: pointSize, height: pointSize),
      scale: key.scale,
      representationTypes: .thumbnail
    )
    request.iconMode = false
  }

  func generate() async throws -> CGImage {
    guard !isCancelled else { throw CancellationError() }
    let url = self.url
    return try await withCheckedThrowingContinuation { continuation in
      generator.generateBestRepresentation(for: request) { representation, error in
        guard let representation else {
          if let error {
            continuation.resume(throwing: error)
          } else {
            continuation.resume(throwing: ThumbnailRequestFailure.noRepresentation(url))
          }
          return
        }
        guard representation.type != .icon else {
          continuation.resume(throwing: ThumbnailRequestFailure.iconOnly(url))
          return
        }
        continuation.resume(returning: representation.cgImage)
      }
    }
  }

  func cancel() {
    isCancelled = true
    generator.cancel(request)
  }
}
