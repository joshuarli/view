import AppKit
import viewCore

@main
struct ViewApp {
  private static var delegate: ViewAppDelegate?

  static func main() {
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
    let delegate = ViewAppDelegate(root: root.url)
    Self.delegate = delegate
    app.delegate = delegate
    app.run()
  }
}

@MainActor
final class ViewAppDelegate: NSObject, NSApplicationDelegate {
  private let root: URL
  private var window: NSWindow?

  init(root: URL) {
    self.root = root
  }

  func applicationDidFinishLaunching(_ notification: Notification) {
    installMainMenu()

    let window = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: 1100, height: 760),
      styleMask: [.titled, .closable, .miniaturizable, .resizable],
      backing: .buffered,
      defer: false
    )
    window.title = root.lastPathComponent
    window.contentViewController = RootViewController(root: root)
    window.setContentSize(NSSize(width: 1100, height: 760))
    window.center()
    window.makeKeyAndOrderFront(nil)
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
