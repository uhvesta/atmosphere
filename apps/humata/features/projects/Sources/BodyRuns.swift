/// How many times the sidebar's views were drawn again, by view, so a test
/// can check that a change draws only what shows it: a read of the list
/// that brings nothing new to show leaves the rows alone. Counted in debug
/// builds only; the app as released counts nothing.
@MainActor
package enum SidebarBodyRuns {
    package enum Body: Hashable, Sendable {
        /// The window's sidebar: what holds the list.
        case sidebar
        /// The projects with their sessions, not the rows themselves.
        case list
        /// Any session's row.
        case sessionRow
        /// The inbox's row, with its count.
        case inbox
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
