import AppKit
import viewCore

public enum ViewApp {
  private static var delegate: ViewAppDelegate?

  public static func main() {
    let rawArguments = Array(CommandLine.arguments.dropFirst())
    if rawArguments.contains("--help") || rawArguments.contains("-h") {
      printUsage()
      return
    }

    var dumpVisualDir: String?
    var arguments: [String] = []
    var idx = 0
    while idx < rawArguments.count {
      if rawArguments[idx] == "--dump-visual", idx + 1 < rawArguments.count {
        dumpVisualDir = rawArguments[idx + 1]
        idx += 2
      } else {
        arguments.append(rawArguments[idx])
        idx += 1
      }
    }

    let options: LaunchOptions
    do {
      options = try LaunchOptions.resolve(
        arguments: arguments,
        currentDirectory: FileManager.default.currentDirectoryPath
      )
    } catch {
      FileHandle.standardError.write(Data("view: \(error.localizedDescription)\n".utf8))
      exit(1)
    }

    let metrics = ViewMetrics(enabled: ProcessInfo.processInfo.environment["VIEW_METRICS"] == "1")
    let app = NSApplication.shared
    app.setActivationPolicy(.regular)
    let delegate = ViewAppDelegate(root: options.root, mode: options.mode, metrics: metrics, dumpVisualDir: dumpVisualDir)
    Self.delegate = delegate
    app.delegate = delegate
    app.run()
  }

  private static func printUsage() {
    print("usage: view [--recursive] [directory]")
    print("  Browse one directory at a time, largest files first.")
    print("  --recursive  flatten the whole tree into one grid")
  }
}

@MainActor
final class ViewAppDelegate: NSObject, NSApplicationDelegate {
  private let root: RootDirectory
  private let mode: BrowserMode
  private let metrics: ViewMetrics
  private let dumpVisualDir: String?
  private var window: NSWindow?
  private var shutdownTask: Task<Void, Never>?
  private var signalSources: [DispatchSourceSignal] = []

  init(root: RootDirectory, mode: BrowserMode, metrics: ViewMetrics, dumpVisualDir: String? = nil) {
    self.root = root
    self.mode = mode
    self.metrics = metrics
    self.dumpVisualDir = dumpVisualDir
  }

  func applicationWillFinishLaunching(_ notification: Notification) {
    UserDefaults.standard.set(true, forKey: "ApplePersistenceIgnoreState")
    ProcessInfo.processInfo.disableAutomaticTermination("view")
    ProcessInfo.processInfo.disableSuddenTermination()
  }

