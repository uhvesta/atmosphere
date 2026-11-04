import Foundation
import LifrAPI
import LifrAssistantClient

#if canImport(FoundationModels)
    import FoundationModels
#endif

extension AssistantClient {
    /// The assistant on Apple's on-device model. Nothing leaves the device.
    /// Before iOS 26 and macOS 26, or without Apple Intelligence, it says
    /// why it can't run.
    package static func live() -> AssistantClient {
        #if canImport(FoundationModels)
            if #available(iOS 26, macOS 26, *) {
                return AssistantClient(
                    availability: { OnDeviceModel.availability },
                    reply: { request in OnDeviceModel.reply(to: request) })
            }
        #endif
        return AssistantClient(
            availability: { .systemTooOld },
            reply: { _ in
                AsyncThrowingStream { $0.finish(throwing: AssistantError.unavailable(.systemTooOld)) }
            })
    }
}

#if canImport(FoundationModels)
    @available(iOS 26, macOS 26, *)
    private enum OnDeviceModel {
        static var availability: AssistantAvailability {
            switch SystemLanguageModel.default.availability {
            case .available: .available
            case .unavailable(.deviceNotEligible): .deviceNotEligible
            case .unavailable(.appleIntelligenceNotEnabled): .notEnabled
            case .unavailable(.modelNotReady): .modelNotReady
            case .unavailable: .deviceNotEligible
            }
        }

        static func reply(to request: AssistantRequest) -> AsyncThrowingStream<AssistantReply, any Error> {
            AsyncThrowingStream { continuation in
                let task = Task {
                    let availability = Self.availability
                    guard availability == .available else {
                        continuation.finish(throwing: AssistantError.unavailable(availability))
                        return
                    }
                    let tools = request.tools
                    do {
                        let session = LanguageModelSession(
                            tools: [try FindTasks(tools), try GetTask(tools), try Overview(tools)],
                            instructions: AssistantPrompt.instructions(request))
                        let schema = try GenerationSchema(root: Schema.turn, dependencies: [])
                        let stream = session.streamResponse(to: AssistantPrompt.prompt(request), schema: schema)
                        var last = AssistantReply(text: "")
                        for try await snapshot in stream {
                            last = AssistantPrompt.reply(from: json(snapshot.rawContent), isComplete: false)
                            continuation.yield(last)
                        }
                        last.isComplete = true
                        continuation.yield(last)
                        continuation.finish()
                    } catch let error as LanguageModelSession.GenerationError {
                        continuation.finish(throwing: assistantError(error))
                    } catch is CancellationError {
                        continuation.finish()
                    } catch {
                        continuation.finish(throwing: AssistantError.failed(error.localizedDescription))
                    }
                }
                continuation.onTermination = { _ in task.cancel() }
            }
        }

        private static func assistantError(_ error: LanguageModelSession.GenerationError) -> AssistantError {
            switch error {
            case .exceededContextWindowSize: .tooLong
            case .guardrailViolation, .refusal: .declined
            default: .failed(error.localizedDescription)
            }
        }

        private static func json(_ content: GeneratedContent) -> JSONValue {
            switch content.kind {
            case .null: .null
            case .bool(let value): .bool(value)
            case .number(let value): .number(value)
            case .string(let value): .string(value)
            case .array(let items): .array(items.map(json))
            case .structure(let properties, _): .object(properties.mapValues(json))
            @unknown default: .null
            }
        }
    }

    /// The shape of each answer, in the order the model writes it: the
    /// message first, so it shows while the plan is still coming.
    @available(iOS 26, macOS 26, *)
    private enum Schema {
        static let text = DynamicGenerationSchema(type: String.self)

        static func list(_ item: DynamicGenerationSchema) -> DynamicGenerationSchema {
            DynamicGenerationSchema(arrayOf: item)
        }

        static func optional(_ name: String, _ description: String, _ schema: DynamicGenerationSchema = text)
            -> DynamicGenerationSchema.Property
        {
            .init(name: name, description: description, schema: schema, isOptional: true)
        }

        static let priority = DynamicGenerationSchema(
            name: "Priority", anyOf: ["none", "low", "medium", "high", "urgent"])

        static func task(sub: Bool) -> DynamicGenerationSchema {
            var properties: [DynamicGenerationSchema.Property] = [
                .init(name: "title", description: "What to do, short.", schema: text)
            ]
            if !sub {
                properties += [
                    optional("place", "An Avenue KEY, or KEY/Project name, from the list."),
                    optional("parent", "The code of an existing task to add this under, from findTasks."),
                ]
            }
            properties += [
                optional("details", "Notes, only if the person gave some."),
                optional("priority", "Only if the person said how important it is.", priority),
                optional("due", "When it's due: YYYY-MM-DD, today, tomorrow, friday, next friday, +3d, +2w or +1m."),
                optional("dueTime", "HH:MM, 24-hour, only with a due date."),
                optional("labels", "Label names.", list(text)),
                optional("remind", "Reminders: 1h before, 1d before, or YYYY-MM-DD HH:MM.", list(text)),
            ]
            if !sub {
                properties.append(optional("subtasks", "Steps of this task, one level deep.", list(task(sub: true))))
            }
            return DynamicGenerationSchema(name: sub ? "SubTask" : "Task", properties: properties)
        }

        static let project = DynamicGenerationSchema(
            name: "Project",
            properties: [
                .init(name: "name", description: "The goal's name.", schema: text),
                .init(name: "avenue", description: "The KEY of the Avenue it belongs to.", schema: text),
                optional("details", "What the goal is, if the person said."),
                optional("due", "When the goal is due."),
                optional("tasks", "The Project's tasks; they need no place.", list(task(sub: true))),
            ])

        static let avenue = DynamicGenerationSchema(
            name: "Avenue",
            properties: [
                .init(name: "key", description: "2 to 10 capital letters, such as HEALTH.", schema: text),
                .init(name: "name", description: "Its name, such as Health.", schema: text),
            ])

        static let plan = DynamicGenerationSchema(
            name: "Plan",
            properties: [
                optional("avenues", "New Avenues, only when the person agreed to one.", list(avenue)),
                optional("projects", "New Projects.", list(project)),
                optional("tasks", "New top-level tasks.", list(task(sub: false))),
            ])

        static let turn = DynamicGenerationSchema(
            name: "Turn",
            properties: [
                .init(
                    name: "message", description: "What you say to the person, one to three sentences.", schema: text),
                optional("plan", "The whole plan to add, when there is one.", plan),
            ])
    }

    // MARK: - Tools

    @available(iOS 26, macOS 26, *)
    private func schema(_ name: String, _ properties: [DynamicGenerationSchema.Property]) throws -> GenerationSchema {
        try GenerationSchema(root: DynamicGenerationSchema(name: name, properties: properties), dependencies: [])
    }

    @available(iOS 26, macOS 26, *)
    private func string(_ arguments: GeneratedContent, _ key: String) -> String? {
        guard case .structure(let properties, _) = arguments.kind, let value = properties[key],
            case .string(let text) = value.kind
        else { return nil }
        return text
    }

    @available(iOS 26, macOS 26, *)
    private struct FindTasks: Tool {
        let tools: AssistantTools
        let name = "findTasks"
        let description = "Search the person's tasks. Returns one line per task: code, title, status, place, due date."
        let parameters: GenerationSchema

        init(_ tools: AssistantTools) throws {
            self.tools = tools
            parameters = try schema(
                "FindTasks",
                [
                    .init(
                        name: "words", description: "Words in the title or details; empty for all.", schema: Schema.text
                    ),
                    Schema.optional("avenue", "Only in this Avenue KEY."),
                    Schema.optional(
                        "status", "Only with this status. Without one, open tasks.",
                        DynamicGenerationSchema(
                            name: "Status", anyOf: ["todo", "in_progress", "procrastinated", "done", "wont_do"])),
                    Schema.optional("dueBy", "Only due on or before this date, such as today or +7d."),
                ])
        }

        func call(arguments: GeneratedContent) async throws -> String {
            tools.findTasks(
                words: string(arguments, "words") ?? "", avenue: string(arguments, "avenue"),
                status: string(arguments, "status"), dueBy: string(arguments, "dueBy"))
        }
    }

    @available(iOS 26, macOS 26, *)
    private struct GetTask: Tool {
        let tools: AssistantTools
        let name = "getTask"
        let description = "One task by its code, with details, Sub tasks and comments."
        let parameters: GenerationSchema

        init(_ tools: AssistantTools) throws {
            self.tools = tools
            parameters = try schema(
                "GetTask", [.init(name: "code", description: "Such as HOME-0003.", schema: Schema.text)])
        }

        func call(arguments: GeneratedContent) async throws -> String {
            tools.getTask(code: string(arguments, "code") ?? "")
        }
    }

    @available(iOS 26, macOS 26, *)
    private struct Overview: Tool {
        let tools: AssistantTools
        let name = "overview"
        let description = "Open tasks per Avenue, and what's overdue, due today, due this week and procrastinated."
        let parameters: GenerationSchema

        init(_ tools: AssistantTools) throws {
            self.tools = tools
            parameters = try schema("Overview", [])
        }

        func call(arguments: GeneratedContent) async throws -> String {
            tools.overview()
        }
    }
#endif
