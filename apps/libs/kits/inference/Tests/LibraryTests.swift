import Foundation
import InferenceKit
import XCTest

final class ModelLinkTests: XCTestCase {
    func testAModelPageIsTheWholeRepository() throws {
        let link = try ModelLink.parse(" https://huggingface.co/litert-community/gemma-4-E2B-it-litert-lm \n")

        XCTAssertEqual(link, ModelLink(repo: "litert-community/gemma-4-E2B-it-litert-lm"))
    }

    func testAFileLinkNamesTheRevisionAndFile() throws {
        let resolve = try ModelLink.parse("https://hf.co/org/model/resolve/abc123/folder/model.litertlm")
        let blob = try ModelLink.parse("https://huggingface.co/org/model/blob/main/model.litertlm")
        let tree = try ModelLink.parse("https://huggingface.co/org/model/tree/v2")

        XCTAssertEqual(resolve, ModelLink(repo: "org/model", revision: "abc123", path: "folder/model.litertlm"))
        XCTAssertEqual(blob, ModelLink(repo: "org/model", revision: "main", path: "model.litertlm"))
        XCTAssertEqual(tree, ModelLink(repo: "org/model", revision: "v2"))
    }

    func testOnlyHuggingFaceOverHTTPSIsAccepted() {
        let refused = [
            "http://huggingface.co/org/model", "https://example.com/org/model",
            "https://user:pass@huggingface.co/org/model", "https://huggingface.co:8443/org/model",
            "https://127.0.0.1/org/model", "https://cdn.hf.co/org/model", "https://huggingface.co.evil.com/org/model",
        ]
        for text in refused {
            XCTAssertThrowsError(try ModelLink.parse(text), text) { error in
                guard case .hostNotAllowed = error as? ModelResolveError else {
                    return XCTFail("\(text): \(error)")
                }
            }
        }
    }

    func testALinkWithoutAModelIsRefused() {
        for text in [
            "https://huggingface.co/org", "https://huggingface.co/datasets/org/data",
            "https://huggingface.co/org/model/discussions",
        ] {
            XCTAssertThrowsError(try ModelLink.parse(text), text) { error in
                XCTAssertEqual(error as? ModelResolveError, .noRepository(text))
            }
        }
        XCTAssertThrowsError(try ModelLink.parse("gemma")) { error in
            XCTAssertEqual(error as? ModelResolveError, .notALink("gemma"))
        }
    }

    func testDownloadsMayComeFromTheCDN() throws {
        let allowed = try XCTUnwrap(URL(string: "https://us.aws.cdn.hf.co/xet-bridge-us/file"))
        let plain = try XCTUnwrap(URL(string: "http://us.aws.cdn.hf.co/file"))
        let elsewhere = try XCTUnwrap(URL(string: "https://cdn.example.com/file"))
        let lookalike = try XCTUnwrap(URL(string: "https://evilhf.co/file"))

        XCTAssertTrue(HostPolicy.allows(allowed))
        XCTAssertFalse(HostPolicy.allows(plain))
        XCTAssertFalse(HostPolicy.allows(elsewhere))
        XCTAssertFalse(HostPolicy.allows(lookalike))
    }
}

final class ModelResolutionTests: XCTestCase {
    private let sha = String(repeating: "ab", count: 32)
    private let info = HubModelInfo(sha: "0123456789abcdef0123456789abcdef01234567", isGated: false, license: "mit")

    private var tree: [HubTreeEntry] {
        [
            HubTreeEntry(path: "README.md", size: 10, oid: "1111", sha256: nil),
            HubTreeEntry(path: "model.litertlm", size: 2_000, oid: "2222", sha256: sha),
        ]
    }

    func testTheOneRunnableFileIsPinnedWithItsHash() throws {
        let spec = try ModelResolution.spec(
            ModelLink(repo: "org/model"), info: info, tree: tree, sha256: nil, catalog: [])

        XCTAssertEqual(spec.runtime, .liteRT)
        XCTAssertEqual(spec.license, "mit")
        XCTAssertEqual(
            spec.source,
            ModelSource(
                repo: "org/model", commit: info.sha,
                files: [ModelFile(path: "model.litertlm", size: 2_000, digest: .sha256(sha))]))
    }

