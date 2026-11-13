import Foundation

extension InMemoryDaemon {
    /// Every message, reply, command and output with all of the query's
    /// words, case ignored, then those with a related word; newest first.
    /// Times named in the words aren't read, unlike the daemon's search.
    func search(_ query: SearchQuery) throws -> SearchResults {
        let words = Self.words(query.text)
        guard !words.isEmpty else {
            throw DaemonError.refused(
                .invalidRequest,
                message: "Give some words to search for; a time alone, as \"last week\", isn't enough.")
        }
        let state = state.value
        var hits: [SearchHit] = []
        for session in state.sessions {
            if let project = query.projectID, session.projectID != project { continue }
            var turn = 1
            for event in state.histories[session.id] ?? [] {
                defer { if case .turnEnded = event.kind { turn += 1 } }
                if let since = query.since, event.time < since { continue }
                if let until = query.until, event.time >= until { continue }
                guard let (kind, text) = Self.searched(event) else { continue }
                let lowered = text.lowercased()
                let related: Bool
                if words.allSatisfy(lowered.contains) {
                    related = false
                } else if query.related.contains(where: { lowered.contains($0.lowercased()) }) {
                    related = true
                } else {
                    continue
                }
                hits.append(
                    SearchHit(
                        kind: kind, sessionID: session.id, sessionName: session.name, eventID: event.id,
                        turn: turn, time: event.time, projectID: session.projectID, itemID: session.itemID,
                        threadID: nil, snippet: Self.marked(text, words + query.related.map { $0.lowercased() }),
                        related: related))
            }
        }
        hits.sort { ($0.related ? 1 : 0, $1.time) < ($1.related ? 1 : 0, $0.time) }
        return SearchResults(hits: hits, words: words.joined(separator: " "), since: query.since, until: query.until)
    }

    private static func words(_ text: String) -> [String] {
        text.lowercased().split(whereSeparator: \.isWhitespace).map(String.init)
    }

    /// What of `event` is searched: a command by itself, not its JSON.
    private static func searched(_ event: SessionEvent) -> (SearchHit.Kind, String)? {
        switch event.kind {
        case .userMessage(let text, _): (.message, text)
        case .assistantMessage(let text): (.reply, text)
        case .toolStarted(_, let tool, let arguments):
            (.toolCall, command(arguments) ?? "\(tool) \(arguments)")
        case .toolCompleted(_, _, let output): (.toolOutput, output)
        default: nil
        }
    }

    private static func command(_ arguments: String) -> String? {
        guard let data = arguments.data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }
        return object["command"] as? String
    }

    /// `text` in runs, each of `words` in it marked found.
    private static func marked(_ text: String, _ words: [String]) -> [SearchHit.Run] {
        var runs: [SearchHit.Run] = []
        var rest = Substring(text)
        while !rest.isEmpty {
            let next =
                words
                .compactMap { word in word.isEmpty ? nil : rest.range(of: word, options: .caseInsensitive) }
                .min { $0.lowerBound < $1.lowerBound }
            guard let next else {
                runs.append(SearchHit.Run(text: String(rest), found: false))
                break
            }
            if next.lowerBound > rest.startIndex {
                runs.append(SearchHit.Run(text: String(rest[..<next.lowerBound]), found: false))
            }
            runs.append(SearchHit.Run(text: String(rest[next]), found: true))
            rest = rest[next.upperBound...]
        }
        return runs
    }
}
