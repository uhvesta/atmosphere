import ComposableArchitecture
import Foundation
import InferenceKit
import LangReadrCore
import LangReadrServices

@testable import LangReadrReading

let epoch = Date(timeIntervalSince1970: 0)

func spanish(_ text: String = "Hola amigo. Buenos días.") throws -> Book {
    try Importer.text(text, title: "Cuento").book(in: Language("es"), at: epoch)
}

func tutor(_ steps: [ScriptedRuntime.Step]) -> Tutor {
    Tutor(runtime: ScriptedRuntime(steps: steps), reader: Language("en"))
}

struct Missing: Error {}

extension Optional {
    func unwrap() throws -> Wrapped {
        guard let self else { throw Missing() }
        return self
    }
}

extension TestStore {
    /// Runs every effect to its end, and the actions they sent.
    func settle() async {
        await finish()
        await skipReceivedActions()
    }
}
