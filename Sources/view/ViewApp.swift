import AppKit
import viewCore

public enum ViewApp {
  private static var delegate: ViewAppDelegate?

  public static func main() {
    let arguments = Array(CommandLine.arguments.dropFirst())
    if arguments.contains("--help") || arguments.contains("-h") {
      printUsage()
      return
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
    let delegate = ViewAppDelegate(root: options.root, mode: options.mode, metrics: metrics)
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
  private var window: NSWindow?
  private var shutdownTask: Task<Void, Never>?
  private var signalSources: [DispatchSourceSignal] = []

  init(root: RootDirectory, mode: BrowserMode, metrics: ViewMetrics) {
    self.root = root
    self.mode = mode
    self.metrics = metrics
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
    window.title = root.url.lastPathComponent
    let previewController = PreviewPanelController(root: root, mode: mode, metrics: metrics)
    window.contentViewController = previewController
    installMainMenu(previewController: previewController)
    window.setContentSize(NSSize(width: 1100, height: 760))
    window.center()
    window.makeKeyAndOrderFront(nil)
    metrics.windowShown()
    self.window = window
    NSApp.activate()
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
