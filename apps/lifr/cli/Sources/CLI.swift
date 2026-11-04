import Foundation
import LifrAPI

/// `lifr`: runs one command line against the Lifr app.
package struct CLI {
    package static let version = "0.1.0"

    /// Opens the connection, when the command needs the app.
    package var connect: () throws(APIError) -> any Connection
    package var standardOutput: (String) -> Void
    package var standardError: (String) -> Void
    /// The next line of standard input, for `lifr mcp`.
    package var readLine: () -> String?
    /// For times in output meant for people.
    package var timeZone: TimeZone

    package init(
        connect: @escaping () throws(APIError) -> any Connection,
        standardOutput: @escaping (String) -> Void,
        standardError: @escaping (String) -> Void,
        readLine: @escaping () -> String? = { nil },
        timeZone: TimeZone = .current
    ) {
        self.connect = connect
        self.standardOutput = standardOutput
        self.standardError = standardError
        self.readLine = readLine
        self.timeZone = timeZone
    }

    /// Exit statuses: 0 for success, 1 when the app refused or failed the
    /// request, 2 for a command line lifr can't run.
    package func run(_ arguments: [String]) -> Int32 {
        let invocation: Invocation
        do {
            invocation = try Invocation(arguments)
        } catch {
            standardError("lifr: \(error)\n")
            return 2
        }
        switch invocation.command {
        case .help:
            standardOutput(Self.help)
            return 0
        case .version:
            standardOutput("lifr \(Self.version), protocol \(LifrProtocol.version)\n")
            return 0
        case .mcp:
            let server = MCPServer(connect: connect)
            while let line = readLine() {
                if let reply = server.reply(to: line) { standardOutput(reply + "\n") }
            }
            return 0
        case .request(let method, let params):
            do throws(APIError) {
                let result = try connect().call(method, params)
                do {
                    standardOutput(
                        invocation.json
                            ? try Output.json(result) : try Output.text(for: method, result, timeZone: timeZone))
                } catch {
                    throw APIError(
                        .internalError,
                        "The Lifr app answered in a shape lifr doesn't know: \(error). Update Lifr so the CLI and "
                            + "app match, or use --json.")
                }
                return 0
            } catch {
                if invocation.json {
                    standardOutput((try? Output.json(["error": RPCError(error)])) ?? "")
                } else {
                    var message = "lifr: \(error.message)\n"
                    if let tasks = error.tasks { message += "Tasks: \(tasks.joined(separator: ", "))\n" }
                    standardError(message)
                }
                return 1
            }
        }
    }

    static let help = """
        lifr: your Lifr tasks from the command line, through the Lifr app on this Mac.

        Usage: lifr <command> [--json]

        Reading:
          avenues                                List the Avenues.
          projects [AVENUE] [--closed]           List the Projects, in one Avenue or all.
          search [WORDS] [--avenue KEY] [--project ID] [--status STATUS]... [--closed] [--limit N]
                                                 Find open tasks, or closed ones too with --closed.
          show TASK                              A task with its Sub tasks, comments and history.

        Changing:
          add TITLE (--avenue KEY | --parent TASK) [--project ID] [--status STATUS] [FIELDS]
          edit TASK [--title TITLE] [FIELDS] [--clear start|due|time]...
          status TASK STATUS [--cascade]         STATUS is todo, in-progress, procrastinated, done or wont-do.
                                                 --cascade closes open Sub tasks with Won't Do.
          move TASK (--avenue KEY [--project ID] | --parent TASK | --top-level)
          comment TASK TEXT
          trash TASK                             The app asks you to confirm.

          FIELDS: --details TEXT --priority none|low|medium|high|urgent --start DATE --due DATE
                  --time HH:MM --label NAME...   Dates look like 2026-11-12.

        Sessions:
          unlock                                 Touch ID, then this terminal acts as you for 15 idle minutes.
          export                                 Every record as JSON, after Touch ID.
          mcp                                    Serve MCP over stdio: claude mcp add lifr -- lifr mcp

        --json prints the API's response as is. TASK is a code such as HEALTH-7K2Q, or just 7K2Q.

        """
}
