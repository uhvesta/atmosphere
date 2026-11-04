import Foundation
import LifrCore
import SwiftUI

/// A message across the top of a screen about something the app did on its
/// own, such as reopening a parent, with Undo when it can be undone.
package struct Banner: Identifiable, Hashable, Sendable {
    package var id: String { message }
    package var message: String
    /// The change Undo reverts, or nil when there's nothing to undo.
    package var undo: Change?

    package init(message: String, undo: Change? = nil) {
        self.message = message
        self.undo = undo
    }

    /// A new or reopened Sub task moved its closed parent back to In
    /// Progress.
    package static func reopened(_ parent: LifrTask, by change: Change) -> Banner {
        Banner(
            message: "\"\(parent.title)\" is In Progress again, because a closed task can't have open Sub tasks.",
            undo: change)
    }

    /// What reconciling a sync import fixed, or nil for fixes that don't
    /// need explaining.
    package init?(_ fix: Fix, in workspace: Workspace) {
        guard case .reopenedParent(let id) = fix, let parent = workspace.tasks[id] else { return nil }
        let open = workspace.openSubtasks(of: id).max { $0.createdAt < $1.createdAt }
        let closedBy = workspace.history.last {
            $0.taskID == id && $0.field == .status
                && ($0.newValue == TaskStatus.done.rawValue
                    || $0.newValue == TaskStatus.wontDo.rawValue)
        }
        let addedOn = open.flatMap { subtask in
            workspace.history.first { $0.taskID == subtask.id && $0.kind == .created }?.actor.device
        }
        if let open, let addedOn, let closedBy {
            let status = TaskStatus(rawValue: closedBy.newValue ?? "")?.title ?? "closed"
            message =
                "\"\(parent.title)\" reopened because \"\(open.title)\" was added on \(addedOn) "
                + "while this task was \(status) on \(closedBy.actor.device)."
        } else {
            message = "\"\(parent.title)\" reopened because it has an open Sub task."
        }
        undo = nil
    }
}

/// Shows a banner with its Undo and dismiss buttons.
package struct BannerView: View {
    package var banner: Banner
    package var undo: () -> Void
    package var dismiss: () -> Void

    package init(_ banner: Banner, undo: @escaping () -> Void, dismiss: @escaping () -> Void) {
        self.banner = banner
        self.undo = undo
        self.dismiss = dismiss
    }

    package var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            Image(systemName: "arrow.uturn.backward.circle")
                .accessibilityHidden(true)
            Text(banner.message)
                .frame(maxWidth: .infinity, alignment: .leading)
            if banner.undo != nil {
                Button("Undo", action: undo)
                    .bold()
            }
            Button(action: dismiss) {
                Image(systemName: "xmark")
            }
            .accessibilityLabel("Dismiss")
        }
        .font(.callout)
        .padding(12)
        .background(.thinMaterial, in: RoundedRectangle(cornerRadius: 12))
        .padding(.horizontal)
        .accessibilityElement(children: .contain)
    }
}
