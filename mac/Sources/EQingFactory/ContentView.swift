import SwiftUI

struct ContentView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var model = model
        NavigationSplitView {
            SessionSidebar()
                .navigationSplitViewColumnWidth(min: 210, ideal: 240, max: 340)
        } content: {
            LibraryView()
                .navigationSplitViewColumnWidth(min: 280, ideal: 310, max: 440)
        } detail: {
            WorkspaceView()
        }
        .sheet(item: $model.prompt) { p in TextPromptSheet(prompt: p).environment(model) }
        .sheet(item: $model.presetDraft) { d in PresetDraftSheet(draft: d).environment(model) }
        .sheet(item: $model.visibilityEdit) { p in VisibilitySheet(preset: p).environment(model) }
        .sheet(item: $model.inboxDraft) { d in InboxSendSheet(draft: d).environment(model) }
        .sheet(item: $model.sessionDefaultsEdit) { s in SessionDefaultsSheet(session: s).environment(model) }
        .alert(
            model.alert?.title ?? "",
            isPresented: Binding(get: { model.alert != nil }, set: { if !$0 { model.alert = nil } }),
            presenting: model.alert
        ) { info in
            if let label = info.actionLabel, let action = info.action {
                Button(label, role: info.destructive ? .destructive : nil) { action() }
                Button("Cancel", role: .cancel) {}
            } else {
                Button("OK", role: .cancel) {}
            }
        } message: { info in
            Text(info.message)
        }
    }
}

// MARK: - Sessions and folders

struct SessionSidebar: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        List(selection: Binding<String?>(
            get: { model.activeSessionId },
            set: { if let id = $0 { model.selectSession(id) } }
        )) {
            Section("Sessions") {
                FolderContents(parent: nil)
            }
            if !model.zombies.isEmpty {
                Section("Deleted sessions") {
                    ForEach(model.zombies) { z in ZombieRow(zombie: z) }
                }
            }
        }
        .listStyle(.sidebar)
        .safeAreaInset(edge: .bottom, spacing: 0) {
            BottomBar {
                Button { model.newSession(folder: nil) } label: {
                    Label("Session", systemImage: "plus")
                }
                Button { model.newFolder(parent: nil) } label: {
                    Label("Folder", systemImage: "folder.badge.plus")
                }
                Spacer()
            }
        }
    }
}

/// Contents of a folder (or of the root): subfolders, then sessions.
struct FolderContents: View {
    let parent: String?
    @Environment(AppModel.self) private var model

    var body: some View {
        ForEach(model.folders.filter { $0.parent == parent }) { f in
            FolderNode(folder: f)
        }
        ForEach(model.sessions.filter { $0.folder == parent }) { s in
            SessionRow(session: s)
                .tag(s.id as String?)
        }
    }
}

struct FolderNode: View {
    let folder: FolderInfo
    @Environment(AppModel.self) private var model
    @State private var expanded = true

    var body: some View {
        DisclosureGroup(isExpanded: $expanded) {
            FolderContents(parent: folder.id)
        } label: {
            Label(folder.name, systemImage: "folder")
                .contextMenu {
                    Button("New session here") { model.newSession(folder: folder.id) }
                    Button("New subfolder") { model.newFolder(parent: folder.id) }
                    Divider()
                    Button("Rename…") { model.renameFolder(folder) }
                    Menu("Move to") {
                        Button("Top level") { model.moveFolder(folder, to: nil) }
                        ForEach(model.folders.filter { $0.id != folder.id }) { f in
                            Button(f.name) { model.moveFolder(folder, to: f.id) }
                        }
                    }
                    Divider()
                    Button("Delete folder (keep contents)", role: .destructive) { model.deleteFolder(folder) }
                }
        }
    }
}

struct SessionRow: View {
    let session: SessionInfo
    @Environment(AppModel.self) private var model

    var body: some View {
        HStack {
            Label(session.name, systemImage: "person.crop.circle")
            Spacer()
            if session.tabCount > 1 {
                Text("\(session.tabCount)")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .contextMenu {
            Button("Rename…") { model.renameSession(session.id) }
            Button("Session settings…") { model.sessionDefaultsEdit = session }
            Menu("Move to") {
                Button("Top level") { model.moveSession(session.id, to: nil) }
                ForEach(model.folders) { f in
                    Button(f.name) { model.moveSession(session.id, to: f.id) }
                }
            }
            Divider()
            Button("Delete…", role: .destructive) { model.deleteSession(session) }
                .disabled(model.sessions.count <= 1)
        }
    }
}

struct ZombieRow: View {
    let zombie: SessionInfo
    @Environment(AppModel.self) private var model

    var body: some View {
        HStack {
            Label(zombie.name, systemImage: "person.crop.circle.badge.xmark")
                .foregroundStyle(.secondary)
            Spacer()
            Text("\(zombie.bindings) link\(zombie.bindings == 1 ? "" : "s")")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .contextMenu {
            Menu("Move its links to") {
                ForEach(model.sessions) { s in
                    Button(s.name) { model.rebindZombie(zombie, to: s) }
                }
            }
            Button("Delete permanently", role: .destructive) { model.purgeZombie(zombie) }
        }
        .help("Deleted session: right-click to move its links (presets, inbox items) to another session.")
    }
}
