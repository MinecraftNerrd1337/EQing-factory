import SwiftUI
import AppKit

@main
struct EQingFactoryApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @State private var model = AppModel()

    var body: some Scene {
        // single window: the menu bar extra brings it back to the front
        Window("EQing Factory", id: "main") {
            ContentView()
                .environment(model)
                .frame(minWidth: 1100, minHeight: 660)
                .onAppear {
                    appDelegate.model = model
                    model.applyTheme()
                }
        }
        .commands { AppCommands(model: model) }

        // a bare `Settings` would refer to the uniFFI-generated struct
        SwiftUI.Settings {
            SettingsView()
                .environment(model)
        }

        MenuBarExtra("EQing Factory", systemImage: "slider.vertical.3") {
            MenuBarContent()
                .environment(model)
        }
    }
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    weak var model: AppModel?

    func applicationDidFinishLaunching(_ notification: Notification) {
        // useful when the executable is launched outside a .app bundle (swift run)
        NSApplication.shared.setActivationPolicy(.regular)
        NSApplication.shared.activate(ignoringOtherApps: true)
    }

    func applicationWillTerminate(_ notification: Notification) {
        model?.saveNow()
    }
}

struct AppCommands: Commands {
    let model: AppModel

    var body: some Commands {
        CommandGroup(replacing: .newItem) {
            Button("New tab") { model.newTab() }
                .keyboardShortcut("t")
            Button("New session") { model.newSession(folder: nil) }
                .keyboardShortcut("n", modifiers: [.command, .option])
            Divider()
            Button("Open measurement…") { model.openFile(role: .measurement) }
                .keyboardShortcut("o")
            Button("Open target…") { model.openFile(role: .target) }
                .keyboardShortcut("o", modifiers: [.command, .shift])
            Button("Open compensation…") { model.openFile(role: .compensation) }
            Button("Add comparison curves…") { model.openFile(role: .compare) }
                .keyboardShortcut("o", modifiers: [.command, .option])
            Divider()
            Button("New preset from file…") { model.newPresetFromFile() }
            Button("Import filters…") { model.importFiltersFromFile(replace: false) }
            Menu("Export EQ") {
                ForEach([ExportFormat.parametric, .graphicEq, .filtersCsv], id: \.self) { f in
                    Button(f.label + "…") { model.exportEq(f) }
                }
            }
            .disabled(model.tab?.filters.isEmpty ?? true)
            Divider()
            Button("Close tab") { model.closeActiveTab() }
                .keyboardShortcut("w", modifiers: [.command, .option])
        }

        CommandGroup(replacing: .undoRedo) {
            Button("Undo") { model.undo() }
                .keyboardShortcut("z")
                .disabled(!(model.tab?.canUndo ?? false))
            Button("Redo") { model.redo() }
                .keyboardShortcut("z", modifiers: [.command, .shift])
                .disabled(!(model.tab?.canRedo ?? false))
        }

        CommandMenu("EQ") {
            Button("Run AutoEQ") { model.runAutoeq() }
                .keyboardShortcut("r")
                .disabled(!model.canRun)
            Button("Add filter") { model.addManualFilter() }
                .keyboardShortcut("f", modifiers: [.command, .option])
            Divider()
            Button("Clear auto filters") { model.clearFilters(onlyAuto: true) }
            Button("Clear all filters") { model.clearFilters(onlyAuto: false) }
            Divider()
            Toggle("Inspect mode", isOn: Binding(get: { model.inspectMode }, set: { model.inspectMode = $0 }))
                .keyboardShortcut("i", modifiers: [.command, .option])
        }

        CommandGroup(after: .sidebar) {
            Picker("Appearance", selection: Binding(get: { model.settings.theme }, set: { model.setTheme($0) })) {
                ForEach([Theme.system, .light, .dark], id: \.self) { Text($0.label).tag($0) }
            }
        }
    }
}

struct MenuBarContent: View {
    @Environment(AppModel.self) private var model
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Text("Session: \(model.activeSession?.name ?? "—")")
        Menu("Switch session") {
            ForEach(model.sessions) { s in
                Button {
                    model.selectSession(s.id)
                } label: {
                    if s.id == model.activeSessionId {
                        Label(s.name, systemImage: "checkmark")
                    } else {
                        Text(s.name)
                    }
                }
            }
        }
        Divider()
        Button("New tab") {
            model.newTab()
            show()
        }
        Button("Show window") { show() }
        Divider()
        Button("Quit EQing Factory") { NSApplication.shared.terminate(nil) }
    }

    private func show() {
        openWindow(id: "main")
        NSApplication.shared.activate(ignoringOtherApps: true)
    }
}
