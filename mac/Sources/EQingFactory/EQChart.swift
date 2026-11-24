import SwiftUI
import Charts

/// Frequency (log, 20 Hz – 20 kHz) × gain grid, with all of the tab's curves.
struct EQChart: View {
    let plot: PlotData
    let inspect: Bool

    static let ticks: [Double] = [20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000]

    private var yStep: Double {
        let span = plot.yMax - plot.yMin
        return span > 80 ? 10 : 5
    }

    private func style(_ kind: SeriesKind) -> StrokeStyle {
        switch kind {
        case .target: return StrokeStyle(lineWidth: 2.2)
        case .result: return StrokeStyle(lineWidth: 2)
        case .eq: return StrokeStyle(lineWidth: 1.4, dash: [5, 3])
        case .measurement: return StrokeStyle(lineWidth: 1.6)
        case .compensation: return StrokeStyle(lineWidth: 1.3, dash: [2, 3])
        case .compare: return StrokeStyle(lineWidth: 1.3)
        }
    }

    var body: some View {
        Chart {
            ForEach(plot.series) { s in
                ForEach(s.values.indices, id: \.self) { i in
                    LineMark(
                        x: .value("Frequency", plot.freqs[i]),
                        y: .value("Gain", s.values[i]),
                        series: .value("Curve", s.key)
                    )
                    .foregroundStyle(Color(hex: s.color))
                    .lineStyle(style(s.kind))
                }
            }
        }
        .chartXScale(domain: 20.0...20000.0, type: .log)
        .chartYScale(domain: plot.yMin...plot.yMax)
        .chartXAxis {
            AxisMarks(values: Self.ticks) { value in
                AxisGridLine()
                AxisTick()
                AxisValueLabel {
                    if let f = value.as(Double.self) { Text(formatHz(f)) }
                }
            }
        }
        .chartYAxis {
            AxisMarks(position: .leading, values: .stride(by: yStep)) { value in
                AxisGridLine()
                AxisValueLabel {
                    if let d = value.as(Double.self) { Text("\(Int(d.rounded()))") }
                }
            }
        }
        .chartLegend(.hidden)
        .chartOverlay { proxy in
            GeometryReader { geo in
                InspectionOverlay(proxy: proxy, geo: geo, plot: plot, enabled: inspect)
            }
        }
    }
}

/// Inspect mode: a vertical line that follows the pointer, with a point and value for each curve.
/// The hover state lives here so the whole chart isn't redrawn on every move.
struct InspectionOverlay: View {
    let proxy: ChartProxy
    let geo: GeometryProxy
    let plot: PlotData
    let enabled: Bool
    @State private var hover: CGPoint?

    /// Value of a series at frequency f (interpolated on the log grid).
    private func value(_ s: PlotSeries, at f: Double) -> Double? {
        let n = plot.freqs.count
        guard n > 1, s.values.count == n, let first = plot.freqs.first, let last = plot.freqs.last else { return nil }
        let t = (log(f) - log(first)) / (log(last) - log(first)) * Double(n - 1)
        let i = max(0, min(n - 2, Int(t.rounded(.down))))
        let w = max(0, min(1, t - Double(i)))
        return s.values[i] * (1 - w) + s.values[i + 1] * w
    }

    var body: some View {
        let frame: CGRect = proxy.plotFrame.map { geo[$0] } ?? .zero
        ZStack(alignment: .topLeading) {
            Rectangle()
                .fill(Color.clear)
                .contentShape(Rectangle())
                .onContinuousHover { phase in
                    switch phase {
                    case .active(let p): hover = p
                    case .ended: hover = nil
                    }
                }

            if enabled, let p = hover, frame.contains(p),
               let f = proxy.value(atX: p.x - frame.minX, as: Double.self) {
                Path { path in
                    path.move(to: CGPoint(x: p.x, y: frame.minY))
                    path.addLine(to: CGPoint(x: p.x, y: frame.maxY))
                }
                .stroke(Color.secondary, style: StrokeStyle(lineWidth: 1, dash: [3, 3]))
                .allowsHitTesting(false)

                ForEach(plot.series) { s in
                    if let v = value(s, at: f), let y = proxy.position(forY: v) {
                        Circle()
                            .fill(Color(hex: s.color))
                            .frame(width: 7, height: 7)
                            .position(x: p.x, y: frame.minY + y)
                            .allowsHitTesting(false)
                    }
                }

                InspectionTooltip(freq: f, rows: plot.series.compactMap { s in
                    value(s, at: f).map { (s.label, s.color, $0) }
                })
                .padding(8)
                .frame(
                    width: frame.width,
                    height: frame.height,
                    alignment: p.x > frame.midX ? .topLeading : .topTrailing
                )
                .offset(x: frame.minX, y: frame.minY)
                .allowsHitTesting(false)
            }
        }
    }
}

struct InspectionTooltip: View {
    let freq: Double
    let rows: [(String, String, Double)]

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(freq >= 1000 ? String(format: "%.2f kHz", freq / 1000) : String(format: "%.1f Hz", freq))
                .font(.caption.weight(.semibold))
                .monospacedDigit()
            ForEach(rows.indices, id: \.self) { i in
                HStack(spacing: 6) {
                    Circle().fill(Color(hex: rows[i].1)).frame(width: 7, height: 7)
                    Text(rows[i].0).lineLimit(1)
                    Spacer(minLength: 8)
                    Text(String(format: "%+.2f dB", rows[i].2)).monospacedDigit()
                }
                .font(.caption)
            }
        }
        .padding(8)
        .frame(maxWidth: 280)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(Color.secondary.opacity(0.25)))
    }
}
