import Foundation
import LifrGlance
import SwiftUI
import UniformTypeIdentifiers

#if os(iOS)
    import UIKit

    /// The iPhone share sheet's Lifr page.
    @objc(ShareController)
    final class ShareController: UIViewController {
        override func viewDidLoad() {
            super.viewDidLoad()
            Task { @MainActor in
                let shared = await Shared(from: extensionContext)
                let host = UIHostingController(rootView: ShareRoot(shared: shared, finish: finish))
                addChild(host)
                host.view.frame = view.bounds
                host.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
                view.addSubview(host.view)
                host.didMove(toParent: self)
            }
        }

        private func finish(_ added: Bool) {
            if added {
                extensionContext?.completeRequest(returningItems: nil)
            } else {
                extensionContext?.cancelRequest(withError: CocoaError(.userCancelled))
            }
        }
    }
#else
    import AppKit

    /// The Mac share menu's Lifr sheet.
    @objc(ShareController)
    final class ShareController: NSViewController {
        override func loadView() {
            view = NSView(frame: NSRect(x: 0, y: 0, width: 420, height: 360))
        }

        override func viewDidLoad() {
            super.viewDidLoad()
            Task { @MainActor in
                let shared = await Shared(from: extensionContext)
                let host = NSHostingView(rootView: ShareRoot(shared: shared, finish: finish))
                host.frame = view.bounds
                host.autoresizingMask = [.width, .height]
                view.addSubview(host)
            }
        }

        private func finish(_ added: Bool) {
            if added {
                extensionContext?.completeRequest(returningItems: nil)
            } else {
                extensionContext?.cancelRequest(withError: CocoaError(.userCancelled))
            }
        }
    }
#endif

/// The form, and the error when the task can't be kept.
private struct ShareRoot: View {
    var shared: Shared
    var finish: (Bool) -> Void

    @State private var error: SharedTasksError?

    var body: some View {
        ShareView(title: shared.title, details: shared.details, avenues: Self.avenues) { task in
            guard let task else { return finish(false) }
            do throws(SharedTasksError) {
                guard let file = SharedTasksFile.shared else { throw SharedTasksError.noAppGroup }
                try file.append(task)
                finish(true)
            } catch {
                self.error = error
            }
        }
        .alert(
            "Couldn't Add the Task",
            isPresented: Binding(get: { error != nil }, set: { if !$0 { error = nil } }),
            presenting: error
        ) { _ in
            Button("OK") { error = nil }
        } message: { error in
            Text(error.description)
        }
    }

    /// The open Avenues as Lifr last wrote them for the widgets, or none
    /// before it has.
    private static var avenues: [ShareView.Choice] {
        let glance = try? GlanceFile.shared?.read()
        return (glance?.avenues ?? []).map { ShareView.Choice(id: $0.id, name: $0.name, symbol: $0.symbol) }
    }
}

extension Shared {
    /// What the share sheet was given: the first item's title and text,
    /// its first link and its first plain text.
    @MainActor
    init(from context: NSExtensionContext?) async {
        let item = context?.inputItems.compactMap { $0 as? NSExtensionItem }.first
        let providers = item?.attachments ?? []
        var url: URL?
        var text = item?.attributedContentText?.string
        for provider in providers {
            if url == nil, provider.hasItemConformingToTypeIdentifier(UTType.url.identifier) {
                url = try? await provider.loadItem(forTypeIdentifier: UTType.url.identifier) as? URL
            } else if text?.isEmpty ?? true,
                provider.hasItemConformingToTypeIdentifier(UTType.plainText.identifier)
            {
                text = try? await provider.loadItem(forTypeIdentifier: UTType.plainText.identifier) as? String
            }
        }
        self.init(title: item?.attributedTitle?.string, text: text, url: url)
    }
}
