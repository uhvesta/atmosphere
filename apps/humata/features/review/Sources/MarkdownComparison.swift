import Foundation
import SwiftUI

/// A rendered block of a Markdown file compared with another version of it:
/// kept, added since, or removed since.
package struct ComparedBlock: Hashable, Sendable, Identifiable {
    package enum Change: Hashable, Sendable {
        case kept
        case added
        case removed
    }

    package var block: MarkdownBlock
    package var change: Change

    package init(block: MarkdownBlock, change: Change) {
        self.block = block
        self.change = change
    }

    /// Kept and added blocks are the new version's, removed ones the old
    /// one's, so their first lines don't collide within a change.
    package var id: String { "\(change)-\(block.startLine)" }
}

/// How two versions of a Markdown file differ, worked out off the main
/// thread by whoever asks: both are a line or block diff.
package enum MarkdownComparison {
    /// The lines of `new`, from 1, that changed since `old`: each added
    /// line, and where lines were removed, the line now in their place.
    package static func changedLines(from old: [String], to new: [String]) -> Set<Int> {
        guard old != new else { return [] }
        var changed = Set<Int>()
        walk(new.difference(from: old), old: old.count, new: new.count) { step in
            switch step {
            case .removed(_, let next):
                if new.isEmpty { return }
                changed.insert(min(next, new.count - 1) + 1)
            case .added(let index): changed.insert(index + 1)
            case .kept: break
            }
        }
        return changed
    }

    /// `old` and `new` rendered as one document: `new`'s blocks, with the
    /// blocks `old` had and `new` doesn't where they were.
    package static func compare(_ old: [String], with new: [String]) -> [ComparedBlock] {
        let before = MarkdownBlock.blocks(old)
        let after = MarkdownBlock.blocks(new)
        let difference = after.map(Key.init).difference(from: before.map(Key.init))
        var compared: [ComparedBlock] = []
        walk(difference, old: before.count, new: after.count) { step in
            switch step {
            case .removed(let index, _): compared.append(ComparedBlock(block: before[index], change: .removed))
            case .added(let index): compared.append(ComparedBlock(block: after[index], change: .added))
            case .kept(let index): compared.append(ComparedBlock(block: after[index], change: .kept))
            }
        }
        return compared
    }

    /// A block as compared: what it is and says, not where it is.
    private struct Key: Hashable {
        var kind: MarkdownBlock.Kind
        var language: String
        var text: String

        init(_ block: MarkdownBlock) {
            kind = block.kind
            language = block.language
            text = block.text
        }
    }

    private enum Step {
        /// An old element, by index, gone before the new one at `next`.
        case removed(Int, next: Int)
        case added(Int)
        case kept(Int)
    }

    /// The difference as the steps that turn the old elements into the new,
    /// in order: removals before the additions in their place.
    private static func walk<T>(
        _ difference: CollectionDifference<T>, old: Int, new: Int, _ step: (Step) -> Void
    ) {
        var removed = Set<Int>()
        var added = Set<Int>()
        for change in difference {
            switch change {
            case .remove(let offset, _, _): removed.insert(offset)
            case .insert(let offset, _, _): added.insert(offset)
            }
        }
        var index = (old: 0, new: 0)
        while index.old < old || index.new < new {
            if index.old < old, removed.contains(index.old) {
                step(.removed(index.old, next: index.new))
                index.old += 1
            } else if index.new < new, added.contains(index.new) {
                step(.added(index.new))
                index.new += 1
            } else if index.old < old, index.new < new {
                step(.kept(index.new))
                index.old += 1
                index.new += 1
            } else {
                // A well-formed difference never gets here; stop rather
                // than loop.
                break
            }
        }
    }
}

/// Two versions of a Markdown file as one rendered document, with what the
/// newer added and what it removed marked.
package struct ComparedDocument: View {
    let blocks: [ComparedBlock]

    package init(blocks: [ComparedBlock]) {
        self.blocks = blocks
    }

    package var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 12) {
                ForEach(blocks) { compared in
                    MarkdownBlockView(block: compared.block)
                        .strikethrough(compared.change == .removed)
                        .opacity(compared.change == .removed ? 0.7 : 1)
                        .padding(.vertical, 2)
                        .padding(.leading, 10)
                        .background(Self.tint(compared.change), in: .rect(cornerRadius: 4))
                        .overlay(alignment: .leading) {
                            if compared.change != .kept {
                                Rectangle().fill(Self.color(compared.change)).frame(width: 3)
                            }
                        }
                        .accessibilityElement(children: .combine)
                        .accessibilityValue(Self.label(compared.change))
                }
            }
            .frame(maxWidth: 820, alignment: .leading)
            .padding()
            .frame(maxWidth: .infinity)
        }
    }

    private static func color(_ change: ComparedBlock.Change) -> Color {
        switch change {
        case .kept: .clear
        case .added: .green
        case .removed: .red
        }
    }

    private static func tint(_ change: ComparedBlock.Change) -> Color {
        color(change).opacity(change == .kept ? 0 : 0.12)
    }

    private static func label(_ change: ComparedBlock.Change) -> String {
        switch change {
        case .kept: ""
        case .added: "Added since"
        case .removed: "Removed since"
        }
    }
}

extension View {
    /// Marks a block an edit just changed, until another file is picked.
    package func changedMark(_ changed: Bool) -> some View {
        overlay(alignment: .leading) {
            if changed {
                Rectangle().fill(Color.orange).frame(width: 3)
                    .help("Changed when this last reloaded")
                    .accessibilityLabel("Changed")
            }
        }
    }
}
