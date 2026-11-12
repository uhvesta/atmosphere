import Foundation

/// What to search for across every session.
package struct SearchQuery: Hashable, Sendable {
    /// Words to find, every one; the last may be the start of a word. A
    /// time in them, as "last week" or "past 3 days", narrows to then.
    package var text: String
    /// Words that mean much the same, found as well and marked related.
    package var related: [String]
    package var projectID: String?
    /// From and before these, when the text names no time.
    package var since: Date?
    package var until: Date?

    package init(
        text: String, related: [String] = [], projectID: String? = nil, since: Date? = nil, until: Date? = nil
    ) {
        self.text = text
        self.related = related
        self.projectID = projectID
        self.since = since
        self.until = until
    }
}

/// Where a search found its words, and the text around them.
package struct SearchHit: Hashable, Sendable, Identifiable {
    package enum Kind: Hashable, Sendable {
        case message
        case reply
        case toolCall
        case toolOutput
        case comment
        case spec
    }

    package var kind: Kind
    /// Nil for a spec.
    package var sessionID: String?
    package var sessionName: String
    /// The event in the session's history; nil for a comment or a spec.
    package var eventID: String?
    /// The turn it's in, from 1; 0 for a comment or a spec.
    package var turn: Int
    package var time: Date
    package var projectID: String?
    package var itemID: String?
    /// The review thread of a comment.
    package var threadID: Int64?
    /// The text around what was found, as runs of plain and found text.
    package var snippet: [Run]
    /// Found by a related word, not the query's own.
    package var related: Bool

    package struct Run: Hashable, Sendable {
        package var text: String
        package var found: Bool

        package init(text: String, found: Bool) {
            self.text = text
            self.found = found
        }
    }

    package var id: String {
        [sessionID ?? "", eventID ?? "", threadID.map(String.init) ?? "", itemID ?? "", "\(kind)"]
            .joined(separator: "/")
    }

    package init(
        kind: Kind, sessionID: String?, sessionName: String, eventID: String?, turn: Int, time: Date,
        projectID: String?, itemID: String?, threadID: Int64?, snippet: [Run], related: Bool
    ) {
        self.kind = kind
        self.sessionID = sessionID
        self.sessionName = sessionName
        self.eventID = eventID
        self.turn = turn
        self.time = time
        self.projectID = projectID
        self.itemID = itemID
        self.threadID = threadID
        self.snippet = snippet
        self.related = related
    }

    /// `snippet` split into runs, each word found marked as the daemon
    /// marks it: between U+E000 and U+E001.
    package static func runs(_ snippet: String) -> [Run] {
        var runs: [Run] = []
        var text = ""
        var found = false
        for character in snippet {
            if character == "\u{E000}" || character == "\u{E001}" {
                if !text.isEmpty { runs.append(Run(text: text, found: found)) }
                text = ""
                found = character == "\u{E000}"
            } else {
                text.append(character)
            }
        }
        if !text.isEmpty { runs.append(Run(text: text, found: found)) }
        return runs
    }
}

/// What a search found.
package struct SearchResults: Hashable, Sendable {
    package var hits: [SearchHit]
    /// The words searched, without the time the query named.
    package var words: String
    /// The time searched, if any.
    package var since: Date?
    package var until: Date?

    package init(hits: [SearchHit], words: String, since: Date? = nil, until: Date? = nil) {
        self.hits = hits
        self.words = words
        self.since = since
        self.until = until
    }
}
