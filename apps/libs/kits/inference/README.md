# InferenceKit

The models an app runs on the device: Apple's on-device model, and models
the person downloads from Hugging Face. Every model answers through one
`ModelRuntime`, so the assistant sends and retries the same way whichever
model it uses.

## Targets

| Target | What it has |
| --- | --- |
| `//apps/libs/kits/inference` | `ModelRuntime`, context budgets, the catalog, Hugging Face links and hashes, which model each use runs on, the downloads in progress, and `ScriptedRuntime` for tests and previews. |
| `//apps/libs/kits/inference/live` | Apple's model through Foundation Models, downloaded models through `ModelHost`, and `ModelLibrary`, which downloads, verifies, stores and deletes models. |
| `//apps/libs/kits/inference/litert` | LiteRT-LM on the GPU, through Google's prebuilt framework. |

Features depend on the first target. Only the app's entry point uses the
other two. All three need Xcode, so build with `--config=xcode`.

## Use it

```swift
import InferenceKit

enum NoteUse: String, ModelUseCase {
    case chat, naming
    var standard: ModelID? { self == .chat ? .apple : nil }
    func model(in selection: [Self: ModelID]) -> ModelID {
        selection[self] ?? standard ?? Self.chat.model(in: selection)
    }
}

let turn = ModelTurn(instructions: "You tidy notes.", prompt: note.body, tools: tools)
let answer = try await runtime.respond(to: turn) { partial in show(partial) }

// In tests:
let runtime = ScriptedRuntime(steps: [{ _, _ in "Tidied." }])
```

## What it gives you

- `ModelRuntime` reports a `status`, a `budget` of tokens for reading and
  writing, and the `inputs` it can read besides text. `respond(to:partial:)`
  streams the answer as JSON: a string, or a value that follows the turn's
  schema. It throws `ModelRuntimeError`, which says what went wrong. For example,
  the turn didn't fit the context window, the model was busy or declined,
  it couldn't read an attachment, or the app unloaded it mid-answer.
- `ModelTurn` holds instructions, the prompt, tools, an optional schema,
  attachments, and a callback for the model's thinking. `ModelThinking`
  records how long the model thought and whether it ran on the GPU or the
  CPU.
- `ModelSpec` and `ModelCatalog` describe the models you can download and
  their files. `ModelLink` reads a Hugging Face link, and `FileDigest`
  checks each file's SHA-256.
- `ModelSelection` remembers which model the person picked for each
  `ModelUseCase`.
- `ModelLibrary` (in live) downloads only from Hugging Face, in the
  background. It checks each file against its hash, keeps models in
  Application Support, and resumes unfinished downloads. `DownloadBoard`
  streams their progress.
- `ModelHost` (in live) keeps one downloaded model loaded. It unloads the
  model when the system runs low on memory. On iOS, it moves the model off
  the GPU when the app goes to the background. Before loading a model, it
  checks with `MemoryPlan` that the model fits.
- Foundation Models is weakly linked, so apps still run on iOS 18 and
  macOS 15. There, Apple's model reports that the system is too old.

Run the tests with `bazel test --config=xcode //apps/libs/kits/inference:tests`
and `//apps/libs/kits/inference/live:tests`, on macOS. The live tests run
downloads against a stub of Hugging Face and turns against a fake engine.
A real model needs a device, so `litert` is checked by building it and by
hand.
