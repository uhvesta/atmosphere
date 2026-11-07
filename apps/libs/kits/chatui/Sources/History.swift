import SwiftUI

/// A conversation in a list: its title, when it was last written in, and
/// what's going on in it or the last thing said, with a badge for what
/// waits in it and a cloud when it syncs.
public struct ConversationRow: View {
    public struct Badge {
        public var text: LocalizedStringKey
        public var systemImage: String

        public init(_ text: LocalizedStringKey, systemImage: String) {
            self.text = text
            self.systemImage = systemImage
        }
    }

    let title: String
    let updatedAt: Date
    let isReplying: Bool
    let preview: String?
    let previewIsModel: Bool
    let badge: Badge?
    let isSynced: Bool

    /// `preview` is the line under the title, such as `Snippet.around`
    /// what a search found; `previewIsModel` when a model wrote it.
    public init(
        title: String, updatedAt: Date, isReplying: Bool, preview: String?, previewIsModel: Bool,
        badge: Badge? = nil, isSynced: Bool
    ) {
        self.title = title
        self.updatedAt = updatedAt
        self.isReplying = isReplying
        self.preview = preview
        self.previewIsModel = previewIsModel
        self.badge = badge
        self.isSynced = isSynced
    }

    public var body: some View {
        HStack(alignment: .top) {
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .firstTextBaseline) {
                    Text(title)
                        .font(.headline)
                        .lineLimit(1)
                    Spacer()
                    Text(updatedAt, format: .dateTime.month().day().hour().minute())
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                if isReplying {
                    Text("Replying…")
                        .font(.subheadline)
                        .foregroundStyle(.tint)
                } else if let preview {
                    Text(preview)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                        .accessibilityIdentifier(previewIsModel ? ModelText.identifier : "")
                }
                if let badge {
                    Label(badge.text, systemImage: badge.systemImage)
                        .font(.caption)
                        .foregroundStyle(.tint)
                }
            }
            if isSynced {
                Image(systemName: "icloud")
                    .foregroundStyle(.secondary)
                    .accessibilityLabel("Synced")
            }
        }
        .padding(.vertical, 4)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(.rect)
    }
}

/// Where a message shows what was searched for.
public enum Snippet {
    /// `text` from a little before the first of `searched` in it, from the
    /// start of a word, when that's more than 40 characters in; else all of
    /// it. Matches ignore case and accents.
    public static func around(_ searched: [String], in text: String) -> String {
        let ranges = searched.compactMap { text.range(of: $0, options: [.caseInsensitive, .diacriticInsensitive]) }
        guard let range = ranges.min(by: { $0.lowerBound < $1.lowerBound }),
            text.distance(from: text.startIndex, to: range.lowerBound) > 40
        else { return text }
        // From the start of a word, so the snippet doesn't open mid-word.
        let near = text.index(range.lowerBound, offsetBy: -30)
        let start = text[near..<range.lowerBound].firstIndex(where: \.isWhitespace).map(text.index(after:)) ?? near
        return "…" + text[start...]
    }
}
