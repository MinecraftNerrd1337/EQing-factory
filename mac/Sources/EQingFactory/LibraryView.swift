import SwiftUI

/// Middle column: presets, history and inbox of the active session.
struct LibraryView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var model = model
        VStack(spacing: 0) {
            Picker("", selection: $model.libraryPane) {
                ForEach(LibraryPane.allCases) { pane in
                    if pane == .inbox && !model.inbox.isEmpty {
                        Text("\(pane.title) (\(model.inbox.count))").tag(pane)
                    } else {
                        Text(pane.title).tag(pane)
                    }
                }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .frame(maxWidth: .infinity)
            .padding(.horizontal, 10)
            .padding(.vertical, 8)

            Divider()

            switch model.libraryPane {
            case .presets: PresetsList()
            case .history: HistoryList()
            case .inbox: InboxList()
            }
        }
        .navigationTitle(model.activeSession?.name ?? "EQing Factory")
    }
}

// MARK: - Presets

struct PresetsList: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        List {
            ForEach(model.presets) { p in
                PresetRow(preset: p)
            }
        }
        .overlay {
            if model.presets.isEmpty {
                EmptyHint(
                    symbol: "square.stack",
                    title: "No presets",
                    message: "Create one from a file, or from a curve's ⋯ menu in a tab."
                )
            }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            BottomBar {
                Button { model.newPresetFromFile() } label: {
                    Label("From file…", systemImage: "plus")
                }
                Spacer()
                Toggle("All", isOn: Binding(get: { model.showAllPresets }, set: { _ in model.toggleShowAllPresets() }))
                    .toggleStyle(.checkbox)
                    .help("Also show presets hidden in this session")
            }
        }
    }
}

struct PresetRow: View {
    let preset: PresetInfo
    @Environment(AppModel.self) private var model

    var body: some View {
        Button {
            model.addPreset(preset, role: nil)
        } label: {
            HStack(spacing: 8) {
                Image(systemName: preset.category.symbol)
                    .frame(width: 18)
                    .foregroundStyle(.secondary)
                VStack(alignment: .leading, spacing: 1) {
                    HStack(spacing: 4) {
                        Text(preset.name).lineLimit(1)
                        if !preset.visibleHere {
                            Badge(text: "hidden here", color: .secondary)
                        }
                    }
                    HStack(spacing: 4) {
                        Image(systemName: preset.embedded ? "shippingbox" : "link")
                        Text(preset.fileName).lineLimit(1).truncationMode(.middle)
                    }
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
                Spacer(minLength: 4)
                if !preset.available {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                        .help("File not found")
                }
                if preset.pinned {
                    Image(systemName: "pin.fill")
                        .foregroundStyle(.orange)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .opacity(preset.visibleHere ? 1 : 0.6)
        .help("Click to add to the current tab")
        .contextMenu {
            Button("Add as measurement") { model.addPreset(preset, role: .measurement) }
            Button("Add as target") { model.addPreset(preset, role: .target) }
            Button("Add as compensation") { model.addPreset(preset, role: .compensation) }
            Button("Add as comparison") { model.addPreset(preset, role: .compare) }
            Divider()
            Button(preset.pinned ? "Unpin" : "Pin") { model.togglePin(preset) }
            Button("Rename…") { model.renamePreset(preset) }
            Menu("Category") {
                ForEach([PresetCategory.measurement, .target, .compensation], id: \.self) { c in
                    Button {
                        model.setCategory(preset, c)
                    } label: {
                        if c == preset.category { Label(c.label, systemImage: "checkmark") } else { Text(c.label) }
                    }
                }
            }
            Button("Visibility…") { model.visibilityEdit = preset }
            if !preset.visibleHere {
                Button("Make visible in this session") { model.grantHere(preset.id) }
            }
            Divider()
            Button(preset.embedded ? "Replace data from file…" : "Change file…") {
                model.updatePresetFromFile(preset)
            }
            if !preset.embedded {
                Button("Embed data (unlink from file)") { model.embedPreset(preset) }
                    .disabled(!preset.available)
            }
            Button("Export as CSV…") { model.exportPresetCSV(preset) }
            Divider()
            Button("Delete…", role: .destructive) { model.deletePreset(preset) }
        }
    }
}

// MARK: - History

struct HistoryList: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        List {
            ForEach(model.history) { h in
                HistoryRow(entry: h)
            }
        }
        .overlay {
            if model.history.isEmpty {
                EmptyHint(
                    symbol: "clock.arrow.circlepath",
                    title: "No history",
                    message: "Files opened in this session will appear here."
                )
            }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if !model.history.isEmpty {
                BottomBar {
                    Button("Remove missing files") { model.clearHistory(onlyMissing: true) }
                    Spacer()
                    Button("Clear all") { model.clearHistory(onlyMissing: false) }
                }
            }
        }
    }
}

struct HistoryRow: View {
    let entry: HistoryInfo
    @Environment(AppModel.self) private var model

    var body: some View {
        HStack(spacing: 6) {
            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 4) {
                    if !entry.exists {
                        Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.orange)
                    }
                    Text(entry.fileName).lineLimit(1)
                }
                Text((entry.path as NSString).deletingLastPathComponent)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.head)
                Text(formatDate(entry.openedAt))
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
            }
            Spacer(minLength: 4)
            HStack(spacing: 2) {
                Button("M") { model.openHistory(entry, role: .measurement) }.help("Open as measurement")
                Button("T") { model.openHistory(entry, role: .target) }.help("Open as target")
                Button { model.openHistory(entry, role: .compare) } label: { Image(systemName: "plus") }
                    .help("Add as comparison")
            }
            .buttonStyle(.bordered)
            .controlSize(.small)
        }
        .opacity(entry.exists ? 1 : 0.6)
        .contextMenu {
            Button("Open as measurement") { model.openHistory(entry, role: .measurement) }
            Button("Open as target") { model.openHistory(entry, role: .target) }
            Button("Open as compensation") { model.openHistory(entry, role: .compensation) }
            Button("Add as comparison") { model.openHistory(entry, role: .compare) }
            Divider()
            Button("Create preset…") { model.presetFromHistory(entry) }
                .disabled(!entry.exists)
            Button("Remove from history") { model.removeHistory(entry) }
        }
    }
}

