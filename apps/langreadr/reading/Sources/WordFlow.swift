import SwiftUI

/// Lays its views out as text is: left to right, onto the next line when
/// the next doesn't fit. Right to left for languages written that way, as
/// SwiftUI mirrors it.
struct WordFlow: Layout {
    var lineSpacing: CGFloat = 4

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = proposal.width ?? .infinity
        let lines = Self.lines(subviews, width: width)
        let height = lines.map(\.height).reduce(0, +) + lineSpacing * CGFloat(max(0, lines.count - 1))
        let used = lines.map(\.width).max() ?? 0
        return CGSize(width: proposal.width ?? used, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var top = bounds.minY
        for line in Self.lines(subviews, width: bounds.width) {
            var left = bounds.minX
            for index in line.indices {
                let size = subviews[index].sizeThatFits(.unspecified)
                subviews[index].place(
                    at: CGPoint(x: left, y: top + line.height - size.height), proposal: ProposedViewSize(size))
                left += size.width
            }
            top += line.height + lineSpacing
        }
    }

    private struct Line {
        var indices: [Int] = []
        var width: CGFloat = 0
        var height: CGFloat = 0
    }

    private static func lines(_ subviews: Subviews, width: CGFloat) -> [Line] {
        var lines: [Line] = [Line()]
        for index in subviews.indices {
            let size = subviews[index].sizeThatFits(.unspecified)
            if lines[lines.count - 1].width + size.width > width, !lines[lines.count - 1].indices.isEmpty {
                lines.append(Line())
            }
            lines[lines.count - 1].indices.append(index)
            lines[lines.count - 1].width += size.width
            lines[lines.count - 1].height = max(lines[lines.count - 1].height, size.height)
        }
        return lines
    }
}
