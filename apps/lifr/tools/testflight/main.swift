// Talks to the App Store Connect API for testflight.sh: registers the
// bundle IDs, makes the distribution certificate and App Store profiles,
// and finds the app.
//
// The API key comes on standard input, never as an argument or a file: its
// Key ID, its Issuer ID, then the .p8's PEM, one per line, as setup.sh
// stored them in the Keychain.

import CryptoKit
import Foundation

enum Failure: Error, CustomStringConvertible {
    case usage
    case key(String)
    case request(method: String, path: String, status: Int, body: String)
    case transport(path: String, reason: String)
    case missingApp(bundleID: String, found: [String])
    case unexpected(String)

    var description: String {
        switch self {
        case .usage:
            """
            usage: asc <command> [arguments], with the key on standard input
              app <bundle ID>
              build <bundle ID> <build number>
              register <bundle ID> <name>
              capability <bundle ID> <capability type>
              certificates
              create-certificate <CSR path> <certificate output path>
              profile <name> <bundle ID> <certificate ID> <profile output path>
            """
        case .key(let reason):
            "The App Store Connect key in the Keychain can't be used (\(reason)). Run setup.sh again with the .p8."
        case .request(let method, let path, let status, let body):
            switch status {
            case 401:
                """
                App Store Connect refused the key (401 on \(method) \(path)). Check that the key hasn't \
                been revoked under Users and Access › Integrations, then run setup.sh again with its Key \
                ID and Issuer ID.
                """
            case 403:
                """
                The key isn't allowed to \(method) \(path) (403). Give it the Admin or App Manager role, \
                and accept any pending agreement in App Store Connect yourself. \(body)
                """
            case 429:
                "App Store Connect is limiting requests (429). Wait an hour, then run it again."
            default:
                "\(method) \(path) failed with \(status): \(body)"
            }
        case .transport(let path, let reason):
            "Couldn't reach App Store Connect for \(path): \(reason). Check the connection and run it again."
        case .missingApp(let bundleID, let found):
            """
            There's no app for \(bundleID) in App Store Connect yet. Make it yourself under Apps › + › \
            New App: iOS, bundle ID \(bundleID), any name and SKU. Then run this again.
            """
                + (found.isEmpty
                    ? " The API key sees no apps at all: check its team and that its role can see apps."
                    : " The apps the API key sees are for " + found.joined(separator: ", ") + ".")
        case .unexpected(let what):
            "App Store Connect answered unexpectedly: \(what)."
        }
    }
}

struct Key {
    let id: String
    let issuer: String
    let signing: P256.Signing.PrivateKey

    static func fromStandardInput() throws(Failure) -> Key {
        guard let input = String(bytes: FileHandle.standardInput.readDataToEndOfFile(), encoding: .utf8) else {
            throw .key("it isn't text")
        }
        let lines = input.split(separator: "\n", maxSplits: 2, omittingEmptySubsequences: false)
        guard lines.count == 3 else { throw .key("expected the Key ID, the Issuer ID and the key") }
        do {
            return Key(
                id: lines[0].trimmingCharacters(in: .whitespaces),
                issuer: lines[1].trimmingCharacters(in: .whitespaces),
                signing: try P256.Signing.PrivateKey(pemRepresentation: String(lines[2])))
        } catch {
            throw .key("the .p8 isn't a P-256 private key")
        }
    }

    /// A token good for 15 minutes.
    func token() throws(Failure) -> String {
        func encode(_ object: [String: Any]) throws(Failure) -> String {
            do {
                return base64URL(try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]))
            } catch {
                throw .key("its token couldn't be made")
            }
        }
        let now = Int(Date().timeIntervalSince1970)
        let header = try encode(["alg": "ES256", "kid": id, "typ": "JWT"])
        let payload = try encode(["iss": issuer, "iat": now, "exp": now + 900, "aud": "appstoreconnect-v1"])
        let signed = "\(header).\(payload)"
        do {
            return "\(signed).\(base64URL(try signing.signature(for: Data(signed.utf8)).rawRepresentation))"
        } catch {
            throw .key("it couldn't sign")
        }
    }
}

