/// How many times the session's views were drawn again, by view, so a test
/// can check that a change draws only what shows it: typing leaves the
/// chat alone, and a streamed word only the reply being written. Counted in
/// debug builds only; the app as released counts nothing.
@MainActor
package enum SessionBodyRuns {
    package enum Body: Hashable, Sendable {
        /// The tab's frame: what holds the header, the chat and the box.
        case session
        case header
        /// The chat's list of rows, not the rows themselves.
        case chatList
        /// Any finished row of the chat.
        case chatRow
        /// The turn under way at the chat's end, with the reply as it streams.
        case turnInProgress
        /// The box to write in, with what sits over it.
        case composer
    }

    #if DEBUG
        package private(set) static var counts: [Body: Int] = [:]
    #endif

    static func ran(_ body: Body) {
        #if DEBUG
            counts[body, default: 0] += 1
        #endif
    }
}
