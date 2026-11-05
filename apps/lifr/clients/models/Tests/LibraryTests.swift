import Foundation
import LifrModelsClient
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
        let readme = [HubTreeEntry(path: "model.gguf", size: 10, oid: "1111", sha256: sha)]

        XCTAssertThrowsError(
            try ModelResolution.spec(ModelLink(repo: "org/model"), info: info, tree: readme, sha256: nil, catalog: [])
        ) { error in
            XCTAssertEqual(error as? ModelResolveError, .noRunnableFile(repo: "org/model"))
        }
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
            guard let source = spec.source else {
                XCTAssertEqual(spec.runtime, .apple)
                continue
            }
            XCTAssertEqual(source.commit.count, 40, spec.name)
            for file in source.files {
                let url = try XCTUnwrap(source.url(of: file))
                XCTAssertTrue(HostPolicy.allows(url), spec.name)
                guard case .sha256(let digest) = file.digest else { return XCTFail("\(spec.name) has no SHA-256") }
                XCTAssertEqual(digest.count, 64, spec.name)
            }
        }
    }

    func testProgressIsTheShareReceived() {
        XCTAssertEqual(DownloadState.downloading(received: 25, total: 100).fraction, 0.25)
        XCTAssertEqual(DownloadState.paused(received: 50, total: 100).fraction, 0.5)
        XCTAssertEqual(DownloadState.downloading(received: 0, total: 0).fraction, 0)
    }
}
