import SwiftUI

/// Lays chips out in rows, starting a new row when the next one doesn't
/// fit, and lets a chip wider than the row wrap its text.
struct ChipFlow: Layout {
    var spacing: CGFloat = 8

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = proposal.width ?? .infinity
        let frames = arrange(subviews, in: width)
        let used = frames.reduce(CGSize.zero) { size, frame in
            CGSize(width: max(size.width, frame.maxX), height: max(size.height, frame.maxY))
        }
        return CGSize(width: width.isFinite ? width : used.width, height: used.height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let frames = arrange(subviews, in: bounds.width)
        for (subview, frame) in zip(subviews, frames) {
            subview.place(
                at: CGPoint(x: bounds.minX + frame.minX, y: bounds.minY + frame.minY),
                proposal: ProposedViewSize(frame.size))
        }
    }

    /// Each chip's frame, row by row, none wider than `width`.
    private func arrange(_ subviews: Subviews, in width: CGFloat) -> [CGRect] {
        var frames: [CGRect] = []
        var origin = CGPoint.zero
        var rowHeight: CGFloat = 0
        for subview in subviews {
            // A chip takes the width its text needs, up to the whole row.
            var size = subview.sizeThatFits(
                width.isFinite ? ProposedViewSize(width: width, height: nil) : .unspecified)
            size.width = min(size.width, width)
            if origin.x > 0, origin.x + size.width > width {
                origin = CGPoint(x: 0, y: origin.y + rowHeight + spacing)
                rowHeight = 0
            }
            frames.append(CGRect(origin: origin, size: size))
            origin.x += size.width + spacing
            rowHeight = max(rowHeight, size.height)
        }
        return frames
    }
}

/// A chip's look: filled when it has a value, a dashed outline with a
/// plus when it only offers to add one.
struct Chip: View {
    let systemImage: String
    let title: Text
    var isSet = true
    var tint: Color?

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 4) {
            Image(systemName: isSet ? systemImage : "plus")
                .accessibilityHidden(true)
            title
                .multilineTextAlignment(.leading)
                .fixedSize(horizontal: false, vertical: true)
        }
        .font(.subheadline)
        .foregroundStyle(isSet ? AnyShapeStyle(tint ?? .primary) : AnyShapeStyle(.secondary))
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background {
            let shape = RoundedRectangle(cornerRadius: 12, style: .continuous)
            if isSet {
                shape.fill(.quaternary)
            } else {
                shape.strokeBorder(.tertiary, style: StrokeStyle(lineWidth: 1, dash: [3, 3]))
            }
        }
        .contentShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
    }
}

extension View {
    /// A menu whose label is a chip, without the platform's button chrome.
    func chipMenu() -> some View {
        menuStyle(.button)
            .buttonStyle(.plain)
            .menuIndicator(.hidden)
    }
}