    func testAGivenHashMustMatchHuggingFaces() {
        let other = String(repeating: "cd", count: 32)

        XCTAssertThrowsError(
            try ModelResolution.spec(ModelLink(repo: "org/model"), info: info, tree: tree, sha256: other, catalog: [])
        ) { error in
            XCTAssertEqual(
                error as? ModelResolveError, .hashDisagrees(file: "model.litertlm", given: other, listed: sha))
        }
        XCTAssertNoThrow(
            try ModelResolution.spec(
                ModelLink(repo: "org/model"), info: info, tree: tree, sha256: sha.uppercased(), catalog: []))
    }

    func testAHashThatIsntOneIsRefused() {
        XCTAssertThrowsError(
            try ModelResolution.spec(ModelLink(repo: "org/model"), info: info, tree: tree, sha256: "abc", catalog: [])
        ) { error in
            XCTAssertEqual(error as? ModelResolveError, .badHash("abc"))
        }
    }

    func testARepositoryWithSeveralModelsNeedsAFileLink() throws {
        let several =
            tree + [HubTreeEntry(path: "model-gpu.litertlm", size: 1_000, oid: "3333", sha256: sha)]

        XCTAssertThrowsError(
            try ModelResolution.spec(ModelLink(repo: "org/model"), info: info, tree: several, sha256: nil, catalog: [])
        ) { error in
            XCTAssertEqual(
                error as? ModelResolveError, .severalFiles(repo: "org/model", ["model.litertlm", "model-gpu.litertlm"]))
        }
        let picked = try ModelResolution.spec(
            ModelLink(repo: "org/model", revision: "main", path: "model-gpu.litertlm"), info: info, tree: several,
            sha256: nil, catalog: [])
        XCTAssertEqual(picked.source?.files.map(\.path), ["model-gpu.litertlm"])
    }

    func testARepositoryWithoutARunnableFileIsRefused() {
        let readme = [
            HubTreeEntry(path: "README.md", size: 10, oid: "1111", sha256: nil),
            HubTreeEntry(path: "model.safetensors", size: 10, oid: "2222", sha256: sha),
            HubTreeEntry(path: "mmproj-model-f16.gguf", size: 10, oid: "3333", sha256: sha),
        ]

        XCTAssertThrowsError(
            try ModelResolution.spec(ModelLink(repo: "org/model"), info: info, tree: readme, sha256: nil, catalog: [])
        ) { error in
            XCTAssertEqual(error as? ModelResolveError, .noRunnableFile(repo: "org/model"))
        }
    }

    func testAGGUFRunsOnLlamaWithItsSmallestVisionEncoder() throws {
        let gguf = [
            HubTreeEntry(path: "model-Q4_K_M.gguf", size: 1_000, oid: "1111", sha256: sha),
            HubTreeEntry(path: "mmproj-model-F16.gguf", size: 400, oid: "2222", sha256: sha),
            HubTreeEntry(path: "mmproj-model-Q8_0.gguf", size: 200, oid: "3333", sha256: sha),
        ]

        let spec = try ModelResolution.spec(
            ModelLink(repo: "org/model"), info: info, tree: gguf, sha256: nil, catalog: [])

        XCTAssertEqual(spec.runtime, .llama)
        XCTAssertEqual(spec.inputs, [.image])
        XCTAssertEqual(spec.source?.files.map(\.path), ["model-Q4_K_M.gguf", "mmproj-model-Q8_0.gguf"])
    }

    func testAGGUFWithoutAnEncoderReadsOnlyText() throws {
        let gguf = [HubTreeEntry(path: "model-Q4_K_M.gguf", size: 1_000, oid: "1111", sha256: sha)]

        let spec = try ModelResolution.spec(
            ModelLink(repo: "org/model"), info: info, tree: gguf, sha256: nil, catalog: [])

        XCTAssertEqual(spec.runtime, .llama)
        XCTAssertEqual(spec.inputs, [])
        XCTAssertEqual(spec.source?.files.map(\.path), ["model-Q4_K_M.gguf"])
    }

    func testAGatedRepositoryIsRefused() {
        let gated = HubModelInfo(sha: info.sha, isGated: true, license: nil)

        XCTAssertThrowsError(
            try ModelResolution.spec(ModelLink(repo: "org/model"), info: gated, tree: tree, sha256: nil, catalog: [])
        ) { error in
            XCTAssertEqual(error as? ModelResolveError, .gated(repo: "org/model"))
        }
    }

