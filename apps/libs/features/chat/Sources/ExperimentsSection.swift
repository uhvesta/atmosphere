import SwiftUI

/// Settings' section of the chat's experiments, kept on this device and off
/// until turned on: each reply's speed, what the model was sent, and a
/// second model's answer beside the first. The app's settings keep them,
/// with `.chatShowsSpeed`, `.chatShowsPrompt` and `.chatCompares`.
public struct ChatExperimentsSection: View {
    @Binding private var showsSpeed: Bool
    @Binding private var showsPrompt: Bool
    @Binding private var compares: Bool

    public init(showsSpeed: Binding<Bool>, showsPrompt: Binding<Bool>, compares: Binding<Bool>) {
        _showsSpeed = showsSpeed
        _showsPrompt = showsPrompt
        _compares = compares
    }

    public var body: some View {
        Section {
            Toggle(isOn: $showsSpeed) {
                Text("Speed")
                Text("Under each reply: its model, tokens a second, time to first token, tokens in and out.")
            }
            Toggle(isOn: $showsPrompt) {
                Text("Prompt")
                Text("A button under each reply shows exactly what the model was sent.")
            }
            Toggle(isOn: $compares) {
                Text("Compare")
                Text("Ask a second model too, see both answers and keep one.")
            }
        } header: {
            Text("Experiments")
        } footer: {
            Text("Apple's model doesn't count tokens, so its counts are guessed from the text and marked ≈.")
        }
    }
}
