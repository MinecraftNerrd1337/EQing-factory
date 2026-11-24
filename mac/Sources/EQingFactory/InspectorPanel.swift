import SwiftUI

/// The tab's panel: AutoEQ settings, curves, filters.
struct InspectorPanel: View {
    let tab: TabInfo

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            AutoeqBox(tab: tab)
            CurvesBox(tab: tab)
            FiltersBox(tab: tab)
        }
    }
}

// MARK: - AutoEQ

struct AutoeqBox: View {
    let tab: TabInfo
    @Environment(AppModel.self) private var model

    var body: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    Text("Bands")
                    Spacer()
                    Stepper(
                        value: Binding(get: { Int(tab.bands) }, set: { model.setTabParams(bands: UInt32($0)) }),
                        in: 1...Int(model.core.maxBands())
                    ) {
                        Text("\(tab.bands)").monospacedDigit().frame(minWidth: 24, alignment: .trailing)
                    }
                }
                HStack {
                    Text("Steps")
                    Spacer()
                    NumberField(value: Double(tab.steps), fraction: 0, width: 80) { v in
                        model.setTabParams(steps: UInt32(max(1, min(1_000_000, v.rounded()))))
                    }
                }
                Picker("Mode", selection: Binding(get: { tab.mode }, set: { model.setTabParams(mode: $0) })) {
                    ForEach([Mode.inEar, .overEar, .precise], id: \.self) { Text($0.label).tag($0) }
                }
                HStack {
                    Button {
                        model.runAutoeq()
                    } label: {
                        Label("Run AutoEQ", systemImage: "wand.and.stars")
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(!model.canRun)
                    if model.isRunning { ProgressView().controlSize(.small) }
                    Spacer()
                }
                if tab.measurement == nil || tab.target == nil {
                    Text("The tab needs a measurement and a target.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                } else if let r = tab.lastRun {
                    Text("Last run: \(r.bands) band\(r.bands == 1 ? "" : "s"), \(r.steps) step\(r.steps == 1 ? "" : "s"), mode \(r.mode.letter), \(Int(r.durationMs.rounded())) ms (\(r.engine))")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            .padding(4)
        } label: {
            Text("AutoEQ").font(.headline)
        }
    }
}

// MARK: - Curves

struct CurvesBox: View {
    let tab: TabInfo
    @Environment(AppModel.self) private var model

    var body: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 6) {
                ForEach(tab.curves) { c in
                    CurveRow(curve: c)
                    Divider()
                }
                let hasFilters = tab.filters.contains(where: { $0.enabled })
                if tab.measurement != nil && (hasFilters || tab.compensation != nil) {
                    DerivedRow(
                        key: "result",
                        title: "Result",
                        subtitle: tab.compensation != nil ? "measurement − compensation + EQ" : "measurement + EQ",
                        color: tab.resultColor,
                        visible: tab.showResult,
                        what: .result
                    )
                }
                if hasFilters {
                    DerivedRow(key: "eq", title: "EQ", subtitle: "response of active filters", color: tab.eqColor, visible: tab.showEq, what: .eq)
                }
                HStack {
                    Menu {
                        Button("Measurement…") { model.openFile(role: .measurement) }
                        Button("Target…") { model.openFile(role: .target) }
                        Button("Compensation…") { model.openFile(role: .compensation) }
                        Button("Comparison…") { model.openFile(role: .compare) }
                        Divider()
                        Button("Paste CSV as comparison") { model.pasteCurve(role: .compare) }
                    } label: {
                        Label("Open", systemImage: "doc.badge.plus")
                    }
                    .fixedSize()
                    Spacer()
                }
                .padding(.top, 4)
            }
            .padding(4)
        } label: {
            Text("Curves").font(.headline)
        }
    }
}

struct CurveRow: View {
    let curve: CurveInfo
    @Environment(AppModel.self) private var model

