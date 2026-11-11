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
    let searched: [String]

    /// `preview` is the line under the title, such as `Snippet.around`
    /// what a search found, with `searched` in it in bold; `previewIsModel`
    /// when a model wrote it.
    public init(
        title: String, updatedAt: Date, isReplying: Bool, preview: String?, previewIsModel: Bool,
        badge: Badge? = nil, isSynced: Bool, searched: [String] = []
    ) {
        self.searched = searched
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
                    Text(Markdown.plainLine(title))
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
                    Text(highlighted(previewIsModel ? Markdown.plainText(preview) : Markdown.plainLine(preview)))
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

    /// `text` with what was searched for in it in bold, in the primary
    /// color, so a match stands out from the rest of the line.
    private func highlighted(_ text: String) -> AttributedString {
        var attributed = AttributedString(text)
        for match in Snippet.matches(searched, in: text) {
            guard let lower = AttributedString.Index(match.lowerBound, within: attributed),
                let upper = AttributedString.Index(match.upperBound, within: attributed)
            else { continue }
            attributed[lower..<upper].inlinePresentationIntent = .stronglyEmphasized
            attributed[lower..<upper].foregroundColor = .primary
        }
        return attributed
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

    /// Where each of `searched` is in `text`, every time it's there.
    /// Matches ignore case and accents.
    public static func matches(_ searched: [String], in text: String) -> [Range<String.Index>] {
        searched.filter { !$0.isEmpty }.flatMap { word in
            var found: [Range<String.Index>] = []
            var from = text.startIndex
            while let range = text.range(
                of: word, options: [.caseInsensitive, .diacriticInsensitive], range: from..<text.endIndex)
            {
                found.append(range)
                from = range.upperBound
            }
            return found
        }
    }
}