    func testACatalogModelsLinkIsItsCatalogEntry() throws {
        let gemma = ModelCatalog.gemma4E2B
        let source = try XCTUnwrap(gemma.source)
        let file = try XCTUnwrap(source.files.first)
        guard case .sha256(let digest) = file.digest else { return XCTFail("no SHA-256") }
        let tree = [HubTreeEntry(path: file.path, size: file.size, oid: "1111", sha256: digest)]
        let info = HubModelInfo(sha: source.commit, isGated: false, license: "apache-2.0")

        let spec = try ModelResolution.spec(
            ModelLink(repo: source.repo, revision: source.commit, path: file.path), info: info, tree: tree,
            sha256: nil)

        XCTAssertEqual(spec, gemma)
    }

    func testHuggingFacesAnswersAreRead() throws {
        let info = Data(
            #"{"id":"org/model","sha":"abc","gated":"manual","cardData":{"license":"apache-2.0"}}"#.utf8)
        let open = Data(#"{"sha":"def","gated":false}"#.utf8)
        let tree = Data(
            #"""
            [{"type":"directory","oid":"9","size":0,"path":"docs"},
             {"type":"file","oid":"fc07","size":11995,"path":"chat_template.jinja"},
             {"type":"file","oid":"6ee3","size":2588147712,"path":"m.litertlm",
              "lfs":{"oid":"181938105E0E","size":2588147712,"pointerSize":135},"xetHash":"ee3c"}]
            """#.utf8)

        let gated = try JSONDecoder().decode(HubModelInfo.self, from: info)
        let ungated = try JSONDecoder().decode(HubModelInfo.self, from: open)
        let entries = try JSONDecoder().decode([HubTreeEntry].self, from: tree)

        XCTAssertEqual(gated, HubModelInfo(sha: "abc", isGated: true, license: "apache-2.0"))
        XCTAssertEqual(ungated, HubModelInfo(sha: "def", isGated: false, license: nil))
        XCTAssertEqual(
            entries.filter { $0.type == "file" }.map(\.file),
            [
                ModelFile(path: "chat_template.jinja", size: 11_995, digest: .gitBlobSHA1("fc07")),
                ModelFile(path: "m.litertlm", size: 2_588_147_712, digest: .sha256("181938105e0e")),
            ])
    }
}

final class ModelLibraryTypesTests: XCTestCase {
    func testADownloadNeedsATenthMoreAndAGigabyteFree() {
        XCTAssertEqual(spaceNeeded(toDownload: 2_000_000_000), 3_200_000_000)
        XCTAssertEqual(spaceNeeded(toDownload: 0), 1_000_000_000)
    }

    func testAPastedModelsFolderStaysInsideTheLibrary() {
        let id = ModelID(rawValue: "hf/../org/model/../../x.litertlm")

        XCTAssertFalse(id.directoryName.contains("/"))
        XCTAssertFalse(id.directoryName.hasPrefix("."))
        XCTAssertNotEqual(id.directoryName, ModelID(rawValue: "hf/org/model/x.litertlm").directoryName)
    }

    func testTheCatalogDownloadsFromHuggingFaceAtAPinnedCommit() throws {
        for spec in ModelCatalog.all {
            for variant in spec.variants {
                guard let source = variant.source else {
                    XCTAssertEqual(variant.runtime, .apple)
                    continue
                }
                XCTAssertEqual(source.commit.count, 40, spec.name)
                for file in source.files {
                    let url = try XCTUnwrap(source.url(of: file))
                    XCTAssertTrue(HostPolicy.allows(url), spec.name)
                    guard case .sha256(let digest) = file.digest else {
                        return XCTFail("\(spec.name) has no SHA-256")
                    }
                    XCTAssertEqual(digest.count, 64, spec.name)
                }
            }
        }
    }

    /// Every variant of the catalog's that runs on `runtime`, as its model.
    private func catalog(on runtime: RuntimeKind) -> [ModelSpec] {
        ModelCatalog.all.flatMap { spec in spec.variants.filter { $0.runtime == runtime }.map(spec.running) }
    }

    func testTheCatalogsMLXModelsAreWholeFoldersAndFitTheirEstimate() {
        let mlx = catalog(on: .mlx)

        XCTAssertFalse(mlx.isEmpty)
        for spec in mlx {
            let paths = Set(spec.source?.files.map(\.path) ?? [])
            XCTAssertTrue(paths.isSuperset(of: ["config.json", "tokenizer.json", "tokenizer_config.json"]), spec.name)
            let weights = spec.source?.files.filter { $0.path.hasSuffix(".safetensors") } ?? []
            XCTAssertFalse(weights.isEmpty, spec.name)
            XCTAssertGreaterThan(spec.memoryBytes, weights.reduce(0) { $0 + $1.size }, spec.name)
            XCTAssertTrue(spec.inputs.isEmpty, spec.name)
            XCTAssertNotNil(spec.profile.thinking, spec.name)
        }
    }

    func testEveryMLXModelFallsBackToLlama() {
        for spec in ModelCatalog.all where spec.runtime == .mlx {
            XCTAssertTrue(spec.variants.contains { $0.runtime == .llama }, spec.name)
        }
    }

    func testTheCatalogsGGUFModelsRunOnLlamaAndFitTheirEstimate() {
        let gguf = catalog(on: .llama)

        XCTAssertFalse(gguf.isEmpty)
        for spec in gguf {
            let files = spec.source?.files ?? []
            let weights = files.reduce(0) { $0 + $1.size }
            XCTAssertTrue(files.allSatisfy { $0.path.hasSuffix(".gguf") }, spec.name)
            XCTAssertGreaterThan(spec.memoryBytes, weights, spec.name)
            XCTAssertEqual(
                spec.inputs.contains(.image), files.contains { ModelResolution.isEncoder($0.path) }, spec.name)
            XCTAssertNotNil(spec.profile.thinking, spec.name)
        }
    }

    func testProgressIsTheShareReceived() {
        XCTAssertEqual(DownloadState.downloading(received: 25, total: 100).fraction, 0.25)
        XCTAssertEqual(DownloadState.paused(received: 50, total: 100).fraction, 0.5)
        XCTAssertEqual(DownloadState.downloading(received: 0, total: 0).fraction, 0)
    }
}

final class ModelThinkingTests: XCTestCase {
    func testThinkingAgainAddsTheTextAndTheTime() {
        let first = ModelThinking(text: "Look up the task.", duration: .seconds(2), processor: .gpu)
        let second = ModelThinking(text: "It's due today.", duration: .milliseconds(500), processor: .gpu)

        XCTAssertEqual(
            first.then(second),
            ModelThinking(
                text: "Look up the task.\n\nIt's due today.", duration: .milliseconds(2_500), processor: .gpu))
    }

    func testARoundWithoutThoughtsAddsOnlyItsTime() {
        let first = ModelThinking(text: "Look up the task.", duration: .seconds(2), processor: .gpu)
        let second = ModelThinking(text: "", duration: .seconds(1), processor: .cpu)

        XCTAssertEqual(
            first.then(second), ModelThinking(text: "Look up the task.", duration: .seconds(3), processor: .cpu))
    }

    func testThinkingNotYetDoneHasNoTime() {
        XCTAssertNil(ModelThinking(text: "So far").then(ModelThinking(text: "more")).duration)
    }
}

final class ModelVariantTests: XCTestCase {
    private let commit = String(repeating: "c", count: 40)

    private func file(_ path: String) -> ModelSource {
        ModelSource(
            repo: "org/model", commit: commit,
            files: [ModelFile(path: path, size: 10, digest: .sha256(String(repeating: "d", count: 64)))])
    }

    private var twoWays: ModelSpec {
        ModelSpec(
            id: "two-ways", name: "Two ways", summary: "", contextTokens: 4_096, license: "",
            variants: [
                ModelVariant(runtime: .liteRT, quantization: "int4", source: file("a.litertlm"), memoryBytes: 1_000),
                ModelVariant(runtime: .liteRT, quantization: "int8", source: file("b.litertlm"), memoryBytes: 2_000),
            ])
    }

    func testAModelWithVariantsRunsAsTheFirst() {
        let spec = twoWays

        XCTAssertEqual(spec.source, file("a.litertlm"))
        XCTAssertEqual(spec.memoryBytes, 1_000)
        XCTAssertEqual(spec.variant.quantization, "int4")
    }

    func testRunningAnotherVariantTakesItsFilesAndMemory() throws {
        let other = try XCTUnwrap(twoWays.variants.last)
        let spec = twoWays.running(other)

        XCTAssertEqual(spec.source, file("b.litertlm"))
        XCTAssertEqual(spec.memoryBytes, 2_000)
        XCTAssertEqual(spec.variant, other)
        XCTAssertEqual(spec.variants.count, 2)
    }

    func testAManifestFromBeforeVariantsHasOnlyItsOwn() throws {
        let manifest = try JSONEncoder().encode(ModelCatalog.gemma4E2B)
        var json = try XCTUnwrap(JSONSerialization.jsonObject(with: manifest) as? [String: Any])
        json["variants"] = nil
        let decoded = try JSONDecoder().decode(ModelSpec.self, from: JSONSerialization.data(withJSONObject: json))

        XCTAssertEqual(decoded, ModelCatalog.gemma4E2B)
        XCTAssertEqual(decoded.variants, [decoded.variant])
    }

    func testTheCatalogKnowsADownloadedModelByItsEngineAndCommit() throws {
        let entry = try XCTUnwrap(ModelCatalog.entry(for: ModelCatalog.gemma4E4B))
        var elsewhere = ModelCatalog.gemma4E4B
        elsewhere.source?.commit = commit

        XCTAssertEqual(entry.source, ModelCatalog.gemma4E4B.source)
        XCTAssertNil(ModelCatalog.entry(for: elsewhere))
    }
}

final class ModelPlannerTests: XCTestCase {
    private let model = InstalledModel(
        spec: ModelCatalog.gemma4E2B, directory: URL(filePath: "/models/gemma"), installedAt: Date())

    private func measured(
        _ backend: ModelBackend, speed: Double, memory: Int64 = 1_000, cpu: Double = 0.001, system: String = "27.0"
    ) -> ModelMeasurement {
        ModelMeasurement(
            backend: backend, runtime: .liteRT, window: model.window, device: "iPhone18,1", system: system,
            prefillTokensPerSecond: speed * 10, decodeTokensPerSecond: speed, timeToFirstToken: 0.2,
            peakMemoryBytes: memory, cpuSecondsPerToken: cpu, measuredAt: Date())
    }

    private func plan(
        _ measurements: [ModelMeasurement], picked: ModelBackend? = nil, available: Int64 = .max
    ) -> ModelPlanner.Plan {
        ModelPlanner.plan(
            model, picked: picked, measurements: measurements, device: "iPhone18,1", system: "27.0",
            fits: { $0 <= available })
    }

    func testWithNothingMeasuredAModelRunsAsItsProfileSays() {
        XCTAssertEqual(plan([]), ModelPlanner.Plan(backend: .gpu, reason: .profile))
    }

    func testTheFastestMeasuredBackendIsPicked() {
        let cpu = measured(.cpu, speed: 30, cpu: 0.05)
        let gpu = measured(.gpu, speed: 20)

        XCTAssertEqual(plan([gpu, cpu]), ModelPlanner.Plan(backend: .cpu, reason: .measured(cpu)))
    }

    func testOfTwoAboutAsFastTheOneLighterOnTheCPUIsPicked() {
        let cpu = measured(.cpu, speed: 21, cpu: 0.05)
        let gpu = measured(.gpu, speed: 20, cpu: 0.002)

        XCTAssertEqual(plan([cpu, gpu]).backend, .gpu)
    }

    func testABackendThatDoesntFitNowIsntPicked() {
        let gpu = measured(.gpu, speed: 40, memory: 3_000)
        let cpu = measured(.cpu, speed: 10, memory: 1_000)

        XCTAssertEqual(plan([gpu, cpu], available: 2_000).backend, .cpu)
    }

    func testMeasurementsFromAnotherSystemDontCount() {
        let old = measured(.cpu, speed: 40, system: "26.4")

        XCTAssertEqual(plan([old]).reason, .profile)
    }

    func testThePersonsPickWins() {
        XCTAssertEqual(
            plan([measured(.gpu, speed: 40)], picked: .cpu), ModelPlanner.Plan(backend: .cpu, reason: .picked))
    }
}
