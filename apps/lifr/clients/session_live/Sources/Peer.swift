#if os(macOS)
    import Darwin
    import Foundation
    import LifrAPI
    import Security

    /// The app that ran `lifr`: the first ancestor that isn't an Apple-signed
    /// shell. Grants are bound to it.
    struct Launcher: Hashable, Sendable {
        /// Its code-signing identifier, such as `com.anthropic.claude-code`.
        var identity: String
        /// Its team ID, or `nil` for Apple's own programs.
        var team: String?
        var name: String
        /// A terminal app: its commands act as the person after `lifr unlock`.
        var isTerminal = false
        /// It runs scripts, which all share its identity.
        var isInterpreter = false
    }

    /// A `lifr` that passed the socket gate.
    struct Peer: Hashable, Sendable {
        var launcher: Launcher
        /// The CLI's controlling terminal, which keys terminal sessions.
        var tty: UInt32?
    }

    /// Decides who is on the other end of an accepted socket, or refuses it.
    struct PeerCheck: Sendable {
        var identify: @Sendable (_ socket: Int32) -> Result<Peer, APIError>
    }

    extension PeerCheck {
        /// The spec's gate: the peer must be `cli`, signed by `team` as
        /// `identifier`, and have a signed launcher.
        static func codeSigning(cli: URL, team: String, identifier: String) -> PeerCheck {
            PeerCheck { socket in CodeSigning(cli: cli, team: team, identifier: identifier).identify(socket) }
        }
    }

    private struct CodeSigning {
        var cli: URL
        var team: String
        var identifier: String

        /// Programs between the launcher and `lifr` that run commands for it.
        /// `login` sits between Terminal and its shell.
        static let passThrough: Set<String> = [
            "com.apple.zsh", "com.apple.bash", "com.apple.sh", "com.apple.dash", "com.apple.ksh", "com.apple.tcsh",
            "com.apple.env", "com.apple.login",
        ]

        static let terminals: Set<String> = [
            "com.apple.Terminal", "com.googlecode.iterm2", "com.github.wez.wezterm", "com.mitchellh.ghostty",
            "net.kovidgoyal.kitty", "dev.warp.Warp-Stable", "org.alacritty",
        ]

        static let interpreters = ["python", "osascript", "perl", "ruby", "node", "tclsh", "php"]

        func identify(_ socket: Int32) -> Result<Peer, APIError> {
            var token = audit_token_t()
            var size = socklen_t(MemoryLayout<audit_token_t>.size)
            guard getsockopt(socket, SOL_LOCAL, LOCAL_PEERTOKEN, &token, &size) == 0 else {
                return .failure(Self.notTheCLI)
            }
            let tokenData = withUnsafeBytes(of: &token) { Data($0) }
            let requirement =
                "anchor apple generic and certificate leaf[subject.OU] = \"\(team)\" and identifier \"\(identifier)\""
            guard let peer = Self.guest([kSecGuestAttributeAudit: tokenData]), Self.satisfies(peer, requirement) else {
                return .failure(Self.notTheCLI)
            }
            guard Self.path(of: peer)?.resolvingSymlinksInPath().path == cli.resolvingSymlinksInPath().path else {
                return .failure(
                    .notAuthorized(
                        "This lifr is a copy from outside the Lifr app, so it can't connect. Use the one Settings > "
                            + "Command line installs, which links to the app's own."))
            }
            // The audit token's sixth word is the pid.
            let pid = pid_t(bitPattern: token.val.5)
            let tty = Self.info(pid).map(\.e_tdev).flatMap { $0 == 0 || $0 == UInt32.max ? nil : $0 }
            return launcher(above: pid).map { Peer(launcher: $0, tty: tty) }
        }

        /// Walks up from `pid` past Apple-signed shells.
        private func launcher(above pid: pid_t) -> Result<Launcher, APIError> {
            var current = pid
            for _ in 0..<32 {
                guard let info = Self.info(current) else { break }
                let parent = pid_t(info.pbi_ppid)
                guard parent > 1 else { break }
                let code = Self.guest([kSecGuestAttributePid: parent])
                let signing = code.flatMap(Self.signing)
                let id = signing?[kSecCodeInfoIdentifier as String] as? String
                if let id, let code, Self.passThrough.contains(id), Self.satisfies(code, "anchor apple") {
                    current = parent
                    continue
                }
                let path = code.flatMap(Self.path)
                guard let id, let code, Self.satisfies(code, "anchor apple generic") else {
                    let name = path?.lastPathComponent ?? "The program that ran lifr"
                    return .failure(
                        .notAuthorized(
                            "\(name) isn't signed by an identified developer, so Lifr can't tell it from any other "
                                + "program and won't let it connect. Run lifr from a signed app, such as Terminal "
                                + "or the native install of Claude Code."))
                }
                let lowered = id.lowercased()
                return .success(
                    Launcher(
                        identity: id, team: signing?[kSecCodeInfoTeamIdentifier as String] as? String,
                        name: Self.name(path, id), isTerminal: Self.terminals.contains(id),
                        isInterpreter: Self.interpreters.contains { lowered.contains($0) }))
            }
            return .failure(
                .notAuthorized(
                    "The app that ran lifr has exited, so Lifr can't tell who is asking. Run the command again from "
                        + "the app or terminal."))
        }

        private static let notTheCLI = APIError.notAuthorized(
            "Only the lifr inside the Lifr app can connect. Run the lifr that Settings > Command line installs.")

        private static func guest(_ attributes: [CFString: Any]) -> SecCode? {
            var code: SecCode?
            guard SecCodeCopyGuestWithAttributes(nil, attributes as CFDictionary, [], &code) == errSecSuccess else {
                return nil
            }
            return code
        }

        private static func satisfies(_ code: SecCode, _ text: String) -> Bool {
            var requirement: SecRequirement?
            guard SecRequirementCreateWithString(text as CFString, [], &requirement) == errSecSuccess,
                let requirement
            else { return false }
            return SecCodeCheckValidity(code, [], requirement) == errSecSuccess
        }

        private static func staticCode(_ code: SecCode) -> SecStaticCode? {
            var staticCode: SecStaticCode?
            guard SecCodeCopyStaticCode(code, [], &staticCode) == errSecSuccess else { return nil }
            return staticCode
        }

        private static func path(of code: SecCode) -> URL? {
            guard let staticCode = staticCode(code) else { return nil }
            var url: CFURL?
            guard SecCodeCopyPath(staticCode, [], &url) == errSecSuccess else { return nil }
            return url as URL?
        }

        private static func signing(_ code: SecCode) -> [String: Any]? {
            guard let staticCode = staticCode(code) else { return nil }
            var info: CFDictionary?
            let flags = SecCSFlags(rawValue: kSecCSSigningInformation)
            guard SecCodeCopySigningInformation(staticCode, flags, &info) == errSecSuccess else { return nil }
            return info as? [String: Any]
        }

        private static func info(_ pid: pid_t) -> proc_bsdinfo? {
            var info = proc_bsdinfo()
            let size = Int32(MemoryLayout<proc_bsdinfo>.size)
            return proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &info, size) == size ? info : nil
        }

        /// The enclosing app's name, or one made from the identifier, such as
        /// "Claude Code" from `com.anthropic.claude-code`.
        private static func name(_ path: URL?, _ identifier: String) -> String {
            var url = path
            while let current = url, current.path != "/" {
                if current.pathExtension == "app", let bundle = Bundle(url: current),
                    let name = bundle.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String
                        ?? bundle.object(forInfoDictionaryKey: "CFBundleName") as? String
                {
                    return name
                }
                url = current.deletingLastPathComponent()
            }
            let last = identifier.split(separator: ".").last.map(String.init) ?? identifier
            return last.split(whereSeparator: { $0 == "-" || $0 == "_" }).map {
                $0.prefix(1).uppercased() + $0.dropFirst()
            }
            .joined(separator: " ")
        }
    }
#endif