func base64URL(_ data: Data) -> String {
    data.base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_")
        .replacingOccurrences(of: "=", with: "")
}

struct API {
    let key: Key

    @discardableResult
    func call(_ method: String, _ path: String, _ body: [String: Any]? = nil) async throws(Failure) -> [String: Any] {
        guard let url = URL(string: "https://api.appstoreconnect.apple.com" + path) else {
            throw .unexpected("bad path \(path)")
        }
        var request = URLRequest(url: url)
        request.httpMethod = method
        request.setValue("Bearer \(try key.token())", forHTTPHeaderField: "Authorization")
        if let body {
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            request.httpBody = try? JSONSerialization.data(withJSONObject: body)
        }
        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await URLSession.shared.data(for: request)
        } catch {
            throw .transport(path: path, reason: error.localizedDescription)
        }
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard (200..<300).contains(status) else {
            throw .request(method: method, path: path, status: status, body: String(bytes: data, encoding: .utf8) ?? "")
        }
        if data.isEmpty { return [:] }
        guard let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
            throw .unexpected("a body that isn't JSON from \(path)")
        }
        return object
    }

    func list(_ path: String) async throws(Failure) -> [[String: Any]] {
        try await call("GET", path)["data"] as? [[String: Any]] ?? []
    }
}

func query(_ value: String) -> String {
    value.addingPercentEncoding(withAllowedCharacters: .alphanumerics) ?? value
}

func attributes(_ object: [String: Any]) -> [String: Any] { object["attributes"] as? [String: Any] ?? [:] }

func bundleIDResource(_ api: API, _ identifier: String) async throws(Failure) -> String? {
    // The identifier filter misses some IDs Xcode registered, so match
    // exactly over all of them.
    let found = try await api.list("/v1/bundleIds?limit=200")
    return found.first { attributes($0)["identifier"] as? String == identifier }?["id"] as? String
}

