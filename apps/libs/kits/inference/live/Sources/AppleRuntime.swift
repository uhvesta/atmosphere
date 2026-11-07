import Foundation
import InferenceKit
import JSONKit

#if canImport(FoundationModels)
    import FoundationModels
#endif

/// Apple's on-device model, through Foundation Models. Before iOS 26 and
/// macOS 26 it says the system is too old.
public struct AppleRuntime: ModelRuntime {
    public init() {}

    public var budget: ContextBudget { .appleOnDevice }
    /// Text only: Apple's on-device model reads no photos or recordings on
    /// iOS and macOS 26.
    public var inputs: Set<ModelInput> { [] }

    public var status: ModelStatus {
        #if canImport(FoundationModels)
            if #available(iOS 26, macOS 26, *) {
                return switch SystemLanguageModel.default.availability {
                case .available: .available
                case .unavailable(.deviceNotEligible): .deviceNotEligible
                case .unavailable(.appleIntelligenceNotEnabled): .notEnabled
                case .unavailable(.modelNotReady): .modelNotReady
                case .unavailable: .deviceNotEligible
                }
            }
        #endif
        return .systemTooOld
    }

    public func prewarm() {
        #if canImport(FoundationModels)
            if #available(iOS 26, macOS 26, *), status == .available {
                LanguageModelSession().prewarm()
            }
        #endif
    }

    public func respond(to turn: ModelTurn, partial: @escaping @Sendable (JSONValue) -> Void)
        async throws(ModelRuntimeError) -> JSONValue
    {
        if let kind = turn.unreadable(by: inputs) { throw .unsupportedInput(kind) }
        #if canImport(FoundationModels)
            if #available(iOS 26, macOS 26, *) {
                let status = status
                guard status == .available else { throw .unavailable(status) }
                do {
                    return try await AppleSession.respond(to: turn, partial: partial)
                } catch let error as LanguageModelSession.GenerationError {
                    throw AppleSession.runtimeError(error)
                } catch is CancellationError {
                    throw .cancelled
                } catch {
                    throw .failed(error.localizedDescription)
                }
            }
        #endif
        throw .unavailable(.systemTooOld)
    }
}

#if canImport(FoundationModels)
    @available(iOS 26, macOS 26, *)
    private enum AppleSession {
        static func respond(to turn: ModelTurn, partial: @escaping @Sendable (JSONValue) -> Void) async throws
            -> JSONValue
        {
            let session = LanguageModelSession(
                tools: try turn.tools.map(AppleTool.init), instructions: turn.instructions)
            guard let schema = turn.schema else {
                var last = ""
                for try await snapshot in session.streamResponse(to: turn.prompt) {
                    last = snapshot.content
                    partial(.string(last))
                }
                return .string(last)
            }
            let generation = try GenerationSchema(root: dynamic(schema), dependencies: [])
            var last = JSONValue.null
            let stream = session.streamResponse(
                to: turn.prompt, schema: generation, includeSchemaInPrompt: turn.schemaInPrompt)
            for try await snapshot in stream {
                last = json(snapshot.rawContent)
                partial(last)
            }
            return last
        }

        #if targetEnvironment(simulator)
            private static let onSimulator = true
        #else
            private static let onSimulator = false
        #endif

        static func runtimeError(_ error: LanguageModelSession.GenerationError) -> ModelRuntimeError {
            switch error {
            case .exceededContextWindowSize: .exceededContext
            case .decodingFailure: .decodingFailure
            case .guardrailViolation, .refusal: .declined
            case .rateLimited, .concurrentRequests: .busy
            default: onSimulator ? .simulatorHost(error.localizedDescription) : .failed(error.localizedDescription)
            }
        }

        static func dynamic(_ schema: ModelSchema) -> DynamicGenerationSchema {
            switch schema {
            case .string: DynamicGenerationSchema(type: String.self)
            case .bool: DynamicGenerationSchema(type: Bool.self)
            case .choice(let name, let choices): DynamicGenerationSchema(name: name, anyOf: choices)
            case .array(let item): DynamicGenerationSchema(arrayOf: dynamic(item))
            case .object(let name, let properties):
                DynamicGenerationSchema(name: name, properties: properties.map(property))
            }
        }

        static func property(_ property: ModelProperty) -> DynamicGenerationSchema.Property {
            .init(
                name: property.name, description: property.description, schema: dynamic(property.schema),
                isOptional: property.isOptional)
        }

        static func json(_ content: GeneratedContent) -> JSONValue {
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

    /// One of the turn's tools, as Foundation Models calls it.
    @available(iOS 26, macOS 26, *)
    private struct AppleTool: Tool {
        let tool: ModelTool
        let parameters: GenerationSchema

        var name: String { tool.name }
        var description: String { tool.description }

        init(_ tool: ModelTool) throws {
            self.tool = tool
            // Each tool's arguments are an object named after it, such as
            // FindTasks.
            let name = tool.name.prefix(1).uppercased() + tool.name.dropFirst()
            parameters = try GenerationSchema(
                root: AppleSession.dynamic(.object(name: name, tool.parameters)), dependencies: [])
        }

        func call(arguments: GeneratedContent) async throws -> String {
            await tool.call(AppleSession.json(arguments))
        }
    }
#endif
