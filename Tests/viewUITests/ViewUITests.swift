import AppKit
import Foundation
import Testing
import viewCore
@testable import viewUI

@MainActor
private enum UITestSupport {
  static func ensureApplication() {
    let app = NSApplication.shared
    if app.activationPolicy() != .accessory {
      app.setActivationPolicy(.accessory)
    }
  }

  static func makeDirectory() throws -> TestDirectory {
    let base = packageRoot
      .appending(path: ".work/tmp/ui-tests")
      .appending(path: UUID().uuidString)
    try FileManager.default.createDirectory(at: base, withIntermediateDirectories: true)
    return TestDirectory(url: base)
  }

  static func writePNG(named name: String, in directory: URL) throws -> URL {
    let url = directory.appending(path: name)
    try minimalPNG.write(to: url)
    return url
  }

  static func pump(until condition: @escaping @MainActor () -> Bool, timeout: TimeInterval = 8) async -> Bool {
    let deadline = Date().addingTimeInterval(timeout)
    while Date() < deadline {
      if condition() { return true }
      try? await Task.sleep(nanoseconds: 20_000_000)
      await Task.yield()
    }
    return condition()
  }

  static func keyEvent(keyCode: UInt16, characters: String = "", charactersIgnoringModifiers: String? = nil, modifiers: NSEvent.ModifierFlags = []) -> NSEvent {
    NSEvent.keyEvent(
      with: .keyDown,
      location: .zero,
      modifierFlags: modifiers,
      timestamp: ProcessInfo.processInfo.systemUptime,
      windowNumber: 0,
      context: nil,
      characters: characters,
      charactersIgnoringModifiers: charactersIgnoringModifiers ?? characters,
      isARepeat: false,
      keyCode: keyCode
    )!
  }

  private static var packageRoot: URL {
    URL(filePath: #filePath)
      .deletingLastPathComponent()
      .deletingLastPathComponent()
      .deletingLastPathComponent()
  }

  private static let minimalPNG: Data = {
    var d = Data()
    d.append(contentsOf: [137, 80, 78, 71, 13, 10, 26, 10])
    func chunk(_ type: String, _ payload: [UInt8]) {
      var c = Data()
      var len = UInt32(payload.count).bigEndian
      withUnsafeBytes(of: &len) { c.append(contentsOf: $0) }
      c.append(contentsOf: type.utf8)
      c.append(contentsOf: payload)
      var crc: UInt32 = 0
      let crcData = Data(type.utf8) + Data(payload)
      var table = [UInt32](repeating: 0, count: 256)
      for n in 0..<256 {
        var v = UInt32(n)
        for _ in 0..<8 { v = (v & 1) != 0 ? 0xEDB88320 ^ (v >> 1) : (v >> 1) }
        table[n] = v
      }
      crc = 0xFFFFFFFF
      for b in crcData { crc = table[Int((crc ^ UInt32(b)) & 0xFF)] ^ (crc >> 8) }
      crc ^= 0xFFFFFFFF
      var crcBE = crc.bigEndian
      withUnsafeBytes(of: &crcBE) { c.append(contentsOf: $0) }
      d.append(c)
    }
    var ihdr: [UInt8] = []
    var w = UInt32(1).bigEndian; var h = UInt32(1).bigEndian
    withUnsafeBytes(of: &w) { ihdr.append(contentsOf: $0) }
    withUnsafeBytes(of: &h) { ihdr.append(contentsOf: $0) }
    ihdr.append(contentsOf: [8, 2, 0, 0, 0])
    chunk("IHDR", ihdr)
    chunk("IDAT", [0x78, 0x01, 0x62, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D, 0xB4])
    chunk("IEND", [])
    return d
  }()
}

@MainActor
private struct TestDirectory {
  let url: URL
  func remove() { try? FileManager.default.removeItem(at: url) }
}

@MainActor
private final class TestHarness {
  let directory: TestDirectory
  let window: NSWindow
  let controller: PreviewPanelController
  var grid: GridViewController { controller.grid }

  static func make(files: [(String, Bool)], subdirectories: [String] = [], viewMode: ViewMode = .carousel) throws -> TestHarness {
    UITestSupport.ensureApplication()
    let directory = try UITestSupport.makeDirectory()
    for sub in subdirectories {
      try FileManager.default.createDirectory(at: directory.url.appending(path: sub), withIntermediateDirectories: true)
    }
    for (name, isImage) in files {
      if isImage {
        _ = try UITestSupport.writePNG(named: name, in: directory.url)
      } else {
        try Data("x".utf8).write(to: directory.url.appending(path: name))
      }
    }
    let root = try RootDirectory.resolve(argument: directory.url.path, currentDirectory: "/")
    let controller = PreviewPanelController(root: root, mode: .browser, metrics: ViewMetrics(enabled: false))
    let window = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: 1100, height: 760),
      styleMask: [.titled, .closable, .miniaturizable, .resizable],
      backing: .buffered,
      defer: false
    )
    window.isReleasedWhenClosed = false
    window.contentViewController = controller
    window.setContentSize(NSSize(width: 1100, height: 760))
    window.layoutIfNeeded()
    controller.view.layoutSubtreeIfNeeded()
    let harness = TestHarness(directory: directory, window: window, controller: controller)
    if viewMode != .carousel {
      harness.setViewMode(viewMode)
    }
    return harness
  }

  init(directory: TestDirectory, window: NSWindow, controller: PreviewPanelController) {
    self.directory = directory
    self.window = window
    self.controller = controller
  }

  func show() {
    window.makeKeyAndOrderFront(nil)
    window.layoutIfNeeded()
    controller.view.layoutSubtreeIfNeeded()
  }

  func waitForScan(itemCount: Int, timeout: TimeInterval = 8) async -> Bool {
    show()
    return await UITestSupport.pump(until: { [grid] in grid.catalog.count == itemCount }, timeout: timeout)
  }

  func setViewMode(_ mode: ViewMode) {
    grid.setViewMode(mode)
    window.layoutIfNeeded()
    controller.view.layoutSubtreeIfNeeded()
  }

  func tearDown() async {
    await controller.cancelWork()
    window.orderOut(nil)
    window.contentViewController = nil
    directory.remove()
  }
}

