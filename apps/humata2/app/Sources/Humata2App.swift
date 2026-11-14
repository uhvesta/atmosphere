import Humata2Models
import SwiftUI

/// Humata 2: the window lays out what the Rust core holds and sends it what
/// the person does. It reaches the daemon Humata runs.
@main
struct Humata2App: App {
    @State private var model = SessionsModel()

    var body: some Scene {
        WindowGroup("Humata 2") {
            SessionsScreen(model: model)
                .frame(minWidth: 640, minHeight: 400)
        }
    }
}
