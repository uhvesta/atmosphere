#if os(macOS)
    import Foundation
    import LifrSessionClient
    import ServiceManagement

    /// Each API request: who, what and how it went. Kept for 90 days, on this
    /// Mac only, readable only by you.
    struct AccessLog: Sendable {
        struct Entry: Hashable, Codable, Sendable {
            var at: Date
            /// The launcher's name, or `nil` when the socket gate refused it.
            var client: String?
            var method: String?
            var tasks: [String]
            /// `ok`, or the error code.
            var result: String
        }

        static let retention: TimeInterval = 90 * 24 * 60 * 60

        var file: URL

        func append(_ entry: Entry) {
            guard var line = try? JSONEncoder.log.encode(entry) else { return }
            line.append(UInt8(ascii: "\n"))
            try? FileManager.default.createDirectory(
                at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
            let descriptor = open(file.path, O_WRONLY | O_APPEND | O_CREAT, 0o600)
            guard descriptor >= 0 else { return }
            defer { close(descriptor) }
            _ = line.withUnsafeBytes { write(descriptor, $0.baseAddress, $0.count) }
        }

        /// Every entry, oldest first.
        func entries() -> [Entry] {
            guard let data = try? Data(contentsOf: file) else { return [] }
            return data.split(separator: UInt8(ascii: "\n")).compactMap {
                try? JSONDecoder.log.decode(Entry.self, from: Data($0))
            }
        }

        /// Drops entries older than 90 days before `now`.
        func prune(now: Date) {
            let all = entries()
            let kept = all.filter { $0.at > now - Self.retention }
            guard kept.count < all.count else { return }
            var data = Data()
            for entry in kept {
                guard let line = try? JSONEncoder.log.encode(entry) else { continue }
                data.append(line)
                data.append(UInt8(ascii: "\n"))
            }
            try? data.write(to: file, options: .atomic)
            _ = chmod(file.path, 0o600)
        }
    }

    extension JSONEncoder {
        fileprivate static var log: JSONEncoder {
            let encoder = JSONEncoder()
            encoder.dateEncodingStrategy = .iso8601
            encoder.outputFormatting = .sortedKeys
            return encoder
        }
    }

    extension JSONDecoder {
        fileprivate static var log: JSONDecoder {
            let decoder = JSONDecoder()
            decoder.dateDecodingStrategy = .iso8601
            return decoder
        }
    }

    /// Links the app's bundled `lifr` onto the PATH.
    struct CLILink: Sendable {
        /// The app's `Contents/Helpers/lifr`.
        var helper: URL
        /// Where to link, in order: the first that exists and is writable,
        /// else the last, created.
        var directories: [URL]

        static func live(helper: URL, home: URL) -> CLILink {
            CLILink(
                helper: helper,
                directories: [URL(fileURLWithPath: "/usr/local/bin"), home.appendingPathComponent(".local/bin")])
        }

        func installation() -> CLIInstallation {
            for directory in directories {
                let link = directory.appendingPathComponent("lifr")
                if isOurs(link) { return .installed(path: link.path) }
            }
            return .notInstalled
        }

        func install() throws(SessionError) -> CLIInstallation {
            if case .installed = installation() { return installation() }
            let files = FileManager.default
            let writable = directories.first { files.isWritableFile(atPath: $0.path) }
            guard let directory = writable ?? directories.last else {
                throw .installFailed(path: "lifr", reason: "Lifr has no folder to put it in")
            }
            let link = directory.appendingPathComponent("lifr")
            do {
                try files.createDirectory(at: directory, withIntermediateDirectories: true)
            } catch {
                throw .installFailed(path: link.path, reason: error.localizedDescription)
            }
            if (try? files.attributesOfItem(atPath: link.path)) != nil {
                throw .installFailed(
                    path: link.path,
                    reason: "another lifr is already there. Move it aside if you don't need it, then install again")
            }
            do {
                try files.createSymbolicLink(at: link, withDestinationURL: helper)
            } catch {
                throw .installFailed(path: link.path, reason: error.localizedDescription)
            }
            return .installed(path: link.path)
        }

        private func isOurs(_ link: URL) -> Bool {
            guard (try? FileManager.default.destinationOfSymbolicLink(atPath: link.path)) != nil else { return false }
            return link.resolvingSymlinksInPath().path == helper.resolvingSymlinksInPath().path
        }
    }

    /// Whether macOS opens Lifr at login.
    struct LoginItem: Sendable {
        var isEnabled: @Sendable () -> Bool
        var setEnabled: @Sendable (_ enabled: Bool) throws(SessionError) -> Void

        static let mainApp = LoginItem(
            isEnabled: { SMAppService.mainApp.status == .enabled },
            setEnabled: { (enabled) throws(SessionError) in
                do {
                    if enabled {
                        try SMAppService.mainApp.register()
                    } else {
                        try SMAppService.mainApp.unregister()
                    }
                } catch {
                    throw .loginItemFailed(reason: error.localizedDescription)
                }
            })
    }
#endif
