import Cocoa
import FlutterMacOS

class MainFlutterWindow: NSWindow {
  /// Talks to the Dart-side auto_updater (see main.dart `_updaterChannel`).
  private var updaterChannel: FlutterMethodChannel?

  override func awakeFromNib() {
    let flutterViewController = FlutterViewController()
    let windowFrame = self.frame
    self.contentViewController = flutterViewController
    self.setFrame(windowFrame, display: true)

    RegisterGeneratedPlugins(registry: flutterViewController)

    updaterChannel = FlutterMethodChannel(
      name: "land.jyn.jyn/updater",
      binaryMessenger: flutterViewController.engine.binaryMessenger)
    installCheckForUpdatesMenuItem()

    super.awakeFromNib()
  }

  /// Adds a standard "Check for Updates…" item to the application menu, just
  /// below "About jyn". Sparkle's update flow lives on the Dart side, so the
  /// item forwards to it over the method channel rather than targeting an
  /// SPUUpdater directly.
  private func installCheckForUpdatesMenuItem() {
    guard let appMenu = NSApp.mainMenu?.items.first?.submenu else { return }
    let item = NSMenuItem(
      title: "Check for Updates…",
      action: #selector(checkForUpdates),
      keyEquivalent: "")
    item.target = self
    appMenu.insertItem(item, at: 1)
    appMenu.insertItem(.separator(), at: 2)
  }

  @objc private func checkForUpdates() {
    updaterChannel?.invokeMethod("checkForUpdates", arguments: nil)
  }
}
