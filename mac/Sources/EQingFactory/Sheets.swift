import SwiftUI

struct TextPromptSheet: View {
    let prompt: TextPrompt
    @State private var text = ""
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(prompt.title).font(.headline)
            if let m = prompt.message { Text(m).foregroundStyle(.secondary) }
            TextField("", text: $text)
                .textFieldStyle(.roundedBorder)
                .frame(minWidth: 320)
                .onSubmit(confirm)
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(prompt.confirmLabel, action: confirm)
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .onAppear { text = prompt.text }
    }

    private func confirm() {
        let value = text
        dismiss()
        prompt.onConfirm(value)
    }
}

// MARK: - New preset (with duplicate detection)

struct PresetDraftSheet: View {
    @State var draft: PresetDraft
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss

    private var isFile: Bool {
        if case .file = draft.source { return true }
        return false
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("New preset").font(.headline)

            if !draft.duplicates.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    Label("This file already matches \(draft.duplicates.count == 1 ? "a preset" : "\(draft.duplicates.count) presets")", systemImage: "doc.on.doc")
                        .fontWeight(.medium)
                    ForEach(draft.duplicates) { d in
                        HStack {
                            Text("\"\(d.presetName)\"")
                            Text(d.samePath ? "same path" : "same content")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                            Spacer()
                            if d.visibleInSession {
                                Text("already visible here").font(.caption).foregroundStyle(.secondary)
                            } else {
                                Button("Make visible here") {
                                    model.grantHere(d.presetId)
                                    dismiss()
                                }
                            }
                        }
                    }
                    Text("You can also create a deliberate duplicate below.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .padding(10)
                .background(Color.orange.opacity(0.1), in: RoundedRectangle(cornerRadius: 8))
            }

            Form {
                TextField("Name", text: $draft.name)
                Picker("Category", selection: $draft.category) {
                    ForEach([PresetCategory.measurement, .target, .compensation], id: \.self) { Text($0.label).tag($0) }
                }
                if isFile {
                    Toggle("Embed the data (the preset no longer depends on the file)", isOn: $draft.embed)
                }
                Picker("Visible in", selection: $draft.visibility) {
                    Text("All sessions").tag(VisibilityChoice.appWide)
                    Text("This session only").tag(VisibilityChoice.thisSession)
                }
                .pickerStyle(.radioGroup)
            }

            HStack {
                Text("Visibility can be fine-tuned later (right-click › Visibility…).")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(draft.duplicates.isEmpty ? "Create" : "Create duplicate") {
                    let d = draft
                    dismiss()
                    model.commitPresetDraft(d)
                }
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 520)
    }
}

// MARK: - Preset visibility

struct VisibilitySheet: View {
    let preset: PresetInfo
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var mode: VisibilityMode = .allExcept
    @State private var selected: Set<String> = []

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Visibility of \"\(preset.name)\"").font(.headline)
            Picker("", selection: $mode) {
                Text("All sessions except the checked ones").tag(VisibilityMode.allExcept)
                Text("Only the checked sessions").tag(VisibilityMode.onlyListed)
            }
            .pickerStyle(.radioGroup)
            .labelsHidden()

            List {
                ForEach(model.sessions) { s in
                    Toggle(s.name, isOn: binding(s.id))
                }
                ForEach(model.zombies) { z in
                    Toggle("\(z.name) (deleted)", isOn: binding(z.id))
                        .foregroundStyle(.secondary)
                }
            }
            .frame(minHeight: 180)

            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Save") {
                    model.saveVisibility(preset, Visibility(mode: mode, sessions: Array(selected).sorted()))
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 440, height: 420)
        .onAppear {
            mode = preset.visibility.mode
            selected = Set(preset.visibility.sessions)
        }
    }

    private func binding(_ id: String) -> Binding<Bool> {
        Binding(
            get: { selected.contains(id) },
            set: { on in
                if on { selected.insert(id) } else { selected.remove(id) }
            }
        )
    }
}

// MARK: - Send to inbox

struct InboxSendSheet: View {
    @State var draft: InboxDraft
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Send to inbox").font(.headline)
            Text(draft.label).foregroundStyle(.secondary)
            Form {
                TextField("Note", text: $draft.note, prompt: Text("to help you find it, e.g. \"Aria, take 3\""))
                Picker("Destination", selection: $draft.toSession) {
                    Text("All sessions").tag(String?.none)
                    ForEach(model.sessions) { s in
                        Text(s.id == model.activeSessionId ? "\(s.name) (this session)" : s.name).tag(Optional(s.id))
                    }
                }
            }
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Send") {
                    let d = draft
                    dismiss()
                    model.commitSend(d)
                }
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 460)
    }
}

// MARK: - Session settings

struct SessionDefaultsSheet: View {
    let session: SessionInfo
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss

    @State private var overrideBands = false
    @State private var bands = 20
    @State private var overrideSteps = false
    @State private var steps = 3000
    @State private var overrideMode = false
    @State private var mode: Mode = .inEar

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Settings for \"\(session.name)\"").font(.headline)
            Text("Defaults for new tabs in this session. Unchecked: use the app default.")
                .font(.caption)
                .foregroundStyle(.secondary)
            Form {
                HStack {
                    Toggle("Bands", isOn: $overrideBands)
                    Spacer()
                    Stepper(value: $bands, in: 1...Int(model.core.maxBands())) { Text("\(bands)").monospacedDigit() }
                        .disabled(!overrideBands)
                }
                HStack {
                    Toggle("Steps", isOn: $overrideSteps)
                    Spacer()
                    TextField("", value: $steps, format: .number)
                        .frame(width: 90)
                        .disabled(!overrideSteps)
                }
                HStack {
                    Toggle("Mode", isOn: $overrideMode)
                    Spacer()
                    Picker("", selection: $mode) {
                        ForEach([Mode.inEar, .overEar, .precise], id: \.self) { Text($0.label).tag($0) }
                    }
                    .labelsHidden()
                    .frame(width: 220)
                    .disabled(!overrideMode)
                }
            }
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Save") {
                    model.saveSessionDefaults(
                        session.id,
                        bands: overrideBands ? UInt32(bands) : nil,
                        steps: overrideSteps ? UInt32(max(1, steps)) : nil,
                        mode: overrideMode ? mode : nil
                    )
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 460)
        .onAppear {
            let eff = model.core.effectiveDefaults(session: session.id)
            overrideBands = session.bands != nil
            bands = Int(eff.bands)
            overrideSteps = session.steps != nil
            steps = Int(eff.steps)
            overrideMode = session.mode != nil
            mode = eff.mode
        }
    }
}