    var body: some View {
        HStack(spacing: 6) {
            ColorPicker("", selection: Binding(get: { Color(hex: curve.color) }, set: { model.setColor(curve.id, $0) }), supportsOpacity: false)
                .labelsHidden()
                .frame(width: 34)
            Button { model.setVisible(curve.id, !curve.visible) } label: {
                Image(systemName: curve.visible ? "eye" : "eye.slash")
            }
            .buttonStyle(.borderless)
            .help(curve.visible ? "Hide" : "Show")
            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 4) {
                    Text(curve.label).lineLimit(1)
                    switch curve.role {
                    case .measurement: Badge(text: "Measurement", color: .blue)
                    case .target: Badge(text: "Target", color: .pink)
                    case .compensation: Badge(text: "Compensation", color: .teal)
                    case .compare: EmptyView()
                    }
                }
                Text(curve.fileName ?? curve.origin)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .help(curve.path ?? curve.origin)
            }
            Spacer(minLength: 4)
            NumberField(value: curve.offsetDb, fraction: 1, width: 52) { model.setOffset(curve, $0) }
                .help("Vertical offset (dB)")
            Stepper("", value: Binding(get: { curve.offsetDb }, set: { model.setOffset(curve, $0) }), step: 0.5)
                .labelsHidden()
            Menu {
                Button("Use as measurement") { model.setRole(curve, .measurement) }.disabled(curve.role == .measurement)
                Button("Use as target") { model.setRole(curve, .target) }.disabled(curve.role == .target)
                Button("Use as compensation") { model.setRole(curve, .compensation) }.disabled(curve.role == .compensation)
                Button("Comparison curve") { model.setRole(curve, .compare) }.disabled(curve.role == .compare)
                Divider()
                Button("Random color") { model.randomColor(curve.id) }
                Button("Align to target") { model.alignToTarget(curve) }.disabled(curve.role == .target)
                Button("Reset offset to 0") { model.setOffset(curve, 0) }
                Button("Rename…") { model.renameCurve(curve) }
                Divider()
                Button("Save as preset…") { model.newPresetFromCurve(.curve(id: curve.id)) }
                Button("Send to inbox…") { model.prepareSend(.curve(id: curve.id)) }
                Button("Export as CSV…") { model.exportCurveCSV(.curve(id: curve.id)) }
                Divider()
                Button("Remove from tab", role: .destructive) { model.removeCurve(curve) }
            } label: {
                Image(systemName: "ellipsis.circle")
            }
            .menuStyle(.button)
            .buttonStyle(.borderless)
            .menuIndicator(.hidden)
            .fixedSize()
        }
    }
}

/// "Result" and "EQ" rows, computed by the core from the measurement and the filters.
struct DerivedRow: View {
    let key: String
    let title: String
    let subtitle: String
    let color: String
    let visible: Bool
    let what: CurveRef
    @Environment(AppModel.self) private var model

    var body: some View {
        HStack(spacing: 6) {
            ColorPicker("", selection: Binding(get: { Color(hex: color) }, set: { model.setColor(key, $0) }), supportsOpacity: false)
                .labelsHidden()
                .frame(width: 34)
            Button { model.setVisible(key, !visible) } label: {
                Image(systemName: visible ? "eye" : "eye.slash")
            }
            .buttonStyle(.borderless)
            VStack(alignment: .leading, spacing: 1) {
                Text(title).fontWeight(.medium)
                Text(subtitle).font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
            Menu {
                Button("Random color") { model.randomColor(key) }
                if case .result = what {
                    Button("Keep as comparison curve") { model.snapshotResult() }
                }
                Divider()
                Button("Save as preset…") { model.newPresetFromCurve(what) }
                Button("Send to inbox…") { model.prepareSend(what) }
                Button("Export as CSV…") { model.exportCurveCSV(what) }
            } label: {
                Image(systemName: "ellipsis.circle")
            }
            .menuStyle(.button)
            .buttonStyle(.borderless)
            .menuIndicator(.hidden)
            .fixedSize()
        }
    }
}

// MARK: - Filters

struct FiltersBox: View {
    let tab: TabInfo
    @Environment(AppModel.self) private var model

