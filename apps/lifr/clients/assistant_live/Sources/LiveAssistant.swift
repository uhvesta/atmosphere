import Foundation
import LifrAPI
import LifrAssistantClient
import LifrCore
import LifrModelsClient
import Synchronization

extension AssistantClient {
    /// The assistant on the model `runtime` returns for each use, asked
    /// again for each request so a change of model applies to the next
    /// message. Nothing leaves the device.
    package static func live(runtime: @escaping @Sendable (ModelUse) -> any ModelRuntime) -> AssistantClient {
        AssistantClient(
            availability: { AssistantAvailability(runtime(.assistant).status) },
            reply: { request in LiveAssistant.reply(to: request, on: runtime(.assistant)) },
            draftSkill: { description in try await LiveAssistant.draftSkill(description, on: runtime(.skills)) },
            suggestTitle: { messages in try await LiveAssistant.suggestTitle(messages, on: runtime(.titles)) },
            prewarm: {
                let model = runtime(.assistant)
                if model.status == .available { model.prewarm() }
            },
            inputs: { runtime(.assistant).inputs })
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
        case .notDownloaded: self = .notDownloaded
        case .notEnoughMemory(let needed, let available):
            self = .notEnoughMemory(needed: needed, available: available)
        }
    }
}

extension AssistantInterruption {
    init(_ reason: ModelInterruption) {
        switch reason {
        case .memory: self = .memory
        case .background: self = .background
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
        case .interrupted(let reason): self = .interrupted(AssistantInterruption(reason), saved: false)
        case .unsupportedInput(let kind): self = .cantRead(kind)
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
                let thoughts = Thoughts()
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
                                request, size, tools, model, thoughts, saved: { tools.workspace != before },
                                continuation)
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
                } catch ModelRuntimeError.interrupted(let reason) {
                    continuation.finish(
                        throwing: AssistantError.interrupted(
                            AssistantInterruption(reason), saved: tools.workspace != before))
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
        _ thoughts: Thoughts, saved: () -> Bool, _ continuation: AssistantReplies.Continuation
    ) async throws {
        tools.resultBudget = size.toolResultBudget
        var forgot = false
        for attempt in 1...2 {
            do {
                try await structured(request, size, tools, model, thoughts, forgot: forgot, continuation)
                return
            } catch ModelRuntimeError.decodingFailure {
                guard !saved() else { throw AssistantError.unreadable(saved: true) }
                if attempt == 2 { break }
            } catch is ForgotTasks {
                // Asked the same way, a model that answers the same each
                // time would forget them again.
                forgot = true
                if attempt == 2 { break }
            }
        }
        do {
            try await plain(request, size, model, thoughts, continuation)
        } catch ModelRuntimeError.decodingFailure {
            throw AssistantError.unreadable(saved: false)
        }
    }

    /// One answer as a message and maybe a plan, with the tools.
    private static func structured(
        _ request: AssistantRequest, _ size: AssistantPrompt.Size, _ tools: AssistantTools, _ model: any ModelRuntime,
        _ thoughts: Thoughts, forgot: Bool, _ continuation: AssistantReplies.Continuation
    ) async throws {
        let allowed = AssistantPrompt.tools(request, size: size)
        let changeable = AssistantPrompt.changeable(request)
        let draftTitles = AssistantPrompt.draftTitles(request)
        let asksForPlan = request.draftsPlans && AssistantPrompt.asksForPlan(request)
        let asksToAdd = AssistantPrompt.asksToAdd(request)
        let mustPlan = asksForPlan && (draftTitles.isEmpty || asksToAdd)
        var turn = ModelTurn(
            instructions: AssistantPrompt.instructions(request, size: size),
            prompt: AssistantPrompt.prompt(request, size: size) + (forgot ? Self.forgotTasks : ""),
            tools: AssistantTool.allCases.filter(allowed.contains).compactMap { $0.definition(tools) },
            schema: Shapes.turn(
                changes: changeable.map(\.code), places: request.lookup.places.flatMap(\.choices),
                draftTitles: draftTitles, mustChange: AssistantPrompt.asksForChange(request),
                mustChangeDraft: asksForPlan && !asksToAdd,
                mustPlan: mustPlan),
            schemaInPrompt: size.schemaInPrompt,
            attachments: request.attachments)
        let draftsPlans = request.draftsPlans
        turn.thinking = thoughts.attempt(continuation)
        let answer = try await model.respond(to: turn) { partial in
            let reply = AssistantPrompt.reply(from: partial, isComplete: false, draftsPlans: draftsPlans)
            thoughts.yield(reply, continuation)
        }
        var changed: [LifrTask] = []
        var refused: [TaskChangeError] = []
        let codes = Set(changeable.map(\.code))
        if !codes.isEmpty {
            for var change in AssistantPrompt.changes(from: answer) where codes.contains(change.code) {
                if let old = changeable.first(where: { $0.code == change.code })?.title {
                    change.title = AssistantPrompt.renamed(old, to: change.title, request)
                }
                change.due = AssistantPrompt.dated(change.due, request)
                do {
                    changed.append(try await tools.change(change))
                } catch {
                    refused.append(error)
                }
            }
        }
        var reply = AssistantPrompt.reply(from: answer, isComplete: true, draftsPlans: draftsPlans)
        // Asked to add something, a small model sometimes writes back an
        // Avenue that's there already and none of the tasks, and says it
        // added them. Told so, it usually writes them.
        if mustPlan, changed.isEmpty, refused.isEmpty, reply.plan?.leavingOut(request.workspace).isEmpty ?? true {
            throw ForgotTasks()
        }
        reply.plan = reply.plan.map {
            AssistantPrompt.withoutRestatedGoals(AssistantPrompt.withAskedProjects($0, request), request)
        }
        if let draft = request.draft, !draftTitles.isEmpty {
            reply.plan = AssistantPrompt.followUp(
                reply.plan, changes: AssistantPrompt.draftChanges(from: answer, titles: draftTitles, request),
                to: draft)
        }
        reply.thinking = thoughts.value
        continuation.yield(AssistantPrompt.reply(reply, changed: changed, refused: refused))
        continuation.finish()
    }

