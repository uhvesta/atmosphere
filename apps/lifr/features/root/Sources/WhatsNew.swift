import ComposableArchitecture
import SwiftUI

extension SharedKey where Self == AppStorageKey<Int>.Default {
    /// The last release whose What's New this device showed, or 0 before
    /// the first launch finished.
    package static var seenRelease: Self {
        Self[.appStorage("lifrSeenRelease"), default: 0]
    }
}

/// What changed in this release, shown once after an update.
package enum WhatsNew {
    /// Raise this with each release whose notes below are new. 1 was the
    /// release before any notes.
    package static let release = 2

    struct Note {
        var symbol: String
        var title: LocalizedStringResource
        var detail: LocalizedStringResource
    }

    static let notes: [Note] = [
        Note(
            symbol: "circle.dashed", title: "Statuses in one order",
            detail: "The status picker always lists Done, In Progress, To Do, Procrastinated, then Won't Do."),
        Note(
            symbol: "icloud.slash", title: "Sync says what's waiting",
            detail: "Offline, Settings counts the changes waiting to send, and they go as soon as you're back."),
        Note(
            symbol: "accessibility", title: "More with VoiceOver",
            detail: "Move a row up or down, reschedule it, trash it or set its status by name."),
        Note(
            symbol: "trash", title: "Delete all data",
            detail: "Settings can now delete everything, here and in iCloud, once you type the word."),
    ]
}

/// The release's notes, with one button to carry on.
struct WhatsNewView: View {
    let dismiss: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            ScrollView {
                VStack(alignment: .leading, spacing: 24) {
                    Text("What's New in Lifr")
                        .font(.largeTitle.bold())
                        .frame(maxWidth: .infinity, alignment: .center)
                        .multilineTextAlignment(.center)
                        .padding(.bottom, 8)
                    ForEach(WhatsNew.notes, id: \.symbol) { note in
                        HStack(alignment: .top, spacing: 16) {
                            Image(systemName: note.symbol)
                                .font(.title2)
                                .foregroundStyle(.tint)
                                .frame(width: 36)
                                .accessibilityHidden(true)
                            VStack(alignment: .leading, spacing: 4) {
                                Text(note.title).font(.headline)
                                Text(note.detail).foregroundStyle(.secondary)
                            }
                        }
                        .accessibilityElement(children: .combine)
                    }
                }
                .frame(maxWidth: 520)
                .padding(24)
                .frame(maxWidth: .infinity)
            }
            .scrollBounceBehavior(.basedOnSize)
            Button(action: dismiss) {
                Text("Continue").frame(maxWidth: 360)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .keyboardShortcut(.defaultAction)
            .padding(24)
        }
        #if os(macOS)
            .frame(minWidth: 460, minHeight: 480)
        #endif
    }
}
