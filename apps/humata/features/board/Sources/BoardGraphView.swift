import ComposableArchitecture
import HumataDaemonClient
import SwiftUI

/// The board as its plan: a column per step, each item in the first column
/// after everything it waits on, with an arrow from each item to what waits
/// on it. A milestone's tasks wait on what it does, and blocked items say so.
struct BoardGraphView: View {
    /// Worked out as the board is read, not as the view draws.
    let graph: PlanGraph
    let items: IdentifiedArrayOf<Item>
    let open: (Item.ID) -> Void

    var body: some View {
        if graph.columns.isEmpty {
            ContentUnavailableView(
                "Nothing Planned", systemImage: "point.3.connected.trianglepath.dotted",
                description: Text("Add a task, then set what it depends on from its page."))
        } else {
            ScrollView([.horizontal, .vertical]) {
                HStack(alignment: .top, spacing: 48) {
                    ForEach(graph.columns.indices, id: \.self) { at in
                        VStack(alignment: .leading, spacing: 12) {
                            ForEach(graph.columns[at], id: \.self) { id in
                                if let item = items[id: id] {
                                    node(item)
                                }
                            }
                        }
                    }
                }
                .padding()
                .overlayPreferenceValue(NodeBounds.self) { bounds in
                    GeometryReader { proxy in
                        Canvas { context, _ in
                            for edge in graph.edges {
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

/// Where the Graph draws each item: a column per step, each item one past
/// everything it waits on, and down a column in the plan's order. A task
/// waits on its milestone, and an item that waits on a milestone comes
/// after the milestone's tasks too.
package struct PlanGraph: Equatable, Sendable {
    package struct Spot: Hashable, Sendable {
        package var column: Int
        package var row: Int
    }

    /// An arrow from an item to one that waits on it.
    package struct Edge: Hashable, Sendable {
        package var from: Item.ID
        package var to: Item.ID
    }

    /// Each column's items, top to bottom.
    package private(set) var columns: [[Item.ID]] = []
    package private(set) var spots: [Item.ID: Spot] = [:]
    package private(set) var edges: [Edge] = []

    package init() {}

    /// Lays out `items`, the board's items in the plan's order. A loop,
    /// which the daemon refuses but an older board may hold, is cut where
    /// it's met.
    package init(planned items: [Item]) {
        var placing = Placing(items)
        for item in items {
            _ = placing.column(of: item.id)
        }
        var columns = Array(repeating: [Item.ID](), count: (placing.columns.values.max() ?? -1) + 1)
        var spots: [Item.ID: Spot] = [:]
        for item in items where spots[item.id] == nil {
            guard let column = placing.columns[item.id] else { continue }
            spots[item.id] = Spot(column: column, row: columns[column].count)
            columns[column].append(item.id)
        }
        self.columns = columns
        self.spots = spots
        edges = items.flatMap { item in
            item.dependsOn.filter { placing.byID[$0] != nil }.map { Edge(from: $0, to: item.id) }
        }
    }
}

/// Works out each item's column, once each.
private struct Placing {
    let byID: [Item.ID: Item]
    /// The items under each item, as a milestone's tasks.
    let children: [Item.ID: [Item.ID]]
    var columns: [Item.ID: Int] = [:]
    /// The items being placed, so a loop is cut rather than followed.
    private var placing: Set<Item.ID> = []
    private var ending: Set<Item.ID> = []

    init(_ items: [Item]) {
        let byID = Dictionary(items.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        var children: [Item.ID: [Item.ID]] = [:]
        for item in items {
            if let parent = item.parentID, byID[parent] != nil {
                children[parent, default: []].append(item.id)
            }
        }
        self.byID = byID
        self.children = children
    }

    /// One past its milestone and past the last column of each item it
    /// depends on; -1 for an item met again in a loop, so it counts for
    /// nothing there.
    mutating func column(of id: Item.ID) -> Int {
        if let known = columns[id] { return known }
        guard let item = byID[id], placing.insert(id).inserted else { return -1 }
        defer { placing.remove(id) }
        var after = -1
        if let parent = item.parentID, byID[parent] != nil {
            after = max(after, column(of: parent))
        }
        for on in item.dependsOn where byID[on] != nil {
            after = max(after, end(of: on))
        }
        columns[id] = after + 1
        return after + 1
    }

    /// The last column `id`, or anything under it, is in.
    private mutating func end(of id: Item.ID) -> Int {
        guard ending.insert(id).inserted else { return -1 }
        defer { ending.remove(id) }
        var last = column(of: id)
        for child in children[id] ?? [] {
            last = max(last, end(of: child))
        }
        return last
    }
}
