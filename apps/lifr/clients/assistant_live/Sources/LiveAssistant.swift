import Foundation
import LifrAPI
import LifrAssistantClient
import LifrModelsClient

extension AssistantClient {
    /// The assistant on the model `runtime` returns, asked again for each
    /// request so a change of model applies to the next message. Nothing
    /// leaves the device.
    package static func live(runtime: @escaping @Sendable () -> any ModelRuntime) -> AssistantClient {
        AssistantClient(
            availability: { AssistantAvailability(runtime().status) },
            reply: { request in LiveAssistant.reply(to: request, on: runtime()) },
            draftSkill: { description in try await LiveAssistant.draftSkill(description, on: runtime()) },
            suggestTitle: { messages in try await LiveAssistant.suggestTitle(messages, on: runtime()) },
            prewarm: {
                let model = runtime()
                if model.status == .available { model.prewarm() }
            })
    }
}

extension AssistantAvailability {
    init(_ status: ModelStatus) {
        switch status {
        case .available: self = .available
        case .systemTooOld: self = .systemTooOld
        case .deviceNotEligible: self = .deviceNotEligible
        case .notEnabled: self = .notEnabled
        case .modelNotReady: self = .modelNotReady
        }
    }
}

extension AssistantError {
    init(_ error: ModelRuntimeError) {
        switch error {
        case .unavailable(let status): self = .unavailable(AssistantAvailability(status))
        case .exceededContext: self = .tooLong(saved: false)
        case .declined: self = .declined
        case .busy: self = .busy
        case .simulatorHost(let reason): self = .simulatorHost(reason)
        case .decodingFailure, .cancelled, .failed: self = .failed(error.description)
        }
    }
}

/// What the assistant asks the model, and what it does when the answer
/// doesn't come back as asked. The same for every model.
enum LiveAssistant {
    static func reply(to request: AssistantRequest, on model: any ModelRuntime) -> AssistantReplies {
        AssistantReplies { continuation in
            let task = Task {
                let availability = AssistantAvailability(model.status)
                guard availability == .available else {
                    continuation.finish(throwing: AssistantError.unavailable(availability))
                    return
                }
                let tools = request.tools()
                let before = tools.workspace
                do {
                    // Start with as much as the estimate says fits, and
                    // send less each time the model says it's too long,
                    // unless it already saved changes: sending again
                    // could make them twice.
                    let sizes = AssistantPrompt.sizes(request, budget: model.budget)
                    guard let smallest = sizes.last else { throw AssistantError.tooLong(saved: false) }
                    for size in sizes {
                        do {
                            try await answer(
                                request, size, tools, model, saved: { tools.workspace != before }, continuation)
                            return
                        } catch ModelRuntimeError.exceededContext {
                            let saved = tools.workspace != before
                            if saved || size == smallest { throw AssistantError.tooLong(saved: saved) }
                        }
                    }
                } catch let error as AssistantError {
                    continuation.finish(throwing: error)
                } catch ModelRuntimeError.cancelled {
                    continuation.finish()
                } catch let error as ModelRuntimeError {
                    continuation.finish(throwing: AssistantError(error))
                } catch is CancellationError {
                    continuation.finish()
                } catch {
                    continuation.finish(throwing: AssistantError.failed(error.localizedDescription))
                }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    /// One answer at `size`. The model sometimes writes an answer that
    /// doesn't fit the schema: ask again once, then for plain text, unless
    /// it already saved changes.
    private static func answer(
        _ request: AssistantRequest, _ size: AssistantPrompt.Size, _ tools: AssistantTools, _ model: any ModelRuntime,
        saved: () -> Bool, _ continuation: AssistantReplies.Continuation
    ) async throws {
        tools.resultBudget = size.toolResultBudget
        for attempt in 1...2 {
            do {
                try await structured(request, size, tools, model, continuation)
                return
            } catch ModelRuntimeError.decodingFailure {
                guard !saved() else { throw AssistantError.unreadable(saved: true) }
                if attempt == 2 { break }
            }
        }
        do {
            try await plain(request, size, model, continuation)
        } catch ModelRuntimeError.decodingFailure {
            throw AssistantError.unreadable(saved: false)
        }
    }

    /// One answer as a message and maybe a plan, with the tools.
    private static func structured(
        _ request: AssistantRequest, _ size: AssistantPrompt.Size, _ tools: AssistantTools, _ model: any ModelRuntime,
        _ continuation: AssistantReplies.Continuation
    ) async throws {
        let allowed = AssistantPrompt.tools(request, size: size)
        let turn = ModelTurn(
            instructions: AssistantPrompt.instructions(request, size: size),
            prompt: AssistantPrompt.prompt(request, size: size),
            tools: AssistantTool.allCases.filter(allowed.contains).map { $0.definition(tools) },
            schema: Shapes.turn, schemaInPrompt: size.schemaInPrompt)
        let draftsPlans = request.draftsPlans
        let answer = try await model.respond(to: turn) { partial in
            continuation.yield(AssistantPrompt.reply(from: partial, isComplete: false, draftsPlans: draftsPlans))
        }
        continuation.yield(AssistantPrompt.reply(from: answer, isComplete: true, draftsPlans: draftsPlans))
        continuation.finish()
    }

    /// One answer as plain text, without tools or a plan: what's left when
    /// the structured answer can't be read.
    private static func plain(
        _ request: AssistantRequest, _ size: AssistantPrompt.Size, _ model: any ModelRuntime,
        _ continuation: AssistantReplies.Continuation
    ) async throws {
        let turn = ModelTurn(
            instructions: AssistantPrompt.plainInstructions(request),
            prompt: AssistantPrompt.prompt(request, size: size))
        let answer = try await model.respond(to: turn) { partial in
            continuation.yield(AssistantReply(text: partial.plainText, isComplete: false))
        }
        continuation.yield(AssistantReply(text: answer.plainText, isComplete: true))
        continuation.finish()
    }

    static func draftSkill(_ description: String, on model: any ModelRuntime) async throws -> SkillDraft {
        let availability = AssistantAvailability(model.status)
        guard availability == .available else { throw AssistantError.unavailable(availability) }
        do {
            let turn = ModelTurn(
                instructions: AssistantPrompt.skillInstructions, prompt: description, schema: Shapes.skill)
            let answer = try await model.respond(to: turn) { _ in }
            guard let draft = AssistantPrompt.skillDraft(from: answer) else {
                throw AssistantError.failed("it didn't write any instructions")
            }
            return draft
        } catch let error as ModelRuntimeError {
            throw AssistantError(error)
        }
    }

    static func suggestTitle(_ messages: [ChatMessage], on model: any ModelRuntime) async throws -> String {
        let availability = AssistantAvailability(model.status)
        guard availability == .available else { throw AssistantError.unavailable(availability) }
        do {
            let turn = ModelTurn(
                instructions: AssistantPrompt.titleInstructions, prompt: AssistantPrompt.titlePrompt(messages))
            let answer = try await model.respond(to: turn) { _ in }
            guard let title = AssistantPrompt.title(from: answer.plainText) else {
                throw AssistantError.failed("it didn't suggest a title")
            }
            return title
        } catch let error as ModelRuntimeError {
            throw AssistantError(error)
        }
    }
}

extension JSONValue {
    /// The text of a plain answer.
    fileprivate var plainText: String {
        if case .string(let text) = self { text } else { "" }
    }
}