  func applicationDidFinishLaunching(_ notification: Notification) {
    installSignalHandlers()
    let window = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: 1100, height: 760),
      styleMask: [.titled, .closable, .miniaturizable, .resizable],
      backing: .buffered,
      defer: false
    )
    window.isRestorable = false
    window.title = root.url.path
    let previewController = PreviewPanelController(root: root, mode: mode, metrics: metrics)
    window.contentViewController = previewController
    installMainMenu(previewController: previewController)
    window.setContentSize(NSSize(width: 1100, height: 760))
    if dumpVisualDir == nil, let savedFrame = savedFrame {
      window.setFrame(savedFrame, display: true)
    } else {
      let contentRect = NSRect(x: 0, y: 0, width: 1100, height: 760)
      window.setFrame(window.frameRect(forContentRect: contentRect), display: true)
      window.center()
    }
    window.makeKeyAndOrderFront(nil)
    observeWindowFrame(window)
    metrics.windowShown()
    self.window = window
    NSApp.activate()

    if let dir = dumpVisualDir {
      runVisualDump(window: window, controller: previewController, outputDir: URL(filePath: dir))
    }
  }

  private func captureScreenshot(window: NSWindow, to url: URL) {
    let contentRect = NSRect(x: 0, y: 0, width: 1100, height: 760)
    window.setFrame(window.frameRect(forContentRect: contentRect), display: true)
    window.layoutIfNeeded()
    guard let content = window.contentView else { return }
    content.setFrameSize(NSSize(width: 1100, height: 760))
    content.layoutSubtreeIfNeeded()
    let bounds = NSRect(x: 0, y: 0, width: 1100, height: 760)
    guard let rep = content.bitmapImageRepForCachingDisplay(in: bounds) else { return }
    content.cacheDisplay(in: bounds, to: rep)
    guard let data = rep.representation(using: .png, properties: [:]) else { return }
    try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
    try? data.write(to: url)
  }

  private func sendKey(window: NSWindow, keyCode: UInt16, modifiers: NSEvent.ModifierFlags = []) {
    let event = NSEvent.keyEvent(
      with: .keyDown,
      location: .zero,
      modifierFlags: modifiers,
      timestamp: ProcessInfo.processInfo.systemUptime,
      windowNumber: window.windowNumber,
      context: nil,
      characters: "",
      charactersIgnoringModifiers: "",
      isARepeat: false,
      keyCode: keyCode
    )!
    window.sendEvent(event)
  }

  private func runVisualDump(window: NSWindow, controller: PreviewPanelController, outputDir: URL) {
    Task { @MainActor in
      // Ensure initial view is carousel
      controller.grid.setViewMode(.carousel)
      window.layoutIfNeeded()
      controller.view.layoutSubtreeIfNeeded()

      // Wait until scan completes
      for _ in 0..<50 {
        if controller.grid.catalog.count > 0 { break }
        try? await Task.sleep(nanoseconds: 50_000_000)
      }
      try? await Task.sleep(nanoseconds: 200_000_000)

      // Step 0: Carousel initial view
      captureScreenshot(window: window, to: outputDir.appending(path: "step_0_carousel_initial.png"))

      // Step 1: Right arrow
      sendKey(window: window, keyCode: 124)
      try? await Task.sleep(nanoseconds: 200_000_000)
      captureScreenshot(window: window, to: outputDir.appending(path: "step_1_carousel_right.png"))

      // Step 2: Right arrow again
      sendKey(window: window, keyCode: 124)
      try? await Task.sleep(nanoseconds: 200_000_000)
      captureScreenshot(window: window, to: outputDir.appending(path: "step_2_carousel_red.png"))

      // Step 3: Switch to Grid mode
      controller.grid.setViewMode(.grid)
      window.layoutIfNeeded()
      controller.view.layoutSubtreeIfNeeded()
      try? await Task.sleep(nanoseconds: 200_000_000)
      captureScreenshot(window: window, to: outputDir.appending(path: "step_3_grid_mode.png"))

      // Step 4: Down arrow in Grid
      sendKey(window: window, keyCode: 125)
      try? await Task.sleep(nanoseconds: 200_000_000)
      captureScreenshot(window: window, to: outputDir.appending(path: "step_4_grid_down.png"))

      // Step 5: Left arrow in Grid
      sendKey(window: window, keyCode: 123)
      try? await Task.sleep(nanoseconds: 200_000_000)
      captureScreenshot(window: window, to: outputDir.appending(path: "step_5_grid_left.png"))

      // Step 6: Switch back to Carousel
      controller.grid.setViewMode(.carousel)
      window.layoutIfNeeded()
      controller.view.layoutSubtreeIfNeeded()
      try? await Task.sleep(nanoseconds: 200_000_000)
      captureScreenshot(window: window, to: outputDir.appending(path: "step_6_back_to_carousel.png"))

      // Step 7: Select directory (index 0) and enter with Return (keyCode 36)
      sendKey(window: window, keyCode: 123)
      sendKey(window: window, keyCode: 123)
      sendKey(window: window, keyCode: 123)
      try? await Task.sleep(nanoseconds: 100_000_000)
      sendKey(window: window, keyCode: 36)
      for _ in 0..<50 {
        if controller.grid.catalog.count > 0 { break }
        try? await Task.sleep(nanoseconds: 50_000_000)
      }
      try? await Task.sleep(nanoseconds: 200_000_000)
      captureScreenshot(window: window, to: outputDir.appending(path: "step_7_entered_subdir.png"))

      // Step 8: History back (Shift+Left, keyCode 123 with Shift)
      sendKey(window: window, keyCode: 123, modifiers: [.shift])
      for _ in 0..<50 {
        if controller.grid.catalog.count > 0 { break }
        try? await Task.sleep(nanoseconds: 50_000_000)
      }
      try? await Task.sleep(nanoseconds: 200_000_000)
      captureScreenshot(window: window, to: outputDir.appending(path: "step_8_history_back.png"))

      exit(0)
    }
  }

  private var savedFrame: NSRect? {
    guard let data = UserDefaults.standard.data(forKey: "View.WindowFrame") else { return nil }
    return (try? NSKeyedUnarchiver.unarchivedObject(ofClass: NSValue.self, from: data))?.rectValue
  }

  private var frameObservers: [NSObjectProtocol] = []

  private func observeWindowFrame(_ window: NSWindow) {
    let center = NotificationCenter.default
    for name in [NSWindow.didMoveNotification, NSWindow.didResizeNotification, NSWindow.didEndLiveResizeNotification, NSWindow.didMiniaturizeNotification] {
      let observer = center.addObserver(forName: name, object: window, queue: .main) { [weak window] _ in
        guard let window else { return }
        if let data = try? NSKeyedArchiver.archivedData(withRootObject: NSValue(rect: window.frame), requiringSecureCoding: true) {
          UserDefaults.standard.set(data, forKey: "View.WindowFrame")
        }
      }
      frameObservers.append(observer)
    }
  }

  func applicationSupportsSecureRestorableState(_ app: NSApplication) -> Bool {
    true
  }

  func application(_ application: NSApplication, shouldRestoreSecureApplicationState coder: NSCoder) -> Bool {
    false
  }

  func application(_ application: NSApplication, shouldSaveSecureApplicationState coder: NSCoder) -> Bool {
    false
  }

  private func installSignalHandlers() {
    for code in [SIGINT, SIGTERM] {
      signalSources.append(Self.makeSignalSource(for: code))
    }
  }

  nonisolated private static func makeSignalSource(for code: Int32) -> DispatchSourceSignal {
    signal(code, SIG_IGN)
    let source = DispatchSource.makeSignalSource(signal: code, queue: DispatchQueue.global())
    source.setEventHandler {
      _exit(0)
    }
    source.resume()
    return source
  }

  func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
    true
  }

  func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
    guard shutdownTask == nil else { return .terminateLater }
    guard let controller = window?.contentViewController as? PreviewPanelController else { return .terminateNow }
    shutdownTask = Task { [weak self, controller] in
      await controller.cancelWork()
      NSApp.reply(toApplicationShouldTerminate: true)
      self?.shutdownTask = nil
    }
    return .terminateLater
  }

  private func installMainMenu(previewController: PreviewPanelController) {
    let appMenu = NSMenu()
    appMenu.addItem(
      withTitle: "Quit view",
      action: #selector(NSApplication.terminate(_:)),
      keyEquivalent: "q"
    )
    let appMenuItem = NSMenuItem()
    appMenuItem.submenu = appMenu
    let mainMenu = NSMenu()
    mainMenu.addItem(appMenuItem)

    let editMenu = NSMenu(title: "Edit")
    editMenu.addItem(menuItem(
      command: .undo,
      title: "Undo Move to Trash",
      action: #selector(PreviewPanelController.undo(_:)),
      target: previewController
    ))
    editMenu.addItem(menuItem(
      command: .redo,
      title: "Redo Move to Trash",
      action: #selector(PreviewPanelController.redo(_:)),
      target: previewController
    ))
    let editMenuItem = NSMenuItem(title: "Edit", action: nil, keyEquivalent: "")
    editMenuItem.submenu = editMenu
    mainMenu.addItem(editMenuItem)

    let viewMenu = NSMenu(title: "View")
    viewMenu.addItem(menuItem(
      command: .togglePreview,
      title: "Show Preview",
      action: #selector(PreviewPanelController.togglePreview(_:)),
      target: previewController
    ))
    viewMenu.addItem(.separator())
    for mode in ViewMode.allCases {
      let item = NSMenuItem(
        title: mode.title,
        action: #selector(PreviewPanelController.setViewMode(_:)),
        keyEquivalent: ""
      )
      item.target = previewController
      item.representedObject = mode.rawValue
      viewMenu.addItem(item)
    }
    viewMenu.addItem(.separator())
    viewMenu.addItem(menuItem(
      command: .trashSelectedItem,
      title: "Move to Trash",
      action: #selector(PreviewPanelController.trashSelectedItem(_:)),
      target: previewController
    ))
    let viewMenuItem = NSMenuItem(title: "View", action: nil, keyEquivalent: "")
    viewMenuItem.submenu = viewMenu
    mainMenu.addItem(viewMenuItem)

    NSApp.mainMenu = mainMenu
  }

  private func menuItem(
    command: GridCommand,
    title: String,
    action: Selector,
    target: AnyObject
  ) -> NSMenuItem {
    let shortcut = command.shortcut
    let item = NSMenuItem(title: title, action: action, keyEquivalent: shortcut.keyEquivalent)
    item.target = target
    item.keyEquivalentModifierMask = modifierFlags(for: shortcut.modifiers)
    return item
  }

  private func modifierFlags(for modifiers: Set<GridKeyModifier>) -> NSEvent.ModifierFlags {
    modifiers.reduce(into: []) { flags, modifier in
      switch modifier {
      case .command: flags.insert(.command)
      case .control: flags.insert(.control)
      case .option: flags.insert(.option)
      case .shift: flags.insert(.shift)
      }
    }
  }
}
