import Foundation
import InferenceKit

extension Bench {
    /// Runs each of `candidates` on the variant of `installed` it names, one
    /// at a time, with nothing else loaded: the short fixed prompt for its
    /// speed, memory and CPU, then the quality prompts for its answers,
    /// which are compared with the most precise candidate's. A candidate
    /// that fails is reported and the others go on; once the run is
    /// cancelled, the rest are skipped. `report` gets each outcome as it
    /// comes, before qualities are known.
    ///
    /// - Returns: Each candidate's outcome, in the order given.
    public func tune(
        _ candidates: [TuneCandidate], of installed: [InstalledModel], host: ModelHost,
        report: @Sendable (TuneCandidate, TuneOutcome) async -> Void = { _, _ in }
    ) async -> [TuneOutcome] {
        await host.unload(nil)
        var outcomes: [TuneOutcome] = []
        var answers: [TuneCandidate: [String]] = [:]
        for candidate in candidates {
            if Task.isCancelled {
                outcomes.append(.skipped)
                continue
            }
            let outcome: TuneOutcome
            switch await run(candidate, of: installed) {
            case .success(let run):
                outcome = .measured(TuneResult(key: key(candidate), measurement: run.measurement))
                answers[candidate] = run.answers
            case .failure(.engine(.cancelled)):
                outcome = .skipped
            case .failure(let failure):
                outcome = .failed(failure)
            }
            outcomes.append(outcome)
            await report(candidate, outcome)
        }
        return scored(outcomes, candidates: candidates, answers: answers)
    }

    /// The key `candidate`'s results are kept under, on this device.
    public func key(_ candidate: TuneCandidate) -> TuneKey {
        candidate.key(device: device.model, system: device.system)
    }

    private struct Run {
        var measurement: ModelMeasurement
        var answers: [String]?
    }

    private func run(_ candidate: TuneCandidate, of installed: [InstalledModel]) async -> Result<Run, TuneFailure> {
        guard
            var model = installed.first(where: {
                $0.id == candidate.model && $0.spec.variant.runtime == candidate.runtime
                    && $0.spec.variant.quantization == candidate.quantization
            })
        else { return .failure(.notDownloaded) }
        do throws(EngineOptionsError) {
            try candidate.options.validate(for: model.spec, on: candidate.runtime)
        } catch {
            return .failure(.invalidOptions(error))
        }
        model.options = candidate.options
        model.spec.profile.sampling = .greedy
        do throws(ModelRuntimeError) {
            let measurement = try await once(model, on: candidate.options.backend)
            let answers = try await self.answers(of: model, on: candidate.options.backend)
            return .success(Run(measurement: measurement, answers: answers))
        } catch {
            return .failure(.engine(error))
        }
    }

    /// `model`'s answers to the quality prompts, each its first
    /// `TunePrompts.answerWords` words; nil when the bench can't load
    /// models.
    private func answers(of model: InstalledModel, on backend: ModelBackend) async throws(ModelRuntimeError)
        -> [String]?
    {
        guard let load else { return nil }
        if Task.isCancelled { throw .cancelled }
        let engine = try await load(model, backend)
        var answers: [String] = []
        for prompt in TunePrompts.standard {
            if Task.isCancelled { throw .cancelled }
            let chat = try await engine.chat(instructions: "", constrained: false)
            let written = Locked("")
            let cut = Locked(false)
            do throws(ModelRuntimeError) {
                let answer = try await chat.send(
                    prompt, attachments: [], format: nil, thinkingTokens: nil, thought: nil,
                    partial: { text in
                        written.setValue(text)
                        if Self.words(text) >= TunePrompts.answerWords {
                            cut.setValue(true)
                            chat.cancel()
                        }
                    })
                answers.append(Self.clipped(answer))
            } catch .cancelled where cut.value && !Task.isCancelled {
                answers.append(Self.clipped(written.value))
            }
        }
        return answers
    }

    /// The outcomes with each measured one's agreement with the most
    /// precise of those that answered.
    private func scored(_ outcomes: [TuneOutcome], candidates: [TuneCandidate], answers: [TuneCandidate: [String]])
        -> [TuneOutcome]
    {
        guard let reference = tuneReference(of: candidates.filter { answers[$0] != nil }),
            let referenceAnswers = answers[reference]
        else { return outcomes }
        return zip(candidates, outcomes).map { candidate, outcome in
            guard case .measured(var result) = outcome, let own = answers[candidate],
                let agreement = TuneQuality.agreement(own, referenceAnswers)
            else { return outcome }
            result.quality = TuneQuality(
                agreement: agreement, isReference: candidate == reference, reference: reference)
            return .measured(result)
        }
    }

    private static func words(_ text: String) -> Int {
        text.split { $0.isWhitespace }.count
    }

    private static func clipped(_ text: String) -> String {
        text.split { $0.isWhitespace }.prefix(TunePrompts.answerWords).joined(separator: " ")
    }
}
