import SwiftUI
import AppKit

/// App settings (⌘,).
struct SettingsView: View {
    @Environment(AppModel.self) private var model

    private func bind<T>(_ kp: WritableKeyPath<AppSettings, T>) -> Binding<T> {
        Binding(
            get: { model.settings[keyPath: kp] },
            set: { v in
                var s = model.settings
                s[keyPath: kp] = v
                model.updateSettings(s)
            }
        )
    }

    private var isExternal: Binding<Bool> {
        Binding(
            get: { if case .external = model.settings.engine { return true } else { return false } },
            set: { on in
                var s = model.settings
                s.engine = on ? .external(path: externalPath, args: externalArgs) : .builtinOrig
                model.updateSettings(s)
            }
        )
    }

    private var externalPath: String {
        if case .external(let p, _) = model.settings.engine { return p }
        return ""
    }

    private var externalArgs: String {
        if case .external(_, let a) = model.settings.engine { return a }
        return ""
    }

    private func setExternal(path: String? = nil, args: String? = nil) {
        var s = model.settings
        s.engine = .external(path: path ?? externalPath, args: args ?? externalArgs)
        model.updateSettings(s)
    }

    var body: some View {
        TabView {
            Form {
                Picker("Appearance", selection: bind(\.theme)) {
                    ForEach([Theme.system, .light, .dark], id: \.self) { Text($0.label).tag($0) }
                }
                .pickerStyle(.segmented)
                Stepper(
                    value: Binding(get: { Int(model.settings.historyLimit) }, set: { v in
                        var s = model.settings
                        s.historyLimit = UInt32(max(1, v))
                        model.updateSettings(s)
                    }),
                    in: 5...500,
                    step: 5
                ) {
                    Text("History: \(model.settings.historyLimit) files per session")
                }
                LabeledContent("Data") {
                    HStack {
                        Text(model.dataDir.path)
                            .font(.caption)
                            .textSelection(.enabled)
                            .lineLimit(1)
                            .truncationMode(.middle)
                        Button("Show in Finder") { NSWorkspace.shared.activateFileViewerSelecting([model.dataDir]) }
                    }
                }
            }
            .padding(20)
            .tabItem { Label("General", systemImage: "gearshape") }

            Form {
                Stepper(
                    value: Binding(get: { Int(model.settings.defaultBands) }, set: { v in
                        var s = model.settings
                        s.defaultBands = UInt32(v)
                        model.updateSettings(s)
                    }),
                    in: 1...Int(model.core.maxBands())
                ) {
                    Text("Default bands: \(model.settings.defaultBands)")
                }
                LabeledContent("Default steps") {
                    NumberField(value: Double(model.settings.defaultSteps), fraction: 0, width: 90) { v in
                        var s = model.settings
                        s.defaultSteps = UInt32(max(1, min(1_000_000, v.rounded())))
                        model.updateSettings(s)
                    }
                }
                Picker("Default mode", selection: bind(\.defaultMode)) {
                    ForEach([Mode.inEar, .overEar, .precise], id: \.self) { Text($0.label).tag($0) }
                }
                Toggle("Align measurement to target after each AutoEQ run", isOn: bind(\.autoAlign))
                Text("These defaults apply to new tabs; each session can override them (right-click › Session settings…).")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .padding(20)
            .tabItem { Label("AutoEQ", systemImage: "wand.and.stars") }

            Form {
                Toggle("Use an external autoeq binary", isOn: isExternal)
                Text("By default, the \"orig\" autoeq-c is built into the app (direct call, no separate process). An external binary is called the same way as the autoeq binary: `<binary> <mode> <bands> <steps> <arguments>`, with the target then the measurement as float32 on stdin.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                if isExternal.wrappedValue {
                    LabeledContent("Binary") {
                        HStack {
                            Text(externalPath.isEmpty ? "none" : externalPath)
                                .lineLimit(1)
                                .truncationMode(.middle)
                            Button("Choose…") {
                                if let url = FilePanels.openApplication(title: "autoeq binary") { setExternal(path: url.path) }
                            }
                        }
                    }
                    LabeledContent("Extra arguments") {
                        TextField("", text: Binding(get: { externalArgs }, set: { setExternal(args: $0) }), prompt: Text("e.g. --treble-start 18000"))
                            .frame(width: 260)
                    }
                }
                Text(model.core.version())
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
            }
            .padding(20)
            .tabItem { Label("Engine", systemImage: "cpu") }
        }
        .frame(width: 560, height: 340)
    }
}
