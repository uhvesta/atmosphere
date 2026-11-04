import LifrCore
import SwiftUI

@main
struct LifrApp: App {
    var body: some Scene {
        WindowGroup {
            ContentView()
        }
    }
}

struct ContentView: View {
    var body: some View {
        Text(Greeting.hello(from: client))
            .font(.title)
            .padding()
    }

    private var client: String {
        #if os(iOS)
        "iOS"
        #else
        "macOS"
        #endif
    }
}

#Preview {
    ContentView()
}
