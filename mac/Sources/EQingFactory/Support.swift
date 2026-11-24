import SwiftUI
import AppKit
import UniformTypeIdentifiers

// MARK: - SwiftUI conformances (uniFFI-generated types)

extension FolderInfo: Identifiable {}
extension SessionInfo: Identifiable {}
extension TabSummary: Identifiable {}
extension CurveInfo: Identifiable {}
extension FilterInfo: Identifiable {}
extension PresetInfo: Identifiable {}
extension InboxInfo: Identifiable {}
extension HistoryInfo: Identifiable { public var id: String { path } }
extension PlotSeries: Identifiable { public var id: String { key } }
extension DuplicateInfo: Identifiable { public var id: String { presetId } }

// MARK: - Colors

extension Color {
    /// Color from "#RRGGBB" (gray if invalid).
    init(hex: String) {
        let t = hex.trimmingCharacters(in: .whitespaces).replacingOccurrences(of: "#", with: "")
        guard t.count == 6, let v = UInt64(t, radix: 16) else {
            self = .gray
            return
        }
        self = Color(
            .sRGB,
            red: Double((v >> 16) & 0xFF) / 255,
            green: Double((v >> 8) & 0xFF) / 255,
            blue: Double(v & 0xFF) / 255,
            opacity: 1
        )
    }

    var hexString: String {
        let ns = NSColor(self).usingColorSpace(.sRGB) ?? NSColor.gray
        let r = Int((ns.redComponent * 255).rounded())
        let g = Int((ns.greenComponent * 255).rounded())
        let b = Int((ns.blueComponent * 255).rounded())
        return String(format: "#%02X%02X%02X", r, g, b)
    }
}

// MARK: - Labels

extension Role {
    var label: String {
        switch self {
        case .measurement: return "Measurement"
        case .target: return "Target"
        case .compensation: return "Compensation"
        case .compare: return "Comparison"
        }
    }
}

extension Mode {
    var label: String {
        switch self {
        case .inEar: return "i — In-ear (default)"
        case .overEar: return "o — Over-ear"
        case .precise: return "x — Precise, no smoothing"
        }
    }
    var letter: String {
        switch self {
        case .inEar: return "i"
        case .overEar: return "o"
        case .precise: return "x"
        }
    }
}

extension FilterKind {
    var label: String {
        switch self {
        case .peak: return "PK"
        case .lowShelf: return "LSC"
        case .highShelf: return "HSC"
        }
    }
}

extension FilterOrigin {
    var label: String {
        switch self {
        case .auto: return "Auto"
        case .manual: return "Manual"
        case .imported: return "Imported"
        }
    }
    var color: Color {
        switch self {
        case .auto: return .blue
        case .manual: return .green
        case .imported: return .purple
        }
    }
}

extension PresetCategory {
    var label: String {
        switch self {
        case .measurement: return "Measurement"
        case .target: return "Target"
        case .compensation: return "Compensation"
        }
    }
    var symbol: String {
        switch self {
        case .measurement: return "headphones"
        case .target: return "scope"
        case .compensation: return "waveform.path.badge.minus"
        }
    }
}

extension Theme {
    var label: String {
        switch self {
        case .system: return "System"
        case .light: return "Light"
        case .dark: return "Dark"
        }
    }
}

extension ExportFormat {
    var label: String {
        switch self {
        case .parametric: return "Parametric (Equalizer APO)"
        case .graphicEq: return "GraphicEQ (Wavelet)"
        case .filtersCsv: return "Filters as CSV"
        }
    }
    var fileSuffix: String {
        switch self {
        case .parametric: return "ParametricEQ.txt"
        case .graphicEq: return "GraphicEQ.txt"
        case .filtersCsv: return "filters.csv"
        }
    }
}

func formatHz(_ f: Double) -> String {
    if f >= 1000 {
        let k = f / 1000
        return k == k.rounded() ? "\(Int(k))k" : String(format: "%.1fk", k)
    }
    return "\(Int(f.rounded()))"
}

func formatDate(_ ms: Int64) -> String {
    let d = Date(timeIntervalSince1970: Double(ms) / 1000)
    return d.formatted(.relative(presentation: .named))
}

/// Readable message for a Rust core error.
func describe(_ error: Error) -> String {
    if let e = error as? CoreError {
        switch e {
        case .NotFound(let what): return "Not found: \(what)."
        case .FileMissing(let path): return "The file no longer exists at this location:\n\(path)"
        case .Io(let detail): return detail
        case .Parse(let detail): return detail
        case .Invalid(let detail): return detail
        case .Engine(let detail): return "AutoEQ: \(detail)"
        }
    }
    return error.localizedDescription
}

// MARK: - File panels

enum FilePanels {
    static func openFiles(title: String, multiple: Bool = false) -> [URL] {
        let p = NSOpenPanel()
        p.title = title
        p.message = title
        p.canChooseDirectories = false
        p.canChooseFiles = true
        p.allowsMultipleSelection = multiple
        return p.runModal() == .OK ? p.urls : []
    }

    static func openFile(title: String) -> URL? {
        openFiles(title: title).first
    }

    static func openApplication(title: String) -> URL? {
        let p = NSOpenPanel()
        p.title = title
        p.canChooseDirectories = false
        p.treatsFilePackagesAsDirectories = true
        return p.runModal() == .OK ? p.url : nil
    }

    static func save(text: String, suggestedName: String) -> Bool {
        let p = NSSavePanel()
        p.nameFieldStringValue = suggestedName.replacingOccurrences(of: "/", with: "-")
        p.canCreateDirectories = true
        guard p.runModal() == .OK, let url = p.url else { return false }
        do {
            try text.write(to: url, atomically: true, encoding: .utf8)
            return true
        } catch {
            NSAlert(error: error).runModal()
            return false
        }
    }

    static func copy(_ text: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }

    static func pasteboardText() -> String? {
        NSPasteboard.general.string(forType: .string)
    }
}

// MARK: - Small reusable views

struct Badge: View {
    let text: String
    let color: Color
    var body: some View {
        Text(text)
            .font(.caption2.weight(.semibold))
            .padding(.horizontal, 5)
            .padding(.vertical, 1)
            .background(color.opacity(0.18), in: Capsule())
            .foregroundStyle(color)
    }
}

/// Numeric field that commits on Return or when it loses focus.
struct NumberField: View {
    let value: Double
    let fraction: Int
    var width: CGFloat = 60
    let onCommit: (Double) -> Void

    var body: some View {
        TextField(
            "",
            value: Binding(get: { value }, set: { if $0 != value { onCommit($0) } }),
            format: .number.precision(.fractionLength(fraction))
        )
        .textFieldStyle(.roundedBorder)
        .multilineTextAlignment(.trailing)
        .monospacedDigit()
        .frame(width: width)
    }
}

/// Centered empty state for a list: icon, title and a wrapping explanation.
struct EmptyHint: View {
    let symbol: String
    let title: String
    let message: String

    var body: some View {
        VStack(spacing: 8) {
            Image(systemName: symbol)
                .font(.system(size: 28, weight: .light))
                .foregroundStyle(.tertiary)
            Text(title)
                .font(.headline)
                .foregroundStyle(.secondary)
            Text(message)
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// Footer bar pinned under a list, separated by a hairline and drawn on the bar material.
struct BottomBar<Content: View>: View {
    @ViewBuilder var content: Content

    var body: some View {
        VStack(spacing: 0) {
            Divider()
            HStack(spacing: 12) { content }
                .buttonStyle(.borderless)
                .font(.callout)
                .padding(.horizontal, 10)
                .padding(.vertical, 7)
        }
        .background(.bar)
    }
}
