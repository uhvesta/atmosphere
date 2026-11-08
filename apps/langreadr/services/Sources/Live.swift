import Foundation
import ImageIO
import LangReadrCore
import NaturalLanguage
import Vision

extension ReaderServices {
    /// The system's: NaturalLanguage, ICU, Vision and URLSession. Nothing leaves the device but a page's request.
    public static var live: ReaderServices {
        ReaderServices(
            detectLanguage: Language.detect,
            words: WordSpan.split,
            transliterate: { text in
                guard !Script.isLatin(text), let latin = text.applyingTransform(.toLatin, reverse: false) else {
                    return nil
                }
                return Script.isLatin(latin) ? latin : nil
            },
            recognizeText: TextRecognition.lines,
            fetchPage: PageFetch.html
        )
    }
}

extension Language {
    static func detect(_ text: String) -> Language? {
        let recognizer = NLLanguageRecognizer()
        recognizer.processString(text)
        guard let language = recognizer.dominantLanguage, language != .undetermined else { return nil }
        return Language(language.rawValue)
    }
}

extension WordSpan {
    /// Words by NaturalLanguage's tokenizer, which finds them in scripts
    /// written without spaces too.
    static func split(_ text: String, in language: Language) -> [WordSpan] {
        let tokenizer = NLTokenizer(unit: .word)
        tokenizer.string = text
        tokenizer.setLanguage(NLLanguage(language.rawValue))
        var spans: [WordSpan] = []
        tokenizer.enumerateTokens(in: text.startIndex..<text.endIndex) { range, _ in
            spans.append(
                WordSpan(
                    text: String(text[range]), start: text.distance(from: text.startIndex, to: range.lowerBound),
                    end: text.distance(from: text.startIndex, to: range.upperBound)))
            return true
        }
        return spans
    }
}

private enum TextRecognition {
    static func lines(in data: Data) async throws(RecognitionError) -> [String] {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil),
            let image = CGImageSourceCreateImageAtIndex(source, 0, nil)
        else { throw .notAnImage }
        let result: Result<[String], RecognitionError> = await withCheckedContinuation { continuation in
            let request = VNRecognizeTextRequest { request, error in
                if let error {
                    continuation.resume(returning: .failure(.failed(error.localizedDescription)))
                    return
                }
                let observations = request.results as? [VNRecognizedTextObservation] ?? []
                continuation.resume(returning: .success(observations.compactMap { $0.topCandidates(1).first?.string }))
            }
            request.recognitionLevel = .accurate
            request.automaticallyDetectsLanguage = true
            request.usesLanguageCorrection = true
            do {
                try VNImageRequestHandler(cgImage: image).perform([request])
            } catch {
                continuation.resume(returning: .failure(.failed(error.localizedDescription)))
            }
        }
        return try result.get()
    }
}

private enum PageFetch {
    static func html(_ url: URL) async throws(PageError) -> String {
        let host = url.host() ?? url.absoluteString
        var request = URLRequest(url: url, timeoutInterval: 20)
        request.setValue("text/html", forHTTPHeaderField: "Accept")
        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await URLSession.shared.data(for: request)
        } catch {
            throw .unreachable(host: host, reason: error.localizedDescription)
        }
        if let http = response as? HTTPURLResponse, !(200..<300).contains(http.statusCode) {
            throw .status(host: host, code: http.statusCode)
        }
        let type = response.mimeType ?? "text/html"
        guard type.hasPrefix("text/") || type.contains("html") else {
            throw .notAPage(host: host, type: type)
        }
        let encoding = response.textEncodingName.map {
            String.Encoding(
                rawValue: CFStringConvertEncodingToNSStringEncoding(
                    CFStringConvertIANACharSetNameToEncoding($0 as CFString)))
        }
        // A page in an encoding it doesn't declare still reads, mostly, as UTF-8.
        return String(bytes: data, encoding: encoding ?? .utf8) ?? String(bytes: data, encoding: .utf8)
            ?? String(bytes: data, encoding: .isoLatin1) ?? ""
    }
}
