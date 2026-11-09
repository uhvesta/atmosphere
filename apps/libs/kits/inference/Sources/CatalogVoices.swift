/// The voice models there are to download: neural text-to-speech, apart
/// from the language models of `ModelCatalog`. See Speech.swift for why
/// these.
public enum VoiceCatalog {
    public static let all: [VoiceModel] = [supertonic3, audio8Persian]

    /// The voice model `id` names, if it's one.
    public static func model(_ id: ModelID) -> VoiceModel? {
        all.first { $0.id == id }
    }

    /// Whether `spec` is a voice model's, so lists of language models
    /// leave it out.
    public static func isVoice(_ spec: ModelSpec) -> Bool {
        spec.runtime == .coreML || model(spec.id) != nil
    }

    /// Supertone's Supertonic 3, as Core ML packages in 32-bit floats, a
    /// parity export of Supertone/supertonic-3 at commit 3cadd1ee: the
    /// duration predictor, text encoder, flow-matching vector estimator and
    /// vocoder, the tokenizer's table of characters, its settings, and ten
    /// voices. The 16-bit export saves only about 50 MB.
    public static let supertonic3 = VoiceModel(
        spec: ModelSpec(
            id: "supertonic-3", name: "Supertonic 3",
            summary: "Natural voices in 31 languages", runtime: .coreML,
            source: ModelSource(
                repo: "aufklarer/Supertonic-3-CoreML", commit: "d159f984aa1c4ccaafd0541076463e422cf75408",
                files: supertonicPackages + supertonicSettings + supertonicVoices),
            contextTokens: 0, memoryBytes: 700_000_000, license: "OpenRAIL-M"),
        engine: .supertonic,
        languages: [
            "en", "ko", "ja", "ar", "bg", "cs", "da", "de", "el", "es", "et", "fi", "fr", "hi", "hr", "hu", "id",
            "it", "lt", "lv", "nl", "pl", "pt", "ro", "ru", "sk", "sl", "sv", "tr", "uk", "vi",
        ],
        voices: ["F1", "F2", "F3", "F4", "F5"].enumerated().map { VoiceStyle(id: $1, name: "Female \($0 + 1)") }
            + ["M1", "M2", "M3", "M4", "M5"].enumerated().map { VoiceStyle(id: $1, name: "Male \($0 + 1)") })

    /// Audio8's Persian voice, a dual autoregressive model of about 600M
    /// parameters in bfloat16, at commit bcca4112, with its tokenizer and
    /// its audio codec, a PyTorch checkpoint. The model is Apache 2.0, but
    /// most of the codec's weights are Fish Audio's S2 Pro codec's, under
    /// the Fish Audio Research License, so the whole is for non-commercial
    /// use only.
    public static let audio8Persian = VoiceModel(
        spec: ModelSpec(
            id: "audio8-tts-persian", name: "Audio8 Persian",
            summary: "A natural Persian voice", runtime: .mlx,
            source: ModelSource(
                repo: "ArminRahimi/Audio8-TTS-Persian", commit: "bcca4112db62fccaf733772226d4f9931a088a0a",
                files: [
                    ModelFile(
                        path: "config.json", size: 1_459,
                        digest: .sha256("92e19f10edb6c79ef2ccf4aebe218baa2c12743d648d75238041eca39439f8d3")),
                    ModelFile(
                        path: "tokenizer_config.json", size: 860_832,
                        digest: .sha256("b8d149343ae425b0da67e6708686aceb51be7815d9792f265fc12ff04d5e9856")),
                    ModelFile(
                        path: "tokenizer.json", size: 12_217_872,
                        digest: .sha256("f24e08099d45a8adf3f52f5f0b03276e433bb9d689bb15fcbcc48ce58744588b")),
                    ModelFile(
                        path: "model.safetensors", size: 1_202_342_528,
                        digest: .sha256("e0540b75b39171ace3942ddca063a0c8881a66da97ce3eecae1a5bd4aa55fa4c")),
                    ModelFile(
                        path: "codec.pth", size: 1_349_857_559,
                        digest: .sha256("c310505aa11fe2f6cc63b8d3130dc7e77e73227774f5c62575769b1f47a8d048")),
                ]),
            contextTokens: 0, memoryBytes: 2_500_000_000, license: "Non-commercial (Fish Audio Research)"),
        engine: .dualAR,
        languages: ["fa"],
        voices: [VoiceStyle(id: "default", name: "Default")])