func run(_ arguments: [String]) async throws(Failure) {
    guard let command = arguments.first else { throw .usage }
    let api = API(key: try Key.fromStandardInput())
    switch (command, arguments.count) {
    case ("app", 2):
        // Like the bundle ID filter, the app filter can miss: match exactly
        // over all of them.
        let apps = try await api.list("/v1/apps?limit=200")
        guard let id = apps.first(where: { attributes($0)["bundleId"] as? String == arguments[1] })?["id"] as? String
        else {
            throw .missingApp(bundleID: arguments[1], found: apps.compactMap { attributes($0)["bundleId"] as? String })
        }
        print(id)

    case ("build", 3):
        // Where Apple is with an uploaded build: PROCESSING, VALID once
        // TestFlight can offer it, FAILED or INVALID; or NOT_LISTED while
        // Apple hasn't listed it yet.
        let apps = try await api.list("/v1/apps?limit=200")
        guard let app = apps.first(where: { attributes($0)["bundleId"] as? String == arguments[1] })?["id"] as? String
        else {
            throw .missingApp(bundleID: arguments[1], found: apps.compactMap { attributes($0)["bundleId"] as? String })
        }
        // A build number is digits and dots, which go into the query as
        // they are: Apple fails on an encoded dot.
        guard arguments[2].allSatisfy({ $0.isNumber || $0 == "." }) else { throw .usage }
        let builds = try await api.list("/v1/builds?filter[app]=\(query(app))&filter[version]=\(arguments[2])")
        print(builds.first.flatMap { attributes($0)["processingState"] as? String } ?? "NOT_LISTED")

    case ("register", 3):
        if let id = try await bundleIDResource(api, arguments[1]) {
            print(id)
            return
        }
        let created = try await api.call(
            "POST", "/v1/bundleIds",
            [
                "data": [
                    "type": "bundleIds",
                    "attributes": ["identifier": arguments[1], "name": arguments[2], "platform": "IOS"],
                ]
            ])
        guard let id = (created["data"] as? [String: Any])?["id"] as? String else {
            throw .unexpected("no ID for the new bundle ID")
        }
        print(id)

    case ("capability", 3):
        // Turns a capability on for a registered bundle ID, once.
        let (identifier, type) = (arguments[1], arguments[2])
        guard let bundle = try await bundleIDResource(api, identifier) else {
            throw .unexpected("\(identifier) isn't registered")
        }
        let enabled = try await api.list("/v1/bundleIds/\(bundle)/bundleIdCapabilities?limit=200")
        guard !enabled.contains(where: { attributes($0)["capabilityType"] as? String == type }) else { return }
        try await api.call(
            "POST", "/v1/bundleIdCapabilities",
            [
                "data": [
                    "type": "bundleIdCapabilities",
                    "attributes": ["capabilityType": type],
                    "relationships": ["bundleId": ["data": ["type": "bundleIds", "id": bundle]]],
                ]
            ])
        FileHandle.standardError.write(Data("Turned on \(type) for \(identifier).\n".utf8))

    case ("certificates", 1):
        for certificate in try await api.list("/v1/certificates?filter[certificateType]=DISTRIBUTION&limit=200") {
            guard let id = certificate["id"] as? String,
                let content = attributes(certificate)["certificateContent"] as? String,
                let der = Data(base64Encoded: content)
            else { continue }
            let sha1 = Insecure.SHA1.hash(data: der).map { String(format: "%02X", $0) }.joined()
            print(id, sha1)
        }

    case ("create-certificate", 3):
        guard let csr = try? String(contentsOfFile: arguments[1], encoding: .utf8) else {
            throw .unexpected("no CSR at \(arguments[1])")
        }
        let created = try await api.call(
            "POST", "/v1/certificates",
            [
                "data": [
                    "type": "certificates",
                    "attributes": ["certificateType": "DISTRIBUTION", "csrContent": csr],
                ]
            ])
        guard let data = created["data"] as? [String: Any], let id = data["id"] as? String,
            let content = attributes(data)["certificateContent"] as? String,
            let der = Data(base64Encoded: content)
        else { throw .unexpected("no certificate in the answer") }
        do {
            try der.write(to: URL(filePath: arguments[2]))
        } catch {
            throw .unexpected("the certificate couldn't be written: \(error.localizedDescription)")
        }
        print(id)

    case ("profile", 5):
        let (name, identifier, certificate, output) = (arguments[1], arguments[2], arguments[3], arguments[4])
        guard let bundle = try await bundleIDResource(api, identifier) else {
            throw .unexpected("\(identifier) isn't registered")
        }
        var content: String?
        // Names must be unique across every kind of profile, and the name
        // filter misses some, so look through them all.
        let existing = try await api.list("/v1/profiles?limit=200&include=certificates")
        for profile in existing where attributes(profile)["name"] as? String == name {
            let certificates =
                ((profile["relationships"] as? [String: Any])?["certificates"] as? [String: Any])?["data"]
                as? [[String: Any]] ?? []
            let current =
                attributes(profile)["profileState"] as? String == "ACTIVE"
                && attributes(profile)["profileType"] as? String == "IOS_APP_STORE"
                && certificates.contains { $0["id"] as? String == certificate }
            if current, content == nil {
                content = attributes(profile)["profileContent"] as? String
            } else if let id = profile["id"] as? String {
                // Signed by another certificate or expired: make a new one.
                try await api.call("DELETE", "/v1/profiles/\(id)")
            }
        }
        if content == nil {
            let created = try await api.call(
                "POST", "/v1/profiles",
                [
                    "data": [
                        "type": "profiles",
                        "attributes": ["name": name, "profileType": "IOS_APP_STORE"],
                        "relationships": [
                            "bundleId": ["data": ["type": "bundleIds", "id": bundle]],
                            "certificates": ["data": [["type": "certificates", "id": certificate]]],
                        ],
                    ]
                ])
            content = attributes(created["data"] as? [String: Any] ?? [:])["profileContent"] as? String
        }
        guard let content, let profile = Data(base64Encoded: content) else {
            throw .unexpected("no profile content for \(name)")
        }
        do {
            try profile.write(to: URL(filePath: output))
        } catch {
            throw .unexpected("the profile couldn't be written: \(error.localizedDescription)")
        }

    default:
        throw .usage
    }
}

do {
    try await run(Array(CommandLine.arguments.dropFirst()))
} catch {
    FileHandle.standardError.write(Data("asc: \(error)\n".utf8))
    exit(1)
}
