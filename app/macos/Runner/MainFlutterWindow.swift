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

    // Content-first chrome: the near-white surface runs edge-to-edge under a
    // transparent titlebar, traffic lights floating over it (the Flutter side
    // paints the design's 34px titlebar strip; see JynTitlebarStrip).
    self.titlebarAppearsTransparent = true
    self.titleVisibility = .hidden
    self.styleMask.insert(.fullSizeContentView)

    // Default to the design's window proportions on first launch; remember
    // whatever the user resizes to afterwards (via our autosave only —
    // window-server restoration would silently override the default).
    self.isRestorable = false
    self.minSize = NSSize(width: 480, height: 480)
    let shotMode = ProcessInfo.processInfo.environment["JYN_SHOT"] != nil
    if shotMode {
      // Screenshot harness: always the design frame, never persisted.
      self.setContentSize(NSSize(width: 680, height: 912))
      self.center()
    } else {
      if !self.setFrameUsingName("JynMainWindow") {
        self.setContentSize(NSSize(width: 680, height: 912))
        self.center()
      }
      self.setFrameAutosaveName("JynMainWindow")
    }

    RegisterGeneratedPlugins(registry: flutterViewController)

    updaterChannel = FlutterMethodChannel(
      name: "app.jyn.jyn/updater",
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
