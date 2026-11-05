import Foundation

/// What someone saw go wrong, with the steps that led to it when they
/// recorded them.
package struct BugReport: Identifiable, Hashable, Codable, Sendable {
    package var id: UUID
    package var createdAt: Date
    /// What happened, in the person's words.
    package var happened: String
    /// What they expected instead.
    package var expected: String
    /// Whether the app's text and the screenshots are kept. Off, each piece
    /// of text is a placeholder and there are no screenshots.
    package var includesPersonal: Bool
    package var device: DeviceInfo
    /// Recording: when it started and the app's state then. Nil for a report
    /// made without one.
    package var recording: Recording?

    package init(
        id: UUID, createdAt: Date, happened: String = "", expected: String = "", includesPersonal: Bool = false,
        device: DeviceInfo, recording: Recording? = nil
    ) {
        self.id = id
        self.createdAt = createdAt
        self.happened = happened
        self.expected = expected
        self.includesPersonal = includesPersonal
        self.device = device
        self.recording = recording
    }

    /// The steps someone recorded.
    package struct Recording: Hashable, Codable, Sendable {
        package var startedAt: Date
        /// The app's state when recording started.
        package var start: String
        package var steps: [Step]
        package var screenshots: [Screenshot]
        /// How many steps were left out once the recording was full.
        package var dropped: Int

        package init(
            startedAt: Date, start: String, steps: [Step] = [], screenshots: [Screenshot] = [], dropped: Int = 0
        ) {
            self.startedAt = startedAt
            self.start = start
            self.steps = steps
            self.screenshots = screenshots
            self.dropped = dropped
        }
    }

    /// One action and what it changed.
    package struct Step: Hashable, Codable, Sendable {
        /// Seconds since recording started.
        package var offset: TimeInterval
        package var action: String
        /// The change to the app's state, as a diff. Nil when nothing changed.
        package var changes: String?

        package init(offset: TimeInterval, action: String, changes: String? = nil) {
            self.offset = offset
            self.action = action
            self.changes = changes
        }
    }

    /// The screen after a step.
    package struct Screenshot: Hashable, Codable, Sendable {
        package var offset: TimeInterval
        /// How many steps had happened when it was taken.
        package var afterSteps: Int
        /// A JPEG.
        package var image: Data

        package init(offset: TimeInterval, afterSteps: Int, image: Data) {
            self.offset = offset
            self.afterSteps = afterSteps
            self.image = image
        }
    }
}

/// The device and app a report came from.
package struct DeviceInfo: Hashable, Codable, Sendable {
    /// For example "iPhone18,1" or "Mac16,10".
    package var model: String
    package var system: String
    package var appVersion: String
    package var language: String
    package var timeZone: String
    /// The model the assistant runs on.
    package var assistantModel: String

    package init(
        model: String, system: String, appVersion: String, language: String, timeZone: String,
        assistantModel: String
    ) {
        self.model = model
        self.system = system
        self.appVersion = appVersion
        self.language = language
        self.timeZone = timeZone
        self.assistantModel = assistantModel
    }
}

extension BugReport {
    /// The report as plain text, for the clipboard. Screenshots are counted,
    /// not included.
    package var text: String {
        var lines = [
            "# Lifr bug report",
            "",
            "Reported: \(createdAt.formatted(.iso8601))",
            "Device: \(device.model), \(device.system)",
            "Lifr: \(device.appVersion)",
            "Language: \(device.language), time zone: \(device.timeZone)",
            "Assistant model: \(device.assistantModel)",
            "Text and screenshots: \(includesPersonal ? "included" : "replaced with placeholders")",
            "",
            "## What happened",
            "",
            happened.isEmpty ? "-" : happened,
            "",
            "## What I expected",
            "",
            expected.isEmpty ? "-" : expected,
        ]
        guard let recording else { return lines.joined(separator: "\n") + "\n" }
        lines += [
            "",
            "## Steps",
            "",
            "Recorded from \(recording.startedAt.formatted(.iso8601)); \(recording.screenshots.count) screenshots.",
        ]
        if recording.dropped > 0 {
            lines.append("\(recording.dropped) later steps were left out once the recording was full.")
        }
        for (index, step) in recording.steps.enumerated() {
            lines += ["", "\(index + 1). +\(String(format: "%.2f", step.offset))s \(step.action)"]
            if let changes = step.changes {
                lines += ["```diff", changes, "```"]
            }
        }
        lines += ["", "## Starting state", "", "```", recording.start, "```"]
        return lines.joined(separator: "\n") + "\n"
    }
}
