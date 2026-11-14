import AppKit
import HumataDaemonClient
import SwiftUI
import XCTest

/// A window for measuring a view as a person uses it: under every other
/// window, so a run neither covers the screen nor takes the keyboard.
@MainActor
final class PerfWindow {
    let window: NSWindow
    private let clock = ContinuousClock()

    /// `root` in a window of `size`. The window keeps that size whatever
    /// its content asks for, unless `sizedByContent`: then the window's
    /// views check their content's smallest size on every change, as the
    /// app's do.
    init(
        size: CGSize = CGSize(width: 1180, height: 760), sizedByContent: Bool = false, _ root: some View
    ) {
        window = NSWindow(
            contentRect: CGRect(origin: CGPoint(x: 80, y: 80), size: size),
            styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.level = NSWindow.Level(rawValue: Int(CGWindowLevelForKey(.desktopIconWindow)) + 1)
        let hosting = NSHostingController(rootView: root)
        if !sizedByContent { hosting.sizingOptions = [] }
        window.contentViewController = hosting
        window.setContentSize(size)
        window.orderFrontRegardless()
    }

    func close() {
        window.close()
        window.contentViewController = nil
    }

    /// Lets queued layout and drawing finish.
    func settle() async {
        for _ in 0..<20 {
            draw()
            try? await Task.sleep(for: .milliseconds(20))
        }
    }

    /// Waits until `shown`, looking every 10 ms: how long it took, or nil
    /// when `timeout` passed first.
    func until(_ timeout: Duration = .seconds(60), _ shown: () -> Bool) async -> Duration? {
        let start = clock.now
        while !shown() {
            if clock.now - start >= timeout { return nil }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return clock.now - start
    }

    /// The scroll view with the tallest content, in a session the chat's;
    /// nil while the window holds none.
    func scroll() -> NSScrollView? {
        var found: [NSScrollView] = []
        func walk(_ view: NSView) {
            if let scroll = view as? NSScrollView { found.append(scroll) }
            for child in view.subviews { walk(child) }
        }
        if let content = window.contentView { walk(content) }
        return found.max { ($0.documentView?.frame.height ?? 0) < ($1.documentView?.frame.height ?? 0) }
    }

    /// Whether the end of `scroll`'s content is in view, give or take 40
    /// points.
    func showsEnd(of scroll: NSScrollView) -> Bool {
        let visible = scroll.contentView.documentVisibleRect
        let height = scroll.documentView?.frame.height ?? 0
        return (scroll.documentView?.isFlipped ?? true) ? visible.maxY >= height - 40 : visible.minY <= 40
    }

    /// Whether the end of `scroll`'s content comes into view within a
    /// second. A jump to the end of a lazy list lands short until the rows
    /// near the end are measured, so the end can lag a frame or two behind.
    func soonShowsEnd(of scroll: NSScrollView) async -> Bool {
        await until(.seconds(1)) { showsEnd(of: scroll) } != nil
    }

    /// Turns a mouse wheel over `scroll` by `lines` up to `times` times,
    /// 8 ms apart and while `going`, as a person turns it: in this process,
    /// not posted to the system, and without the phases a trackpad's events
    /// have. Each frame is the time from one turn to the next less the
    /// wait: what the view took to draw it, and what else ran in between.
    func wheel(
        _ scroll: NSScrollView, lines: Int32, times: Int, while going: () -> Bool = { true }
    ) async -> Frames {
        var frames: [Duration] = []
        var last = clock.now
        for _ in 0..<times where going() {
            if let event = CGEvent(
                scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 1, wheel1: lines * 12, wheel2: 0,
                wheel3: 0),
                let wheel = NSEvent(cgEvent: event)
            {
                scroll.scrollWheel(with: wheel)
            }
            try? await Task.sleep(for: .milliseconds(8))
            frames.append(clock.now - last - .milliseconds(8))
            last = clock.now
        }
        return Frames(frames)
    }

    /// Moves `scroll` to the offset `target` gives for the furthest offset
    /// there is, laid out and drawn: how long that took, and whether it
    /// reached either end.
    func move(_ scroll: NSScrollView, to target: (CGFloat) -> CGFloat) async -> (took: Duration, atAnEnd: Bool) {
        let height = scroll.documentView?.frame.height ?? 0
        let furthest = max(0, height - scroll.contentView.bounds.height)
        let offset = min(max(target(furthest), 0), furthest)
        let start = clock.now
        scroll.contentView.scroll(to: NSPoint(x: 0, y: offset))
        scroll.reflectScrolledClipView(scroll.contentView)
        draw()
        await Task.yield()
        return (clock.now - start, offset <= 0 || offset >= furthest)
    }

    /// Reads `scroll` from its end up to its start and down again, `points`
    /// at a time. With no end in sight, each way stops at 8,000 steps.
    func page(_ scroll: NSScrollView, by points: CGFloat) async -> Frames {
        var frames: [Duration] = []
        var position = CGFloat.greatestFiniteMagnitude
        for _ in 0..<8_000 {
            let step = await move(scroll) { furthest in
                position = min(position, furthest) - points
                return position
            }
            frames.append(step.took)
            if step.atAnEnd, position <= 0 { break }
        }
        position = 0
        for _ in 0..<8_000 {
            let step = await move(scroll) { furthest in
                position += points
                return min(position, furthest)
            }
            frames.append(step.took)
            if step.atAnEnd, position > 0 { break }
        }
        return Frames(frames)
    }

    private func draw() {
        window.contentView?.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
    }
}

/// How long each of some steps took.
struct Frames: CustomStringConvertible {
    private let sorted: [Duration]

    init(_ times: [Duration]) { sorted = times.sorted() }

    var count: Int { sorted.count }
    var median: Duration { sorted.isEmpty ? .zero : sorted[sorted.count / 2] }
    var p95: Duration { sorted.isEmpty ? .zero : sorted[sorted.count * 95 / 100] }
    var worst: Duration { sorted.last ?? .zero }

    /// How many took longer than `limit`.
    func over(_ limit: Duration) -> Int { sorted.filter { $0 > limit }.count }

    static func + (one: Frames, other: Frames) -> Frames { Frames(one.sorted + other.sorted) }

    var description: String {
        "\(count) steps, median \(median.inMilliseconds) ms, p95 \(p95.inMilliseconds) ms, "
            + "worst \(worst.inMilliseconds) ms"
    }
}

/// What the main thread did while it was watched.
struct MainThread: CustomStringConvertible {
    /// The longest it went without waiting for something to do: nothing is
    /// drawn and no key or click is taken in that time.
    let longest: Duration
    /// The part of the time it was at work, from 0 to 1.
    let busy: Double
    /// How long it had been watched when the longest pass began, to tell
    /// what the pass was for.
    var longestAt = Duration.zero

    var description: String {
        "longest pass \(longest.inMilliseconds) ms (\(longestAt.inMilliseconds) ms in), "
            + "busy \(Int((busy * 100).rounded()))%"
    }

    /// Watches the main thread until `stop()`. A pass is from when its run
    /// loop wakes to when it next waits; the one under way at either end
    /// counts for the part of it that was watched. A pass ends after
    /// everything else that runs before the wait: the window's layout and
    /// drawing are done there, and are most of what a view costs.
    @MainActor
    final class Watch {
        private let passes: Passes
        private let observers: [CFRunLoopObserver]
        private let clock: ContinuousClock
        private let start: ContinuousClock.Instant

        init() {
            let clock = ContinuousClock()
            self.clock = clock
            let now = clock.now
            start = now
            let passes = Passes(start: now)
            passes.woke = now
            self.passes = passes
            // First of all to hear the run loop wake, and last of all to
            // hear it's about to wait: after the window's display cycle and
            // its layers' commit, which are observers of the same kind.
            let woke = CFRunLoopObserverCreateWithHandler(
                nil, CFRunLoopActivity.afterWaiting.rawValue, true, CFIndex.min
            ) { [passes, clock] _, _ in
                passes.woke = clock.now
            }
            let waits = CFRunLoopObserverCreateWithHandler(
                nil, CFRunLoopActivity.beforeWaiting.rawValue, true, CFIndex.max
            ) { [passes, clock] _, _ in
                passes.end(at: clock.now)
            }
            observers = [woke, waits].compactMap { $0 }
            for observer in observers { CFRunLoopAddObserver(CFRunLoopGetMain(), observer, .commonModes) }
        }

        func stop() -> MainThread {
            for observer in observers { CFRunLoopRemoveObserver(CFRunLoopGetMain(), observer, .commonModes) }
            let now = clock.now
            passes.end(at: now)
            let watched = now - start
            let busy = watched > .zero ? passes.total / watched : 0
            return MainThread(longest: passes.longest, busy: min(max(busy, 0), 1), longestAt: passes.longestAt)
        }
    }

    /// The passes seen so far. Only the main thread touches it: the run
    /// loop's observer runs there, as the watch does.
    private final class Passes: @unchecked Sendable {
        var woke: ContinuousClock.Instant?
        var longest = Duration.zero
        var longestAt = Duration.zero
        var total = Duration.zero
        private let start: ContinuousClock.Instant

        init(start: ContinuousClock.Instant) {
            self.start = start
        }

        func end(at now: ContinuousClock.Instant) {
            guard let woke else { return }
            let pass = now - woke
            if pass > longest {
                longest = pass
                longestAt = woke - start
            }
            total += pass
            self.woke = nil
        }
    }
}

enum Memory {
    /// The memory the process uses, as Activity Monitor counts it.
    static func footprint() -> UInt64 {
        var info = task_vm_info_data_t()
        var count = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<natural_t>.size)
        let result = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
            }
        }
        return result == KERN_SUCCESS ? info.phys_footprint : 0
    }

    /// How much more the process uses now than the `before` it used then;
    /// none when it uses less.
    static func grown(since before: UInt64) -> UInt64 {
        let now = footprint()
        return now > before ? now - before : 0
    }
}

enum Budget {
    /// Whether times are checked against budgets: only with `HUMATA_PERF`
    /// set, since how long a step takes depends on the machine, on what
    /// else runs on it and on whether the build is optimized.
    static var timed: Bool { ProcessInfo.processInfo.environment["HUMATA_PERF"] != nil }

    /// Prints one measure on a line of its own, and keeps it in the test's
    /// report.
    static func note(_ line: String) {
        print("perf: \(line)")
        XCTContext.runActivity(named: line) { _ in }
    }
}

enum Seeds {
    /// A working session's history of `turns` turns of every kind of row,
    /// the same for the same `seed`.
    static func chat(turns: Int, seed: UInt64) -> [SessionEvent] {
        var maker = ChatMaker(seed: seed)
        return maker.history(turns: turns)
    }
}

extension Duration {
    /// In milliseconds, to a tenth.
    var inMilliseconds: Double {
        let (seconds, attoseconds) = components
        return ((Double(seconds) * 1_000 + Double(attoseconds) / 1e15) * 10).rounded() / 10
    }
}