@Suite("UI integration", .serialized)
@MainActor
struct ViewUITests {
  @Test
  func enteringDirectorySelectsFirstItemAndLoadsCarouselPreview() async throws {
    let h = try TestHarness.make(files: [], subdirectories: ["sub"])
    _ = try UITestSupport.writePNG(named: "a.png", in: h.directory.url.appending(path: "sub"))

    #expect(await h.waitForScan(itemCount: 1))
    let subIndex = try #require(h.grid.catalog.items.firstIndex(where: { $0.kind == .directory }))
    h.grid.activateItem(at: subIndex)

    let scanned = await UITestSupport.pump(until: { h.grid.catalog.count == 1 && h.grid.catalog.selectedURL != nil }, timeout: 8)
    #expect(scanned)
    #expect(h.grid.catalog.selectedURL?.lastPathComponent == "a.png")

    let loaded = await UITestSupport.pump(until: { h.grid.carouselPreviewImageForTesting != nil }, timeout: 8)
    #expect(loaded)
    await h.tearDown()
  }

  @Test
  func backRestoresParentCursorPosition() async throws {
    let h = try TestHarness.make(files: [], subdirectories: ["sub"])
    _ = try UITestSupport.writePNG(named: "a.png", in: h.directory.url)
    _ = try UITestSupport.writePNG(named: "b.png", in: h.directory.url)

    #expect(await h.waitForScan(itemCount: 3))
    let bURL = try #require(h.grid.catalog.items.first(where: { $0.url.lastPathComponent == "b.png" })?.url)
    h.grid.selectFromPreview(bURL)
    _ = await UITestSupport.pump(until: { h.grid.catalog.selectedURL == bURL }, timeout: 2)

    let subIndex = try #require(h.grid.catalog.items.firstIndex(where: { $0.kind == .directory }))
    h.grid.activateItem(at: subIndex)
    #expect(await UITestSupport.pump(until: { h.grid.root.url.lastPathComponent == "sub" }, timeout: 8))
    #expect(await UITestSupport.pump(until: { h.grid.canNavigateBackForTesting }, timeout: 8))

    h.grid.navigateBack()
    #expect(await UITestSupport.pump(until: { h.grid.root.url == h.directory.url && h.grid.catalog.selectedURL == bURL }, timeout: 8))
    await h.tearDown()
  }

  @Test
  func shiftLeftNavigatesBack() async throws {
    let h = try TestHarness.make(files: [], subdirectories: ["sub"])

    #expect(await h.waitForScan(itemCount: 1))
    let subIndex = try #require(h.grid.catalog.items.firstIndex(where: { $0.kind == .directory }))
    h.grid.activateItem(at: subIndex)
    #expect(await UITestSupport.pump(until: { h.grid.root.url.lastPathComponent == "sub" }, timeout: 8))

    let event = UITestSupport.keyEvent(keyCode: 123, characters: "", modifiers: [.shift])
    let handled = h.grid.handleCollectionKeyDown(event)
    #expect(handled)
    #expect(await UITestSupport.pump(until: { h.grid.root.url == h.directory.url }, timeout: 8))
    await h.tearDown()
  }

  @Test
  func arrowWraparound() async throws {
    let h = try TestHarness.make(files: [("a.png", true), ("b.png", true)], viewMode: .grid)

    #expect(await h.waitForScan(itemCount: 2))
    let first = try #require(h.grid.catalog.items.first?.url)
    let last = try #require(h.grid.catalog.items.last?.url)
    h.grid.selectFromPreview(first)
    _ = await UITestSupport.pump(until: { h.grid.catalog.selectedURL == first }, timeout: 2)

    let up = UITestSupport.keyEvent(keyCode: 126, characters: "", modifiers: [])
    #expect(h.grid.handleCollectionKeyDown(up))
    #expect(await UITestSupport.pump(until: { h.grid.catalog.selectedURL == last }, timeout: 2))

    let down = UITestSupport.keyEvent(keyCode: 125, characters: "", modifiers: [])
    #expect(h.grid.handleCollectionKeyDown(down))
    #expect(await UITestSupport.pump(until: { h.grid.catalog.selectedURL == first }, timeout: 2))
    await h.tearDown()
  }

  @Test
  func modeSwitchKeepsWindowUnconstrained() async throws {
    let h = try TestHarness.make(files: [("a.png", true)], viewMode: .carousel)
    h.show()
    h.window.setFrame(NSRect(x: 100, y: 100, width: 900, height: 700), display: true)
    h.window.layoutIfNeeded()
    h.setViewMode(.grid)
    try? await Task.sleep(nanoseconds: 50_000_000)
    h.setViewMode(.carousel)
    try? await Task.sleep(nanoseconds: 50_000_000)
    #expect(h.window.styleMask.contains(.resizable))
    #expect(h.window.contentAspectRatio == NSSize(width: 0, height: 0))
    #expect(h.window.contentMinSize == NSSize(width: 0, height: 0))
    await h.tearDown()
  }
}
