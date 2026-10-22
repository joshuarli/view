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

  init(root: RootDirectory, metrics: ViewMetrics) {
    self.root = root
    self.metrics = metrics
  }

  func applicationDidFinishLaunching(_ notification: Notification) {
    installMainMenu()

    let window = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: 1100, height: 760),
      styleMask: [.titled, .closable, .miniaturizable, .resizable],
      backing: .buffered,
      defer: false
    )
    window.title = root.url.lastPathComponent
    window.contentViewController = GridViewController(root: root, metrics: metrics)
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

  private func installMainMenu() {
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
    NSApp.mainMenu = mainMenu
  }
}
