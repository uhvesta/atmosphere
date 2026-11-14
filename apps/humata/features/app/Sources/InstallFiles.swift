import Foundation

/// A build of Humata as its Info.plist names it: the version people see,
/// then the build number, compared part by part as numbers.
package struct BundleVersion: Comparable, Sendable, CustomStringConvertible {
    /// `CFBundleShortVersionString`, as 0.1.2.
    package var short: String
    /// `CFBundleVersion`, as 42.
    package var build: String

    package init(short: String, build: String) {
        self.short = short
        self.build = build
    }

    /// The version in an Info.plist's dictionary; nil without a
    /// `CFBundleShortVersionString`. A missing build number reads as 0.
    package init?(info: [String: Any]) {
        guard let short = info["CFBundleShortVersionString"] as? String,
            !short.trimmingCharacters(in: .whitespaces).isEmpty
        else { return nil }
        self.init(short: short, build: info["CFBundleVersion"] as? String ?? "0")
    }

    package var description: String { "\(short) (\(build))" }

    package static func < (lhs: BundleVersion, rhs: BundleVersion) -> Bool {
        let short = compare(lhs.short, rhs.short)
        if short != .orderedSame { return short == .orderedAscending }
        return compare(lhs.build, rhs.build) == .orderedAscending
    }

    package static func == (lhs: BundleVersion, rhs: BundleVersion) -> Bool {
        compare(lhs.short, rhs.short) == .orderedSame && compare(lhs.build, rhs.build) == .orderedSame
    }

    /// Dotted numbers compared part by part, a missing part as 0, so 1.2
    /// and 1.2.0 are the same. A part that isn't all digits counts by the
    /// digits it starts with, so 1.2b1 is 1.2.
    private static func compare(_ lhs: String, _ rhs: String) -> ComparisonResult {
        let left = parts(lhs)
        let right = parts(rhs)
        for index in 0..<max(left.count, right.count) {
            let mine = index < left.count ? left[index] : 0
            let theirs = index < right.count ? right[index] : 0
            if mine != theirs { return mine < theirs ? .orderedAscending : .orderedDescending }
        }
        return .orderedSame
    }

    private static func parts(_ version: String) -> [Int] {
        version.trimmingCharacters(in: .whitespaces).split(separator: ".", omittingEmptySubsequences: false).map {
            Int($0.prefix { $0.isASCII && $0.isNumber }) ?? 0
        }
    }
}

/// What the installer offers a copy of Humata opened from outside
/// Applications.
package enum InstallOffer: Equatable, Sendable {
    /// There's no copy in Applications, or an older one: this copy goes in
    /// its place. `replacing` is the version there, if any.
    case install(replacing: BundleVersion?)
    /// The copy in Applications is this one's version or newer: it's
    /// opened instead, and nothing is copied.
    case openInstalled(BundleVersion)
}

/// What the installer found when the app opened: this copy, where it would
/// go, and what's there now.
package struct InstallSurvey: Equatable, Sendable {
    /// This copy of the app, wherever it was opened from.
    package var source: URL
    /// Where it goes: `Humata.app` in Applications.
    package var destination: URL
    /// This copy's version.
    package var version: BundleVersion
    /// The version of the copy in Applications; nil when there's none.
    package var installed: BundleVersion?
    /// The team that signed this copy, whose copies alone have their
    /// quarantine cleared; nil when it has no team.
    package var team: String?

    package init(source: URL, destination: URL, version: BundleVersion, installed: BundleVersion?, team: String?) {
        self.source = source
        self.destination = destination
        self.version = version
        self.installed = installed
        self.team = team
    }

    package var offer: InstallOffer { InstallFiles.offer(running: version, installed: installed) }
}

/// The installer's choices about paths and versions, kept apart from the
/// files so they're tested alone.
package enum InstallFiles {
    /// The folder the app installs itself in.
    package static let applications = URL(filePath: "/Applications", directoryHint: .isDirectory)

    /// Whether `bundle` sits directly in `applications`, after symlinks,
    /// so the installer leaves it be. One in a folder under it, or in
    /// `~/Applications`, doesn't count.
    package static func isInstalled(_ bundle: URL, in applications: URL = applications) -> Bool {
        let folder = bundle.standardizedFileURL.resolvingSymlinksInPath().deletingLastPathComponent()
        let target = applications.standardizedFileURL.resolvingSymlinksInPath()
        return folder.path(percentEncoded: false).trimmingSlash == target.path(percentEncoded: false).trimmingSlash
    }

    /// Where a bundle named `name` goes in `applications`: by the app's
    /// name, not the file's, so a download renamed `Humata 2.app` still
    /// replaces `Humata.app`.
    package static func destination(named name: String, in applications: URL = applications) -> URL {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let file = (trimmed.isEmpty ? "Humata" : trimmed) + ".app"
        return applications.appending(path: file)
    }

    /// Install unless the copy there is as new as this one or newer.
    package static func offer(running: BundleVersion, installed: BundleVersion?) -> InstallOffer {
        guard let installed, installed >= running else { return .install(replacing: installed) }
        return .openInstalled(installed)
    }
}

extension String {
    fileprivate var trimmingSlash: String {
        count > 1 && hasSuffix("/") ? String(dropLast()) : self
    }
}
