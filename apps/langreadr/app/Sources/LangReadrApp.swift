import ComposableArchitecture
import Foundation
import InferenceKitLive
import LangReadrCore
import LangReadrReading
import SwiftUI

/// LangReadr: read what you import in the language you're learning, with
/// the device's own model to translate it and answer questions about it.
@main
struct LangReadrApp: App {
    @State private var store = Store(initialState: LibraryFeature.State()) { LibraryFeature() }

    var body: some Scene {
        WindowGroup {
            LangReadrView(store: store)
        }
    }
}

/// The tutor on the device's model, explaining in the device's language.
extension TutorKey: DependencyKey {
    package static let liveValue = Tutor(
        runtime: AppleRuntime(), reader: Language(Locale.current.language.languageCode?.identifier ?? "en"))
}
