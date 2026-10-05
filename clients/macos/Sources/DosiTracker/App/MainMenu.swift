import AppKit

// The main menu of a menu-bar (LSUIElement) app. It is never shown, but its
// key equivalents still work while one of the app's windows is key — without
// it ⌘V, ⌘C, ⌘A and ⌘Z would do nothing in the sign-in fields (pasting a
// server address or a password), and ⌘W / ⌘Q would not close or quit.

@MainActor
enum MainMenu {
    /// Install the menu. `settings` is sent (to `target`) by ⌘, ("Settings…").
    static func install(settings: Selector, target: AnyObject) {
        let main = NSMenu()

        let appItem = NSMenuItem()
        let appMenu = NSMenu(title: "Dosi Tracker")
        appMenu.addItem(withTitle: "About Dosi Tracker",
                        action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)), keyEquivalent: "")
        appMenu.addItem(.separator())
        let settingsItem = appMenu.addItem(withTitle: "Settings…", action: settings, keyEquivalent: ",")
        settingsItem.target = target
        appMenu.addItem(.separator())
        appMenu.addItem(withTitle: "Hide Dosi Tracker", action: #selector(NSApplication.hide(_:)), keyEquivalent: "h")
        appMenu.addItem(.separator())
        appMenu.addItem(withTitle: "Quit Dosi Tracker", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        appItem.submenu = appMenu
        main.addItem(appItem)

        let editItem = NSMenuItem()
        let editMenu = NSMenu(title: "Edit")
        editMenu.addItem(withTitle: "Undo", action: Selector(("undo:")), keyEquivalent: "z")
        let redo = editMenu.addItem(withTitle: "Redo", action: Selector(("redo:")), keyEquivalent: "z")
        redo.keyEquivalentModifierMask = [.command, .shift]
        editMenu.addItem(.separator())
        editMenu.addItem(withTitle: "Cut", action: #selector(NSText.cut(_:)), keyEquivalent: "x")
        editMenu.addItem(withTitle: "Copy", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
        editMenu.addItem(withTitle: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
        editMenu.addItem(withTitle: "Select All", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")
        editItem.submenu = editMenu
        main.addItem(editItem)

        let windowItem = NSMenuItem()
        let windowMenu = NSMenu(title: "Window")
        windowMenu.addItem(withTitle: "Minimize", action: #selector(NSWindow.performMiniaturize(_:)), keyEquivalent: "m")
        windowMenu.addItem(withTitle: "Close", action: #selector(NSWindow.performClose(_:)), keyEquivalent: "w")
        windowItem.submenu = windowMenu
        main.addItem(windowItem)

        NSApp.mainMenu = main
    }
}
