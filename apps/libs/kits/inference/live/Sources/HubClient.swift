import Foundation
import InferenceKit

/// Talks to Hugging Face: the API for what a repository has, and `resolve`
/// for its files. Refuses to follow a redirect off Hugging Face.
struct HubClient: Sendable {
    let session: URLSession

    /// The repository at `revision`, pinned to its commit.
    func info(_ repo: String, revision: String) async throws(HubError) -> HubModelInfo {
        let what = revision == "main" ? repo : "\(repo) at \(revision)"
        return try await json(api(repo, "revision", revision), what: what)
    }

    /// Every file in the repository at `commit`.
    func tree(_ repo: String, commit: String) async throws(HubError) -> [HubTreeEntry] {
        guard var url = api(repo, "tree", commit) else { throw .notFound(repo) }
        url.append(queryItems: [URLQueryItem(name: "recursive", value: "true")])
        return try await json(url, what: "the files of \(repo)")
    }

    /// The bytes `range` of the file at `url`. `what` names it in errors.
    func bytes(_ url: URL, _ range: ClosedRange<Int64>, what: String) async throws(ModelDownloadError) -> (
        Data, HTTPURLResponse
    ) {
        var request = URLRequest(url: url)
        request.setValue("bytes=\(range.lowerBound)-\(range.upperBound)", forHTTPHeaderField: "Range")
        do {
            return try await send(request, what: what)
        } catch {
            throw .hub(error)
        }
    }

    private func api(_ repo: String, _ kind: String, _ revision: String) -> URL? {
        var components = URLComponents()
        components.scheme = "https"
        components.host = "huggingface.co"
        let encoded = revision.addingPercentEncoding(withAllowedCharacters: .alphanumerics) ?? revision
        components.percentEncodedPath = "/api/models/\(repo)/\(kind)/\(encoded)"
        return components.url
    }

    private func json<Value: Decodable>(_ url: URL?, what: String) async throws(HubError) -> Value {
        guard let url else { throw .notFound(what) }
        let (data, _) = try await send(URLRequest(url: url), what: what)
        do {
            return try JSONDecoder().decode(Value.self, from: data)
        } catch {
            throw .unreadable(what)
        }
    }

    private func send(_ request: URLRequest, what: String) async throws(HubError) -> (Data, HTTPURLResponse) {
        let redirects = RedirectGuard()
        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await session.data(for: request, delegate: redirects)
        } catch {
            if let host = redirects.refused.value { throw .redirectNotAllowed(host) }
            throw .offline(error.localizedDescription)
        }
        if let host = redirects.refused.value { throw .redirectNotAllowed(host) }
        guard let http = response as? HTTPURLResponse else { throw .unreadable(what) }
        try Self.check(http, what: what)
        return (data, http)
    }

    /// Throws unless `http` is a success from Hugging Face.
    static func check(_ http: HTTPURLResponse, what: String) throws(HubError) {
        if let url = http.url, !HostPolicy.allows(url) { throw .redirectNotAllowed(url.host() ?? what) }
        switch http.statusCode {
        case 200..<300: return
        case 401, 403: throw .notAllowed(what)
        case 404: throw .notFound(what)
        case 429: throw .rateLimited
        default: throw .status(http.statusCode, what)
        }
    }
}

/// Stops a request that would leave Hugging Face.
private final class RedirectGuard: NSObject, URLSessionTaskDelegate, Sendable {
    /// The host a redirect pointed at, once one is refused.
    let refused = Locked<String?>(nil)

    func urlSession(
        _ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
        newRequest request: URLRequest
    ) async -> URLRequest? {
        guard let url = request.url, HostPolicy.allows(url) else {
            refused.setValue(request.url?.host() ?? "an unknown host")
            return nil
        }
        return request
    }
}