    /// Added to the prompt when asking again after an answer that added
    /// none of what the person asked for.
    private static let forgotTasks = """


        Your last answer added none of what the person asked for. Put each new task they asked for in the \
        plan's "tasks", even when it goes in an Avenue that's there already.
        """

    /// An answer that was asked for new things and added none of them.
    private struct ForgotTasks: Error {}

    /// One answer as plain text, without tools or a plan: what's left when
    /// the structured answer can't be read.
    private static func plain(
        _ request: AssistantRequest, _ size: AssistantPrompt.Size, _ model: any ModelRuntime, _ thoughts: Thoughts,
        _ continuation: AssistantReplies.Continuation
    ) async throws {
        var turn = ModelTurn(
            instructions: AssistantPrompt.plainInstructions(request),
            prompt: AssistantPrompt.prompt(request, size: size), attachments: request.attachments)
        turn.thinking = thoughts.attempt(continuation)
        let answer = try await model.respond(to: turn) { partial in
            thoughts.yield(AssistantReply(text: partial.plainText, isComplete: false), continuation)
        }
        continuation.yield(AssistantReply(text: answer.plainText, isComplete: true, thinking: thoughts.value))
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

/// What the model has thought for one request, over each time Lifr asked
/// it, and the reply written so far, so a reply as it's written carries
/// both.
final class Thoughts: Sendable {
    private struct State {
        /// The thinking of answers Lifr asked again after.
        var earlier: ModelThinking?
        var current: ModelThinking?
        var reply = AssistantReply(text: "")
    }

    private let state = Mutex(State())

    /// Everything thought so far.
    var value: ModelThinking? {
        state.withLock { Self.joined($0.earlier, $0.current) }
    }

    /// Starts an answer: the thinking of any before it stays, and what the
    /// model thinks now is added after it, sent on with the reply so far.
    func attempt(_ continuation: AssistantReplies.Continuation) -> @Sendable (ModelThinking) -> Void {
        state.withLock { state in
            state.earlier = Self.joined(state.earlier, state.current)
            state.current = nil
            state.reply = AssistantReply(text: "")
        }
        return { [self] thought in
            let reply = state.withLock { state in
                state.current = thought
                state.reply.thinking = Self.joined(state.earlier, thought)
                return state.reply
            }
            continuation.yield(reply)
        }
    }

    /// Sends on `reply`, a partial answer, with the thinking so far.
    func yield(_ reply: AssistantReply, _ continuation: AssistantReplies.Continuation) {
        let reply = state.withLock { state in
            state.reply = reply
            state.reply.thinking = Self.joined(state.earlier, state.current)
            return state.reply
        }
        continuation.yield(reply)
    }

    private static func joined(_ earlier: ModelThinking?, _ current: ModelThinking?) -> ModelThinking? {
        guard let earlier else { return current }
        return current.map(earlier.then) ?? earlier
    }
}

extension JSONValue {
    /// The text of a plain answer.
    fileprivate var plainText: String {
        if case .string(let text) = self { text } else { "" }
    }
}
