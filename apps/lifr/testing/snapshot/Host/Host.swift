import SwiftUI

/// An empty app for flow snapshot tests to run in: they need a window scene,
/// which only an app has, for sheets to present and screens to draw.
@main
struct FlowHost: App {
    var body: some Scene {
        WindowGroup { Color.clear }
    }
}
