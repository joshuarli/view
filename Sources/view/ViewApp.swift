import AppKit
import viewCore

@main
struct ViewApp {
  private static var delegate: ViewAppDelegate?

  static func main() {
    let metrics = ViewMetrics(enabled: ProcessInfo.processInfo.environment["VIEW_METRICS"] == "1")
    let root: RootDirectory
    do {
      root = try RootDirectory.resolve(
        argument: CommandLine.arguments.dropFirst().first,
        currentDirectory: FileManager.default.currentDirectoryPath
      )
    } catch {
      FileHandle.standardError.write(Data("view: \(error.localizedDescription)\n".utf8))
      exit(1)
    }

    let app = NSApplication.shared
    app.setActivationPolicy(.regular)
    let delegate = ViewAppDelegate(root: root, metrics: metrics)
    Self.delegate = delegate
    app.delegate = delegate
    app.run()
  }
}

@MainActor
final class ViewAppDelegate: NSObject, NSApplicationDelegate {
  private let root: RootDirectory
  private let metrics: ViewMetrics
  private var window: NSWindow?
  private var shutdownTask: Task<Void, Never>?

  init(root: RootDirectory, metrics: ViewMetrics) {
    self.root = root
    self.metrics = metrics
  }

  func applicationDidFinishLaunching(_ notification: Notification) {
    let window = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: 1100, height: 760),
      styleMask: [.titled, .closable, .miniaturizable, .resizable],
      backing: .buffered,
      defer: false
    )
    window.title = root.url.lastPathComponent
    let previewController = PreviewPanelController(root: root, metrics: metrics)
    window.contentViewController = previewController
    installMainMenu(previewController: previewController)
    window.setContentSize(NSSize(width: 1100, height: 760))
    window.center()
    window.makeKeyAndOrderFront(nil)
    metrics.windowShown()
    self.window = window
    NSApp.activate()
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