    var body: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 6) {
                HStack {
                    Text("Preamp: \(String(format: "%.1f", tab.preamp)) dB")
                        .monospacedDigit()
                        .help("Minus the peak gain of the active filters (5 Hz – 22 kHz), minus 0.2 dB")
                    Spacer()
                    Menu("Import") {
                        Button("From file (add)…") { model.importFiltersFromFile(replace: false) }
                        Button("From file (replace)…") { model.importFiltersFromFile(replace: true) }
                        Divider()
                        Button("Paste (add)") { model.importFiltersFromClipboard(replace: false) }
                        Button("Paste (replace)") { model.importFiltersFromClipboard(replace: true) }
                    }
                    .fixedSize()
                    Menu("Export") {
                        ForEach([ExportFormat.parametric, .graphicEq, .filtersCsv], id: \.self) { f in
                            Button(f.label + "…") { model.exportEq(f) }
                        }
                        Divider()
                        ForEach([ExportFormat.parametric, .graphicEq], id: \.self) { f in
                            Button("Copy as " + f.label) { model.exportEq(f, toClipboard: true) }
                        }
                    }
                    .fixedSize()
                    .disabled(tab.filters.isEmpty)
                }

                if !tab.filters.isEmpty {
                    HStack(spacing: 6) {
                        Text("").frame(width: 18)
                        Text("").frame(width: 52)
                        Text("Type").frame(width: 72, alignment: .leading)
                        Text("Fc (Hz)").frame(width: 72, alignment: .trailing)
                        Text("Gain").frame(width: 56, alignment: .trailing)
                        Text("Q").frame(width: 56, alignment: .trailing)
                    }
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
                ForEach(tab.filters) { f in
                    FilterRow(filter: f)
                }

                HStack {
                    Button { model.addManualFilter() } label: { Label("Add filter", systemImage: "plus") }
                    Spacer()
                    Menu("Clear") {
                        Button("Auto filters") { model.clearFilters(onlyAuto: true) }
                        Button("All filters") { model.clearFilters(onlyAuto: false) }
                    }
                    .fixedSize()
                    .disabled(tab.filters.isEmpty)
                }
                .padding(.top, 4)
            }
            .padding(4)
        } label: {
            let auto = tab.filters.filter { $0.origin == .auto }.count
            Text("Filters (\(tab.filters.count)\(auto > 0 && auto < tab.filters.count ? ", \(auto) auto" : ""))")
                .font(.headline)
        }
    }
}

struct FilterRow: View {
    let filter: FilterInfo
    @Environment(AppModel.self) private var model

    var body: some View {
        HStack(spacing: 6) {
            Toggle("", isOn: Binding(get: { filter.enabled }, set: { model.updateFilter(filter, enabled: $0) }))
                .labelsHidden()
                .toggleStyle(.checkbox)
                .frame(width: 18)
            Badge(text: filter.origin.label, color: filter.origin.color)
                .frame(width: 52, alignment: .leading)
            Picker("", selection: Binding(get: { filter.kind }, set: { model.updateFilter(filter, kind: $0) })) {
                ForEach([FilterKind.peak, .lowShelf, .highShelf], id: \.self) { Text($0.label).tag($0) }
            }
            .labelsHidden()
            .frame(width: 72)
            NumberField(value: filter.f0, fraction: 1, width: 72) { model.updateFilter(filter, f0: $0) }
            NumberField(value: filter.gain, fraction: 2, width: 56) { model.updateFilter(filter, gain: $0) }
            NumberField(value: filter.q, fraction: 3, width: 56) { model.updateFilter(filter, q: $0) }
            Menu {
                if filter.origin == .auto {
                    Button("Keep as manual (survives next AutoEQ run)") { model.setOrigin(filter, .manual) }
                } else {
                    Button("Make auto (replaced on next AutoEQ run)") { model.setOrigin(filter, .auto) }
                }
                Divider()
                Button("Delete", role: .destructive) { model.removeFilter(filter) }
            } label: {
                Image(systemName: "ellipsis.circle")
            }
            .menuStyle(.button)
            .buttonStyle(.borderless)
            .menuIndicator(.hidden)
            .fixedSize()
        }
        .opacity(filter.enabled ? 1 : 0.55)
    }
}