    private static let supertonicPackages: [ModelFile] = [
        package(
            "DurationPredictor", spec: (83_267, "23f4128d68de2cc0aacd5568d134d33f0340deeac64443bd5dc2073a4fada293"),
            weights: (3_590_080, "714d7d6bcf0cbae95a8e0a27c88b643961d0785fe381d33243fc679fd1b76625"),
            manifest: "ac9c00a29c1da41147c863f67d8b61f528acf519"),
        package(
            "TextEncoder", spec: (128_982, "3671067b6123ea2fdefc2b36fde98ea25d09a0c9ebc0051e7b2b195387d088a7"),
            weights: (36_035_200, "9618a75954419ab1e19d1f0946df20ab391d993632f0d2478085c32ddc17e8d7"),
            manifest: "d4afc32f6f9d37365aca7923e57945b5e4146df2"),
        package(
            "VectorEstimator", spec: (289_258, "b14fb761f8d755f4859d53116478be091cae378a32bbd75cad7be245ea475b2a"),
            weights: (255_276_032, "593cb7df14efd8d98e6e0543e0c18fb5778f3602e36164ec0dcf35f5c713e1fd"),
            manifest: "0c86c460da5e714201ed09dc94b8582c8f03c4e5"),
        package(
            "Vocoder", spec: (64_393, "a1f37fe142b88dc57487092be463de4e0a1ec8bf850128f03c34005a28c1be33"),
            weights: (101_339_136, "f0f325c9e90fe325cf1450e7f5f204ded3a8771b32d5e151435a9fb0fc07ac6d"),
            manifest: "bfdc7090c09dd7de8032a6e28fbdb9d354baddfb"),
    ].flatMap { $0 }

    private static let supertonicSettings: [ModelFile] = [
        ModelFile(path: "tts.json", size: 8_253, digest: .gitBlobSHA1("28575b0793c92607a0f0a292550df9bd17709a79")),
        ModelFile(
            path: "unicode_indexer.json", size: 277_676,
            digest: .gitBlobSHA1("3aaf4d33024aff0be455633c24b3636b3a810150")),
    ]

    private static let supertonicVoices: [ModelFile] = [
        ("F1", 292_046, "421365f307ed1535d8da16845031d0e9fa3e60c5"),
        ("F2", 292_423, "b09a3a433df8a4856d1d7d64736a56ddf37a4ea3"),
        ("F3", 290_794, "a366f94c77c4b440404071f1a52bed3ccf83a5ff"),
        ("F4", 291_808, "39c78a52795320c6101b7eaf984c0e2352fbdc18"),
        ("F5", 291_479, "06983f8816ad00a8eeb7efd37a33d457587e16f8"),
        ("M1", 291_748, "bddfff85a7c4c140c11620bd7005d86681dccdac"),
        ("M2", 292_055, "602ff6c4d6d48cdb71c37461f7c175d71fe8b34b"),
        ("M3", 290_198, "000b60436b77619b933fdc33628ad737595e6794"),
        ("M4", 291_522, "aae253ae81185fa06a6469c3f1cdff04ed121ddf"),
        ("M5", 291_469, "0536b252698830f8536c077edcb94e1ce4768ab6"),
    ].map { ModelFile(path: "voice_styles/\($0).json", size: $1, digest: .gitBlobSHA1($2)) }

    /// A Core ML package's spec, weights and manifest.
    private static func package(
        _ name: String, spec: (Int64, String), weights: (Int64, String), manifest: String
    ) -> [ModelFile] {
        let data = "\(name).mlpackage/Data/com.apple.CoreML"
        return [
            ModelFile(path: "\(data)/model.mlmodel", size: spec.0, digest: .sha256(spec.1)),
            ModelFile(path: "\(data)/weights/weight.bin", size: weights.0, digest: .sha256(weights.1)),
            ModelFile(path: "\(name).mlpackage/Manifest.json", size: 617, digest: .gitBlobSHA1(manifest)),
        ]
    }
}