// MARK: - Inbox

struct InboxList: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        List {
            ForEach(model.inbox) { item in
                InboxRow(item: item)
            }
        }
        .overlay {
            if model.inbox.isEmpty {
                EmptyHint(
                    symbol: "tray",
                    title: "Inbox is empty",
                    message: "Send a result or curve from a tab (a curve's ⋯ menu) to find it here or in another session."
                )
            }
        }
    }
}

struct InboxRow: View {
    let item: InboxInfo
    @Environment(AppModel.self) private var model

    private var route: String {
        let to = item.toSession == nil ? "all sessions" : (item.toSessionName ?? "?")
        return item.sentFromHere ? "sent to \(to)" : "from \(item.fromSessionName)"
    }

    private var filtersSuffix: String {
        let n = item.filterCount
        guard n > 0 else { return "" }
        return " · \(n) " + (n == 1 ? "filter" : "filters")
    }

    var body: some View {
        HStack(spacing: 6) {
            VStack(alignment: .leading, spacing: 1) {
                Text(item.note.isEmpty ? item.label : item.note)
                    .fontWeight(.medium)
                    .lineLimit(2)
                if !item.note.isEmpty {
                    Text(item.label).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
                Text("\(route) · \(formatDate(item.createdAt))\(filtersSuffix)")
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
            }
            Spacer(minLength: 4)
            HStack(spacing: 2) {
                Button("M") { model.takeInbox(item, role: .measurement) }.help("Use as measurement")
                Button("T") { model.takeInbox(item, role: .target) }.help("Use as target")
                Button { model.takeInbox(item, role: .compare) } label: { Image(systemName: "plus") }
                    .help("Add as comparison")
            }
            .buttonStyle(.bordered)
            .controlSize(.small)
        }
        .contextMenu {
            Button("Use as measurement") { model.takeInbox(item, role: .measurement) }
            Button("Use as target") { model.takeInbox(item, role: .target) }
            Button("Use as compensation") { model.takeInbox(item, role: .compensation) }
            Button("Add as comparison") { model.takeInbox(item, role: .compare) }
            if item.filterCount > 0 {
                Divider()
                Button("Replace tab's filters with these") { model.applyInboxFilters(item, replace: true) }
                Button("Add these filters to tab") { model.applyInboxFilters(item, replace: false) }
            }
            Divider()
            Button("Edit note…") { model.editInboxNote(item) }
            Button("Export as CSV…") { model.exportInboxCSV(item) }
            Button("Delete", role: .destructive) { model.deleteInbox(item) }
        }
    }
}
