import SwiftUI
import AppKit
import Observation

enum LibraryPane: String, CaseIterable, Identifiable {
    case presets, history, inbox
    var id: String { rawValue }
    var title: String {
        switch self {
        case .presets: return "Presets"
        case .history: return "History"
        case .inbox: return "Inbox"
        }
    }
}

struct AlertInfo: Identifiable {
    let id = UUID()
    var title: String
    var message: String
    var actionLabel: String? = nil
    var destructive = false
    var action: (() -> Void)? = nil
}

struct TextPrompt: Identifiable {
    let id = UUID()
    var title: String
    var message: String? = nil
    var text: String
    var confirmLabel = "OK"
    var onConfirm: (String) -> Void
}

enum PresetSourceDraft {
    case file(path: String)
    case curve(tab: String, what: CurveRef)
}

enum VisibilityChoice: Hashable {
    case appWide
    case thisSession
}

struct PresetDraft: Identifiable {
    let id = UUID()
    var source: PresetSourceDraft
    var name: String
    var category: PresetCategory
    var embed: Bool
    var visibility: VisibilityChoice = .appWide
    var duplicates: [DuplicateInfo]
}

struct InboxDraft: Identifiable {
    let id = UUID()
    var tab: String
    var what: CurveRef
    var label: String
    var note: String = ""
    /// nil = all sessions
    var toSession: String? = nil
}

/// State observed by the UI. All the logic lives in the Rust core:
/// each action calls `core`, then re-reads whatever may have changed.
@MainActor
@Observable
final class AppModel {
    let core: AppCore
    let dataDir: URL

    var settings: AppSettings
    var folders: [FolderInfo] = []
    var sessions: [SessionInfo] = []
    var zombies: [SessionInfo] = []
    var activeSessionId = ""
    var tabs: [TabSummary] = []
    var activeTabId: String?
    var tab: TabInfo?
    var plot: PlotData?
    var presets: [PresetInfo] = []
    var showAllPresets = false
    var history: [HistoryInfo] = []
    var inbox: [InboxInfo] = []
    var libraryPane: LibraryPane = .presets
    var inspectMode = true
    var isRunning = false

    var alert: AlertInfo?
    var prompt: TextPrompt?
    var presetDraft: PresetDraft?
    var visibilityEdit: PresetInfo?
    var inboxDraft: InboxDraft?
    var sessionDefaultsEdit: SessionInfo?

    @ObservationIgnored private var saveTask: Task<Void, Never>?

    static let plotPoints: UInt32 = 480

