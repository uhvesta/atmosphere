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
            pickSkill: { messages, skills, drafting in
                try await LiveAssistant.pickSkill(messages, skills, drafting: drafting, on: runtime(.assistant))
            },
            suggestTitle: { messages in try await LiveAssistant.suggestTitle(messages, on: runtime(.titles)) },
            compact: { messages, earlier in
                try await LiveAssistant.compact(messages, earlier: earlier, on: runtime(.assistant))
            },
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
        case .exceededContext: self = .tooLong
        case .declined: self = .declined
        case .busy: self = .busy
        case .simulatorHost(let reason): self = .simulatorHost(reason)
        case .interrupted(let reason): self = .interrupted(AssistantInterruption(reason))
        case .unsupportedInput(let kind): self = .cantRead(kind)
        case .decodingFailure, .cancelled, .engineMissing, .failed: self = .failed(error.description)
        }
    }
}

/// What the assistant asks the model, and what it does when the answer
/// doesn't come back as asked. The same for every model.
enum LiveAssistant {
    static func reply(to request: AssistantRequest, on model: any ModelRuntime) -> AssistantReplies {
        var sized = request
        sized.historyBudget = AssistantPrompt.historyBudget(for: model.budget)
        let request = sized
        return AssistantReplies { continuation in
            let task = Task {
                let availability = AssistantAvailability(model.status)
                guard availability == .available else {
                    continuation.finish(throwing: AssistantError.unavailable(availability))
                    return
                }
                let tools = request.tools()
                let thoughts = Thoughts()
                do {
                    // Start with as much as the estimate says fits, and
                    // send less each time the model says it's too long.
                    let sizes = AssistantPrompt.sizes(request, budget: model.budget)
                    guard let smallest = sizes.last else { throw AssistantError.tooLong }
                    for size in sizes {
                        do {
                            try await answer(request, size, tools, model, thoughts, continuation)
                            return
                        } catch ModelRuntimeError.exceededContext {
                            if size == smallest { throw AssistantError.tooLong }
                        }
                    }
                } catch let error as AssistantError {
                    continuation.finish(throwing: error)
                } catch ModelRuntimeError.cancelled {
                    continuation.finish()
                } catch ModelRuntimeError.interrupted(let reason) {
                    continuation.finish(
                        throwing: AssistantError.interrupted(AssistantInterruption(reason)))
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
    /// doesn't fit the schema: ask again once, then for plain text.
    private static func answer(
        _ request: AssistantRequest, _ size: AssistantPrompt.Size, _ tools: AssistantTools, _ model: any ModelRuntime,
        _ thoughts: Thoughts, _ continuation: AssistantReplies.Continuation
    ) async throws {
        // Without a skill, the assistant just talks.
        guard request.skill != nil else {
            return try await plain(
                request, size, instructions: AssistantPrompt.chatInstructions(request), model, thoughts, continuation)
        }
        tools.resultBudget = size.toolResultBudget
        var forgot = false
        for attempt in 1...2 {
            do {
                try await structured(request, size, tools, model, thoughts, forgot: forgot, continuation)
                return
            } catch ModelRuntimeError.decodingFailure {
                if attempt == 2 { break }
            } catch is ForgotTasks {
                // Asked the same way, a model that answers the same each
                // time would forget them again.
                forgot = true
                if attempt == 2 { break }
            }
        }
        // Plain text has no tools, so it asks to change nothing.
        tools.discardProposal()
        do {
            try await plain(
                request, size, instructions: AssistantPrompt.plainInstructions(request), model, thoughts, continuation)
        } catch ModelRuntimeError.decodingFailure {
            throw AssistantError.unreadable
        }
    }

    /// One answer as a message and maybe a plan, with the tools.
    private static func structured(
        _ request: AssistantRequest, _ size: AssistantPrompt.Size, _ tools: AssistantTools, _ model: any ModelRuntime,
        _ thoughts: Thoughts, forgot: Bool, _ continuation: AssistantReplies.Continuation
    ) async throws {
        // What an earlier try asked to change is asked again, or not.
        tools.discardProposal()
        let allowed = AssistantPrompt.tools(request, size: size)
        let changeable = AssistantPrompt.changeable(request)
        let projects = AssistantPrompt.changeableProjects(request)
        let asksForChange = AssistantPrompt.asksForChange(request)
        let asksForProjectChange = AssistantPrompt.asksForProjectChange(request)
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
                draftTitles: draftTitles, projects: projects.map(\.place),
                mustChange: asksForChange && !asksForProjectChange,
                mustChangeDraft: asksForPlan && !asksToAdd,
                mustChangeProject: asksForProjectChange && !asksForChange,
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
        var changedProjects = 0
        var refusedProjects: [ProjectChangeError] = []
        for var change in AssistantPrompt.projectChanges(from: answer) {
            guard let known = projects.first(where: { $0.place.caseInsensitiveCompare(change.project) == .orderedSame })
            else { continue }
            change.name = AssistantPrompt.renamed(known.name, to: change.name, request)
            change.due = AssistantPrompt.dated(change.due, request)
            do {
                _ = try await tools.change(change)
                changedProjects += 1
            } catch {
                refusedProjects.append(error)
            }
        }
        var reply = AssistantPrompt.reply(from: answer, isComplete: true, draftsPlans: draftsPlans)
        // Asked to add something, a small model sometimes writes back an
        // Avenue that's there already and none of the tasks, and says it
        // added them. Told so, it usually writes them.
        if mustPlan, changed.isEmpty, refused.isEmpty, changedProjects == 0, refusedProjects.isEmpty,
            reply.plan?.leavingOut(request.workspace).isEmpty ?? true
        {
            throw ForgotTasks()
        }
        reply.plan = reply.plan.map {
            AssistantPrompt.withoutRepeatedTasks(
                AssistantPrompt.withoutRestatedGoals(AssistantPrompt.withAskedProjects($0, request), request))
        }
        if let draft = request.draft, !draftTitles.isEmpty {
            reply.plan = AssistantPrompt.followUp(
                reply.plan, changes: AssistantPrompt.draftChanges(from: answer, titles: draftTitles, request),
                to: draft)
        }
        reply.thinking = thoughts.value
        reply.leftOut = AssistantPrompt.leftOut(request, size: size)
        reply.proposal = tools.proposal
        continuation.yield(
            AssistantPrompt.reply(reply, changed: changed, refused: refused, refusedProjects: refusedProjects))
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

    /// One answer as plain text, without tools or a plan: the answer
    /// without a skill, and what's left when the structured answer can't
    /// be read.
    private static func plain(
        _ request: AssistantRequest, _ size: AssistantPrompt.Size, instructions: String, _ model: any ModelRuntime,
        _ thoughts: Thoughts, _ continuation: AssistantReplies.Continuation
    ) async throws {
        var turn = ModelTurn(
            instructions: instructions,
            prompt: AssistantPrompt.prompt(request, size: size), attachments: request.attachments)
        turn.thinking = thoughts.attempt(continuation)
        let answer = try await model.respond(to: turn) { partial in
            thoughts.yield(AssistantReply(text: partial.plainText, isComplete: false), continuation)
        }
        continuation.yield(
            AssistantReply(
                text: answer.plainText, isComplete: true, thinking: thoughts.value,
                leftOut: AssistantPrompt.leftOut(request, size: size)))
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

    static func pickSkill(
        _ messages: [ChatMessage], _ skills: [Skill], drafting: String? = nil, on model: any ModelRuntime
    ) async throws
        -> String?
    {
        guard !skills.isEmpty else { return nil }
        let availability = AssistantAvailability(model.status)
        guard availability == .available else { throw AssistantError.unavailable(availability) }
        do {
            let turn = ModelTurn(
                instructions: AssistantPrompt.pickInstructions(skills),
                prompt: AssistantPrompt.pickPrompt(messages, drafting: drafting),
                schema: Shapes.pick(skills.map(\.id)))
            let answer = try await model.respond(to: turn) { _ in }
            return AssistantPrompt.pickedSkill(from: answer, in: skills)
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

extension LiveAssistant {
    static func compact(_ messages: [ChatMessage], earlier: String?, on model: any ModelRuntime) async throws
        -> String
    {
        let availability = AssistantAvailability(model.status)
        guard availability == .available else { throw AssistantError.unavailable(availability) }
        do {
            let turn = ModelTurn(
                instructions: AssistantPrompt.summaryInstructions,
                prompt: AssistantPrompt.summaryPrompt(
                    messages, earlier: earlier, budget: AssistantPrompt.summaryInputBudget(for: model.budget)))
            let answer = try await model.respond(to: turn) { _ in }
            guard let summary = AssistantPrompt.summary(from: answer.plainText) else {
                throw AssistantError.failed("it didn't write a summary")
            }
            return summary
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
