import SwiftUI

/// One column of a board: its title and how many cards it holds, the
/// cards, then `footer`. Dropping dragged cards on it hands their ids to
/// `onDrop`, which says whether it took them.
public struct BoardColumn<Cards: View, Footer: View>: View {
    private let title: String
    private let count: Int
    private let countLabel: Text
    private let onDrop: ([String]) -> Bool
    private let cards: Cards
    private let footer: Footer
    @State private var isTargeted = false

    /// `countLabel` is what VoiceOver reads for the header, such as
    /// "To do, 3 tasks".
    public init(
        title: String, count: Int, countLabel: Text, @ViewBuilder cards: () -> Cards,
        @ViewBuilder footer: () -> Footer = { EmptyView() }, onDrop: @escaping ([String]) -> Bool
    ) {
        self.title = title
        self.count = count
        self.countLabel = countLabel
        self.onDrop = onDrop
        self.cards = cards()
        self.footer = footer()
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("\(title.uppercased()) (\(count))")
                .font(.caption.bold())
                .foregroundStyle(.secondary)
                .accessibilityAddTraits(.isHeader)
                .accessibilityLabel(countLabel)
            cards
            footer
            Spacer(minLength: 40)
        }
        .frame(width: 200, alignment: .topLeading)
        .frame(maxHeight: .infinity, alignment: .top)
        .padding(8)
        .background(
            RoundedRectangle(cornerRadius: 10)
                .fill(isTargeted ? Color.accentColor.opacity(0.15) : Color.secondary.opacity(0.08))
        )
        .dropDestination(for: String.self) { ids, _ in
            onDrop(ids)
        } isTargeted: {
            isTargeted = $0
        }
    }
}

/// A card on a board: `content` on a rounded background. Tapping it, or
/// VoiceOver's default action, calls `onTap`; it drags as `id`, and its
/// context menu is `menu`, such as a button per column to move it to.
/// With `onDrop`, cards dragged onto it hand their ids to it, which says
/// whether it took them, such as to reorder.
public struct BoardCard<Content: View, Menu: View>: View {
    private let id: String
    private let onDrop: (([String]) -> Bool)?
    private let onTap: () -> Void
    private let content: Content
    private let menu: Menu
    /// Whether a card dragged is over this one, to drop onto it.
    @State private var isTargeted = false
    /// The card's width, so the one lifted while dragging keeps it.
    @State private var width: CGFloat = 240
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    public init(
        id: String, onDrop: (([String]) -> Bool)? = nil, @ViewBuilder content: () -> Content,
        @ViewBuilder menu: () -> Menu, onTap: @escaping () -> Void
    ) {
        self.id = id
        self.onDrop = onDrop
        self.onTap = onTap
        self.content = content()
        self.menu = menu()
    }

    public var body: some View {
        card
            .frame(maxWidth: .infinity, alignment: .leading)
            .onGeometryChange(for: CGFloat.self, of: \.size.width) { width = $0 }
            .contentShape(Rectangle())
            .onTapGesture(perform: onTap)
            .draggable(id) {
                // Lifted off the column while it moves: tilted, with a shadow.
                card
                    .frame(width: width, alignment: .leading)
                    .rotationEffect(.degrees(reduceMotion ? 0 : 2))
                    .shadow(color: .black.opacity(0.2), radius: 8, y: 4)
                    .padding(12)
            }
            .modifier(CardDrop(onDrop: onDrop, isTargeted: $isTargeted))
            .overlay {
                if isTargeted {
                    RoundedRectangle(cornerRadius: 8).strokeBorder(.tint, lineWidth: 2).accessibilityHidden(true)
                }
            }
            .accessibilityElement(children: .combine)
            .accessibilityAction { onTap() }
            .contextMenu { menu }
    }

    private var card: some View {
        content
            .padding(8)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: 8).fill(.background))
    }
}

/// Takes cards dropped onto a card, when it has somewhere to send them.
private struct CardDrop: ViewModifier {
    let onDrop: (([String]) -> Bool)?
    @Binding var isTargeted: Bool

    func body(content: Content) -> some View {
        if let onDrop {
            content.dropDestination(for: String.self) { ids, _ in
                onDrop(ids)
            } isTargeted: {
                isTargeted = $0
            }
        } else {
            content
        }
    }
}