    init() {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSHomeDirectory())
        dataDir = base.appendingPathComponent("EQing Factory", isDirectory: true)
        var startupError: String?
        let c: AppCore
        do {
            c = try AppCore(dataDir: dataDir.path)
        } catch {
            c = AppCore.inMemory()
            startupError = describe(error)
        }
        core = c
        settings = c.settings()
        refreshAll()
        if let e = startupError {
            alert = AlertInfo(
                title: "Couldn't open the data",
                message: e + "\n\nThe app is running in memory only: nothing will be saved."
            )
        }
    }

    // MARK: - Refreshing

    var activeSession: SessionInfo? { sessions.first { $0.id == activeSessionId } }

    func refreshAll() {
        settings = core.settings()
        folders = core.folders()
        let all = core.sessions(includeZombies: true)
        sessions = all.filter { !$0.isZombie }
        zombies = all.filter { $0.isZombie }
        activeSessionId = core.activeSession()
        refreshTabs()
        refreshLibrary()
    }

    func refreshTabs() {
        tabs = core.tabs(session: activeSessionId)
        activeTabId = core.activeTab(session: activeSessionId) ?? tabs.first?.id
        refreshTab()
    }

    func refreshTab() {
        guard let id = activeTabId, let t = try? core.tab(tab: id) else {
            tab = nil
            plot = nil
            return
        }
        tab = t
        plot = try? core.plot(tab: id, points: Self.plotPoints)
        tabs = core.tabs(session: activeSessionId)
    }

    func refreshLibrary() {
        presets = showAllPresets ? core.allPresets(session: activeSessionId) : core.presets(session: activeSessionId)
        history = core.history(session: activeSessionId)
        inbox = core.inbox(session: activeSessionId)
    }

    enum Scope { case tab, library, tabAndLibrary, sessions, all }

    private func refresh(_ scope: Scope) {
        switch scope {
        case .tab: refreshTab()
        case .library: refreshLibrary()
        case .tabAndLibrary: refreshTab(); refreshLibrary()
        case .sessions:
            folders = core.folders()
            let all = core.sessions(includeZombies: true)
            sessions = all.filter { !$0.isZombie }
            zombies = all.filter { $0.isZombie }
        case .all: refreshAll()
        }
    }

    /// Runs a core action, shows any error, refreshes and schedules a save.
    @discardableResult
    func act<T>(_ scope: Scope = .tab, _ body: () throws -> T) -> T? {
        defer {
            refresh(scope)
            scheduleSave()
        }
        do {
            return try body()
        } catch {
            show(error)
            return nil
        }
    }

    func show(_ error: Error) {
        alert = AlertInfo(title: "Oops", message: describe(error))
    }

    func scheduleSave() {
        saveTask?.cancel()
        saveTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 700_000_000)
            guard !Task.isCancelled else { return }
            self?.saveNow()
        }
    }

    func saveNow() {
        do { try core.save() } catch { show(error) }
    }

    // MARK: - Settings and theme

    func updateSettings(_ s: AppSettings) {
        act(.all) { try core.setSettings(settings: s) }
        applyTheme()
    }

    func setTheme(_ t: Theme) {
        var s = settings
        s.theme = t
        updateSettings(s)
    }

    func applyTheme() {
        switch settings.theme {
        case .system: NSApplication.shared.appearance = nil
        case .light: NSApplication.shared.appearance = NSAppearance(named: .aqua)
        case .dark: NSApplication.shared.appearance = NSAppearance(named: .darkAqua)
        }
    }

    // MARK: - Folders and sessions

    func selectSession(_ id: String) {
        guard id != activeSessionId else { return }
        act(.all) { try core.setActiveSession(session: id) }
    }

    func newSession(folder: String?) {
        if let id = act(.all, { try core.createSession(name: "", folder: folder) }) {
            act(.all) { try core.setActiveSession(session: id) }
            renameSession(id)
        }
    }

    func renameSession(_ id: String) {
        let current = (sessions + zombies).first { $0.id == id }?.name ?? ""
        prompt = TextPrompt(title: "Rename session", text: current) { [weak self] name in
            self?.act(.all) { try self?.core.renameSession(session: id, name: name) }
        }
    }

    func moveSession(_ id: String, to folder: String?) {
        act(.sessions) { try core.moveSession(session: id, folder: folder) }
    }

    func deleteSession(_ s: SessionInfo) {
        alert = AlertInfo(
            title: "Delete \"\(s.name)\"?",
            message: "Its tabs and history will be erased. The session will stay under \"Deleted sessions\" so its links (presets, inbox items) can be moved to another session.",
            actionLabel: "Delete",
            destructive: true
        ) { [weak self] in
            self?.act(.all) { try self?.core.deleteSession(session: s.id) }
        }
    }

    func rebindZombie(_ zombie: SessionInfo, to target: SessionInfo) {
        if let n = act(.all, { try core.rebindZombie(zombie: zombie.id, target: target.id) }) {
            alert = AlertInfo(title: "Links moved", message: "\(n) item(s) from \"\(zombie.name)\" now point to \"\(target.name)\".")
        }
    }

    func purgeZombie(_ zombie: SessionInfo) {
        alert = AlertInfo(
            title: "Permanently erase \"\(zombie.name)\"?",
            message: "Its \(zombie.bindings) link(s) will be removed from presets and inbox items.",
            actionLabel: "Erase",
            destructive: true
        ) { [weak self] in
            self?.act(.all) { try self?.core.purgeZombie(zombie: zombie.id) }
        }
    }

    func newFolder(parent: String?) {
        prompt = TextPrompt(title: "New folder", text: "", confirmLabel: "Create") { [weak self] name in
            self?.act(.sessions) { try self?.core.createFolder(name: name, parent: parent) }
        }
    }

    func renameFolder(_ f: FolderInfo) {
        prompt = TextPrompt(title: "Rename folder", text: f.name) { [weak self] name in
            self?.act(.sessions) { try self?.core.renameFolder(folder: f.id, name: name) }
        }
    }

    func moveFolder(_ f: FolderInfo, to parent: String?) {
        act(.sessions) { try core.moveFolder(folder: f.id, parent: parent) }
    }

    func deleteFolder(_ f: FolderInfo) {
        act(.sessions) { try core.deleteFolder(folder: f.id) }
    }

    func saveSessionDefaults(_ session: String, bands: UInt32?, steps: UInt32?, mode: Mode?) {
        act(.sessions) { try core.setSessionDefaults(session: session, bands: bands, steps: steps, mode: mode) }
    }

    // MARK: - Tabs

    @discardableResult
    func ensureTab() -> String? {
        if let id = activeTabId { return id }
        newTab()
        return activeTabId
    }

    func newTab() {
        act(.all) { _ = try core.createTab(session: activeSessionId, name: nil) }
    }

    func selectTab(_ id: String) {
        guard id != activeTabId else { return }
        act(.all) { try core.setActiveTab(tab: id) }
    }

    func closeTab(_ id: String) {
        act(.all) { try core.closeTab(tab: id) }
    }

    func closeActiveTab() {
        if let id = activeTabId { closeTab(id) }
    }

    func duplicateTab(_ id: String) {
        act(.all) { _ = try core.duplicateTab(tab: id) }
    }

    func renameTab(_ id: String) {
        let current = tabs.first { $0.id == id }?.name ?? ""
        prompt = TextPrompt(title: "Rename tab", text: current) { [weak self] name in
            self?.act(.all) { try self?.core.renameTab(tab: id, name: name) }
        }
    }

    func moveTab(_ id: String, by delta: Int) {
        guard let i = tabs.firstIndex(where: { $0.id == id }) else { return }
        let j = max(0, min(tabs.count - 1, i + delta))
        act(.all) { try core.moveTab(tab: id, index: UInt32(j)) }
    }

    func setTabParams(bands: UInt32? = nil, steps: UInt32? = nil, mode: Mode? = nil) {
        guard let t = tab else { return }
        act { try core.setTabParams(tab: t.id, bands: bands ?? t.bands, steps: steps ?? t.steps, mode: mode ?? t.mode) }
    }

    func undo() {
        guard let id = activeTabId else { return }
        act { _ = try core.undo(tab: id) }
    }

    func redo() {
        guard let id = activeTabId else { return }
        act { _ = try core.redo(tab: id) }
    }

    // MARK: - Curves

    func openFile(role: Role) {
        let title: String
        switch role {
        case .measurement: title = "Open measurement"
        case .target: title = "Open target"
        case .compensation: title = "Open compensation"
        case .compare: title = "Add comparison curves"
        }
        let urls = FilePanels.openFiles(title: title, multiple: role == .compare)
        guard !urls.isEmpty, let tabId = ensureTab() else { return }
        for url in urls {
            act(.tabAndLibrary) { _ = try core.addCurveFromFile(tab: tabId, path: url.path, role: role) }
        }
    }

    func openHistory(_ h: HistoryInfo, role: Role) {
        guard h.exists else {
            alert = AlertInfo(
                title: "File not found",
                message: "\"\(h.fileName)\" no longer exists at this location:\n\(h.path)",
                actionLabel: "Remove from history",
                destructive: true
            ) { [weak self] in
                guard let self else { return }
                self.act(.library) { try self.core.removeHistory(session: self.activeSessionId, path: h.path) }
            }
            return
        }
        guard let tabId = ensureTab() else { return }
        act(.tabAndLibrary) { _ = try core.addCurveFromFile(tab: tabId, path: h.path, role: role) }
    }

    func pasteCurve(role: Role) {
        guard let text = FilePanels.pasteboardText(), let tabId = ensureTab() else {
            alert = AlertInfo(title: "Clipboard is empty", message: "Copy a frequency/dB CSV first.")
            return
        }
        act { _ = try core.addCurveFromText(tab: tabId, label: "Pasted curve", text: text, role: role) }
    }

    func removeCurve(_ c: CurveInfo) {
        guard let t = tab else { return }
        act { try core.removeCurve(tab: t.id, curve: c.id) }
    }

    func setRole(_ c: CurveInfo, _ role: Role) {
        guard let t = tab else { return }
        act { try core.setCurveRole(tab: t.id, curve: c.id, role: role) }
    }

    /// `key`: curve id, "result" or "eq".
    func setVisible(_ key: String, _ visible: Bool) {
        guard let t = tab else { return }
        act { try core.setCurveVisible(tab: t.id, curve: key, visible: visible) }
    }

    func setColor(_ key: String, _ color: Color) {
        guard let t = tab else { return }
        act { try core.setCurveColor(tab: t.id, curve: key, color: color.hexString) }
    }

    func randomColor(_ key: String) {
        guard let t = tab else { return }
        act { _ = try core.randomizeCurveColor(tab: t.id, curve: key) }
    }

    func renameCurve(_ c: CurveInfo) {
        guard let t = tab else { return }
        prompt = TextPrompt(title: "Rename curve", text: c.label) { [weak self] name in
            self?.act { try self?.core.renameCurve(tab: t.id, curve: c.id, label: name) }
        }
    }

    func setOffset(_ c: CurveInfo, _ v: Double) {
        guard let t = tab else { return }
        act { try core.setCurveOffset(tab: t.id, curve: c.id, offsetDb: v) }
    }

    func alignToTarget(_ c: CurveInfo) {
        guard let t = tab else { return }
        act { _ = try core.alignCurveToTarget(tab: t.id, curve: c.id) }
    }

    func snapshotResult() {
        guard let t = tab else { return }
        act { _ = try core.snapshotResult(tab: t.id) }
    }

    func label(for what: CurveRef) -> String {
        guard let t = tab else { return "" }
        switch what {
        case .curve(let id): return t.curves.first { $0.id == id }?.label ?? "Curve"
        case .result:
            let m = t.curves.first { $0.id == t.measurement }?.label ?? "Measurement"
            return "\(m) + EQ"
        case .eq: return "EQ — \(t.name)"
        }
    }

    func exportCurveCSV(_ what: CurveRef) {
        guard let t = tab, let csv = act(.tab, { try core.exportCurveCsv(tab: t.id, what: what) }) else { return }
        _ = FilePanels.save(text: csv, suggestedName: "\(label(for: what)).csv")
    }

    // MARK: - Filters

    func addManualFilter() {
        guard let id = ensureTab() else { return }
        act { _ = try core.addFilter(tab: id, kind: .peak, f0: 1000, gain: 0, q: 1.0) }
    }

    func updateFilter(_ f: FilterInfo, kind: FilterKind? = nil, f0: Double? = nil, gain: Double? = nil, q: Double? = nil, enabled: Bool? = nil) {
        guard let t = tab else { return }
        act {
            try core.updateFilter(
                tab: t.id, filter: f.id, kind: kind ?? f.kind, f0: f0 ?? f.f0,
                gain: gain ?? f.gain, q: q ?? f.q, enabled: enabled ?? f.enabled
            )
        }
    }

    func removeFilter(_ f: FilterInfo) {
        guard let t = tab else { return }
        act { try core.removeFilter(tab: t.id, filter: f.id) }
    }

    func setOrigin(_ f: FilterInfo, _ o: FilterOrigin) {
        guard let t = tab else { return }
        act { try core.setFilterOrigin(tab: t.id, filter: f.id, origin: o) }
    }

    func clearFilters(onlyAuto: Bool) {
        guard let t = tab else { return }
        act { try core.clearFilters(tab: t.id, onlyAuto: onlyAuto) }
    }

    private func report(_ r: ImportReport) {
        if !r.skipped.isEmpty {
            alert = AlertInfo(
                title: "\(r.count) filter(s) imported",
                message: "Skipped lines:\n" + r.skipped.prefix(12).joined(separator: "\n")
            )
        }
    }

    func importFiltersFromFile(replace: Bool) {
        guard let url = FilePanels.openFile(title: "Import filters"), let id = ensureTab() else { return }
        if let r = act(.tab, { try core.importFiltersFile(tab: id, path: url.path, replace: replace) }) { report(r) }
    }

    func importFiltersFromClipboard(replace: Bool) {
        guard let text = FilePanels.pasteboardText(), let id = ensureTab() else { return }
        if let r = act(.tab, { try core.importFiltersText(tab: id, text: text, replace: replace) }) { report(r) }
    }

    func exportEq(_ format: ExportFormat, toClipboard: Bool = false) {
        guard let t = tab, let text = act(.tab, { try core.exportEq(tab: t.id, format: format) }) else { return }
        if toClipboard {
            FilePanels.copy(text)
        } else {
            _ = FilePanels.save(text: text, suggestedName: "\(t.name) \(format.fileSuffix)")
        }
    }

    // MARK: - AutoEQ

    var canRun: Bool { !isRunning && tab?.measurement != nil && tab?.target != nil }

    func runAutoeq() {
        guard canRun, let id = activeTabId else { return }
        isRunning = true
        let core = self.core
        Task.detached(priority: .userInitiated) {
            let result = Result<RunInfo, Error> { try core.runAutoeq(tab: id) }
            await MainActor.run {
                self.isRunning = false
                if case .failure(let e) = result { self.show(e) }
                self.refreshTab()
                self.scheduleSave()
            }
        }
    }

    // MARK: - Presets

    func addPreset(_ p: PresetInfo, role: Role?) {
        guard p.available else {
            alert = AlertInfo(
                title: "File not found",
                message: "The file for preset \"\(p.name)\" no longer exists:\n\(p.path ?? "")\n\nUpdate the preset from another file.",
                actionLabel: "Choose file…"
            ) { [weak self] in self?.updatePresetFromFile(p) }
            return
        }
        guard let id = ensureTab() else { return }
        act(.tabAndLibrary) { _ = try core.addCurveFromPreset(tab: id, preset: p.id, role: role) }
    }

    func newPresetFromFile() {
        guard let url = FilePanels.openFile(title: "New preset from file") else { return }
        guard let dups = act(.library, { try core.findDuplicatesForFile(session: activeSessionId, path: url.path) }) else { return }
        let lower = url.lastPathComponent.lowercased()
        let guess: PresetCategory =
            lower.contains("comp") ? .compensation
            : (lower.contains("target") || lower.contains("harman") || lower.contains("cible") ? .target : .measurement)
        presetDraft = PresetDraft(
            source: .file(path: url.path),
            name: url.deletingPathExtension().lastPathComponent,
            category: guess,
            embed: false,
            duplicates: dups
        )
    }

    func newPresetFromCurve(_ what: CurveRef) {
        guard let t = tab else { return }
        let dups = act(.tab, { try core.findDuplicatesForCurve(session: activeSessionId, tab: t.id, what: what) }) ?? []
        var cat: PresetCategory = .measurement
        if case .curve(let id) = what {
            if id == t.target { cat = .target } else if id == t.compensation { cat = .compensation }
        }
        presetDraft = PresetDraft(source: .curve(tab: t.id, what: what), name: label(for: what), category: cat, embed: true, duplicates: dups)
    }

    func commitPresetDraft(_ d: PresetDraft) {
        let vis: Visibility
        switch d.visibility {
        case .appWide: vis = Visibility(mode: .allExcept, sessions: [])
        case .thisSession: vis = Visibility(mode: .onlyListed, sessions: [activeSessionId])
        }
        switch d.source {
        case .file(let path):
            act(.library) { _ = try core.createPresetFromFile(path: path, name: d.name, category: d.category, embed: d.embed, visibility: vis) }
        case .curve(let tab, let what):
            act(.library) { _ = try core.createPresetFromCurve(tab: tab, what: what, name: d.name, category: d.category, visibility: vis) }
        }
        libraryPane = .presets
    }

    func grantHere(_ presetId: String) {
        act(.library) { try core.grantPresetToSession(preset: presetId, session: activeSessionId) }
    }

    func togglePin(_ p: PresetInfo) {
        act(.library) { try core.setPresetPinned(preset: p.id, session: activeSessionId, pinned: !p.pinned) }
    }

    func renamePreset(_ p: PresetInfo) {
        prompt = TextPrompt(title: "Rename preset", text: p.name) { [weak self] name in
            self?.act(.library) { try self?.core.renamePreset(preset: p.id, name: name) }
        }
    }

    func updatePresetFromFile(_ p: PresetInfo) {
        guard let url = FilePanels.openFile(title: "Update \"\(p.name)\" from file") else { return }
        act(.library) { try core.updatePresetFromFile(preset: p.id, path: url.path) }
    }

    func embedPreset(_ p: PresetInfo) {
        act(.library) { try core.embedPreset(preset: p.id) }
    }

    func setCategory(_ p: PresetInfo, _ c: PresetCategory) {
        act(.library) { try core.setPresetCategory(preset: p.id, category: c) }
    }

    func saveVisibility(_ p: PresetInfo, _ v: Visibility) {
        act(.library) { try core.setPresetVisibility(preset: p.id, visibility: v) }
    }

    func exportPresetCSV(_ p: PresetInfo) {
        guard let csv = act(.library, { try core.exportPresetCsv(preset: p.id) }) else { return }
        _ = FilePanels.save(text: csv, suggestedName: "\(p.name).csv")
    }

    func deletePreset(_ p: PresetInfo) {
        alert = AlertInfo(
            title: "Delete preset \"\(p.name)\"?",
            message: "It will disappear from all sessions. Curves already added to tabs will stay.",
            actionLabel: "Delete",
            destructive: true
        ) { [weak self] in
            self?.act(.library) { try self?.core.deletePreset(preset: p.id) }
        }
    }

    func toggleShowAllPresets() {
        showAllPresets.toggle()
        refreshLibrary()
    }

    // MARK: - History

    func removeHistory(_ h: HistoryInfo) {
        act(.library) { try core.removeHistory(session: activeSessionId, path: h.path) }
    }

    func clearHistory(onlyMissing: Bool) {
        act(.library) { try core.clearHistory(session: activeSessionId, onlyMissing: onlyMissing) }
    }

    func presetFromHistory(_ h: HistoryInfo) {
        guard h.exists else { openHistory(h, role: .compare); return }
        guard let dups = act(.library, { try core.findDuplicatesForFile(session: activeSessionId, path: h.path) }) else { return }
        presetDraft = PresetDraft(
            source: .file(path: h.path),
            name: (h.fileName as NSString).deletingPathExtension,
            category: .measurement,
            embed: false,
            duplicates: dups
        )
    }

    // MARK: - Inbox

    func prepareSend(_ what: CurveRef) {
        guard let t = tab else { return }
        inboxDraft = InboxDraft(tab: t.id, what: what, label: label(for: what))
    }

    func commitSend(_ d: InboxDraft) {
        act(.library) { _ = try core.sendToInbox(tab: d.tab, what: d.what, note: d.note, toSession: d.toSession) }
        libraryPane = .inbox
    }

    func takeInbox(_ i: InboxInfo, role: Role) {
        guard let id = ensureTab() else { return }
        act(.tabAndLibrary) { _ = try core.addCurveFromInbox(tab: id, item: i.id, role: role) }
    }

    func applyInboxFilters(_ i: InboxInfo, replace: Bool) {
        guard let id = ensureTab() else { return }
        act { _ = try core.applyInboxFilters(tab: id, item: i.id, replace: replace) }
    }

    func editInboxNote(_ i: InboxInfo) {
        prompt = TextPrompt(title: "Item note", text: i.note) { [weak self] note in
            self?.act(.library) { try self?.core.setInboxNote(item: i.id, note: note) }
        }
    }

    func exportInboxCSV(_ i: InboxInfo) {
        guard let csv = act(.library, { try core.exportInboxCsv(item: i.id) }) else { return }
        _ = FilePanels.save(text: csv, suggestedName: "\(i.note.isEmpty ? i.label : i.note).csv")
    }

    func deleteInbox(_ i: InboxInfo) {
        act(.library) { try core.deleteInboxItem(item: i.id) }
    }
}
