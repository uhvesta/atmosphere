import HumataDaemonClient
import SwiftUI

/// The board as its plan: a column per step, each item in the first column
/// after everything it waits on, with an arrow from each item to what waits
/// on it. A milestone's tasks wait on what it does, and blocked items say so.
struct BoardGraphView: View {
    let open: (Item.ID) -> Void
    private let steps: [[Item]]
    private let edges: [Edge]

    private struct Edge: Hashable {
        var from: Item.ID
        var to: Item.ID
    }

    /// `items` in the plan's order, so whatever an item waits on comes
    /// before it. The steps are worked out here, once per read of the
    /// board, not as the view draws.
    init(items: [Item], open: @escaping (Item.ID) -> Void) {
        self.open = open
        let byID = Dictionary(items.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        var step: [Item.ID: Int] = [:]
        var edges: [Edge] = []
        for item in items {
            let milestone = item.parentID.flatMap { byID[$0] }
            // A task comes after its milestone, and waits on what it does.
            let waits = item.dependsOn + (milestone.map { [$0.id] } ?? [])
            // Only items already placed: in the plan's order, everything an
            // item waits on is.
            let after = waits.compactMap { step[$0] }.max()
            step[item.id] = after.map { $0 + 1 } ?? 0
            edges += item.dependsOn.filter { byID[$0] != nil }.map { Edge(from: $0, to: item.id) }
        }
        let count = (step.values.max() ?? -1) + 1
        var steps = Array(repeating: [Item](), count: count)
        for item in items {
            if let at = step[item.id] { steps[at].append(item) }
        }
        self.steps = steps
        self.edges = edges
    }

    var body: some View {
        if steps.isEmpty {
            ContentUnavailableView(
                "Nothing Planned", systemImage: "point.3.connected.trianglepath.dotted",
                description: Text("Add a task, then set what it depends on from its page."))
        } else {
            ScrollView([.horizontal, .vertical]) {
                HStack(alignment: .top, spacing: 48) {
                    ForEach(steps.indices, id: \.self) { at in
                        VStack(alignment: .leading, spacing: 12) {
                            ForEach(steps[at]) { item in
                                node(item)
                            }
                        }
                    }
                }
                .padding()
                .overlayPreferenceValue(NodeBounds.self) { bounds in
                    GeometryReader { proxy in
                        Canvas { context, _ in
                            for edge in edges {
                                guard let from = bounds[edge.from], let to = bounds[edge.to] else { continue }
                                let start = proxy[from]
                                let end = proxy[to]
                                var path = Path()
                                let tail = CGPoint(x: start.maxX, y: start.midY)
                                let tip = CGPoint(x: end.minX, y: end.midY)
                                path.move(to: tail)
                                path.addCurve(
                                    to: tip, control1: CGPoint(x: tail.x + 24, y: tail.y),
                                    control2: CGPoint(x: tip.x - 24, y: tip.y))
                                context.stroke(path, with: .color(.secondary.opacity(0.6)), lineWidth: 1)
                                var head = Path()
                                head.move(to: tip)
                                head.addLine(to: CGPoint(x: tip.x - 6, y: tip.y - 4))
                                head.addLine(to: CGPoint(x: tip.x - 6, y: tip.y + 4))
                                head.closeSubpath()
                                context.fill(head, with: .color(.secondary.opacity(0.6)))
                            }
                        }
                        .allowsHitTesting(false)
                    }
                }
            }
        }
    }

    private func node(_ item: Item) -> some View {
        Button {
            open(item.id)
        } label: {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 4) {
                    if item.isMilestone {
                        Image(systemName: "flag")
                            .accessibilityLabel("Milestone")
                    }
                    Text(item.title)
                        .font(item.isMilestone ? .callout.bold() : .callout)
                        .lineLimit(2)
                }
                HStack(spacing: 4) {
                    Text(item.status.title)
                    if item.blocked {
                        Label("Blocked", systemImage: "lock")
                    }
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            .padding(8)
            .frame(width: 180, alignment: .leading)
            .background(.background.secondary, in: .rect(cornerRadius: 8))
            .overlay {
                RoundedRectangle(cornerRadius: 8)
                    .strokeBorder(item.blocked ? Color.orange.opacity(0.5) : Color.secondary.opacity(0.2))
            }
            .opacity(item.closed ? 0.6 : 1)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .anchorPreference(key: NodeBounds.self, value: .bounds) { [item.id: $0] }
        .help("Open \(item.title)")
    }
}

/// Where each item's node is, for the arrows between them.
private struct NodeBounds: PreferenceKey {
    static let defaultValue: [Item.ID: Anchor<CGRect>] = [:]

    static func reduce(value: inout [Item.ID: Anchor<CGRect>], nextValue: () -> [Item.ID: Anchor<CGRect>]) {
        value.merge(nextValue()) { _, new in new }
    }
}
