import SwiftUI

/// Right column: the session's tabs, the chart and the tab's panel.
struct WorkspaceView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(spacing: 0) {
            TabStrip()
            Divider()
            if let tab = model.tab {
                HSplitView {
                    ChartArea(tab: tab)
                        .frame(minWidth: 420, maxWidth: .infinity, maxHeight: .infinity)
                    ScrollView {
                        InspectorPanel(tab: tab)
                            .padding(12)
                    }
                    .frame(minWidth: 360, idealWidth: 400, maxWidth: 560)
                }
            } else {
                ContentUnavailableView {
                    Label("No tabs", systemImage: "waveform.path.ecg")
                } description: {
                    Text("A tab is a workspace for building an EQ.")
                } actions: {
                    Button("New tab") { model.newTab() }
                        .fixedSize()
                }
            }
        }
        .toolbar {
            ToolbarItemGroup(placement: .primaryAction) {
                Button { model.undo() } label: { Label("Undo", systemImage: "arrow.uturn.backward") }
                    .disabled(!(model.tab?.canUndo ?? false))
                    .help("Undo (⌘Z)")
                Button { model.redo() } label: { Label("Redo", systemImage: "arrow.uturn.forward") }
                    .disabled(!(model.tab?.canRedo ?? false))
                    .help("Redo (⇧⌘Z)")
                Toggle(isOn: Binding(get: { model.inspectMode }, set: { model.inspectMode = $0 })) {
                    Label("Inspect", systemImage: "scope")
                }
                .help("Inspect mode: a line follows the pointer and shows the values (⌥⌘I)")
                Button { model.runAutoeq() } label: {
                    if model.isRunning {
                        ProgressView().controlSize(.small)
                    } else {
                        Label("AutoEQ", systemImage: "wand.and.stars")
                    }
                }
                .disabled(!model.canRun)
                .help("Run AutoEQ (⌘R)")
            }
        }
    }
}

// MARK: - Tab bar

struct TabStrip: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 4) {
                ForEach(model.tabs) { t in
                    TabChip(tab: t, active: t.id == model.activeTabId)
                }
                Button { model.newTab() } label: { Image(systemName: "plus") }
                    .buttonStyle(.borderless)
                    .help("New tab (⌘T)")
                    .padding(.horizontal, 6)
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
        }
    }
}

struct TabChip: View {
    let tab: TabSummary
    let active: Bool
    @Environment(AppModel.self) private var model
    @State private var hovering = false

    var body: some View {
        HStack(spacing: 6) {
            Circle()
                .fill(tab.hasMeasurement && tab.hasTarget ? Color.green : Color.secondary.opacity(0.4))
                .frame(width: 6, height: 6)
            Text(tab.name)
                .lineLimit(1)
            Button { model.closeTab(tab.id) } label: {
                Image(systemName: "xmark").font(.caption2.weight(.bold))
            }
            .buttonStyle(.borderless)
            .opacity(hovering || active ? 1 : 0)
            .help("Close tab")
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 4)
        .background(
            RoundedRectangle(cornerRadius: 6)
                .fill(active ? Color.accentColor.opacity(0.2) : (hovering ? Color.secondary.opacity(0.12) : Color.clear))
        )
        .contentShape(Rectangle())
        .onHover { hovering = $0 }
        .onTapGesture(count: 2) { model.renameTab(tab.id) }
        .onTapGesture { model.selectTab(tab.id) }
        .contextMenu {
            Button("Rename…") { model.renameTab(tab.id) }
            Button("Duplicate") { model.duplicateTab(tab.id) }
            Divider()
            Button("Move left") { model.moveTab(tab.id, by: -1) }
            Button("Move right") { model.moveTab(tab.id, by: 1) }
            Divider()
            Button("Close", role: .destructive) { model.closeTab(tab.id) }
        }
    }
}

// MARK: - Chart

struct ChartArea: View {
    let tab: TabInfo
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let plot = model.plot, !plot.series.isEmpty {
                Legend(series: plot.series)
                EQChart(plot: plot, inspect: model.inspectMode)
            } else {
                ContentUnavailableView {
                    Label("Empty tab", systemImage: "chart.xyaxis.line")
                } description: {
                    Text("Open a measurement (⌘O) and a target (⇧⌘O), or click a preset.")
                } actions: {
                    // fixedSize: the buttons keep their full label instead of being squeezed
                    HStack(spacing: 10) {
                        Button("Open measurement…") { model.openFile(role: .measurement) }
                            .fixedSize()
                        Button("Open target…") { model.openFile(role: .target) }
                            .fixedSize()
                    }
                    .controlSize(.large)
                    .fixedSize()
                }
            }
        }
        .padding(12)
    }
}

struct Legend: View {
    let series: [PlotSeries]

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 14) {
                ForEach(series) { s in
                    HStack(spacing: 5) {
                        RoundedRectangle(cornerRadius: 1)
                            .fill(Color(hex: s.color))
                            .frame(width: 14, height: s.kind == .target ? 3 : 2)
                        Text(s.label)
                            .font(.caption)
                            .lineLimit(1)
                    }
                }
            }
        }
    }
}
