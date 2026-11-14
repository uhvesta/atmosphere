import Foundation
import HumataDaemonClient
import HumataTerminal
import XCTest

final class TerminalCommandTests: XCTestCase {
    private func terminal(id: String, host: String?) -> Terminal {
        Terminal(id: id, host: host, sessionID: nil, folder: "/tmp", shell: "/bin/zsh", opened: Date())
    }

    /// The words a shell makes of `line`, one per line of its output.
    private func words(_ line: String) throws -> [String] {
        let shell = Process()
        shell.executableURL = URL(filePath: "/bin/sh")
        shell.arguments = ["-c", "set -- \(line); printf '%s\\n' \"$@\""]
        let output = Pipe()
        shell.standardOutput = output
        try shell.run()
        let data = output.fileHandleForReading.readDataToEndOfFile()
        shell.waitUntilExit()
        // swiftlint:disable:next optional_data_string_conversion
        return String(decoding: data, as: UTF8.self).split(separator: "\n").map(String.init)
    }

    func testAShellReadsTheAttachCommandAsItsWordsWhateverTheyHold() throws {
        let executable = "/Applications/It's Humata $HOME.app/Contents/Helpers/humata"
        let command = TerminalCommand.attach(
            terminal(id: "t 1;echo", host: "build box \"a\""), executable: executable, socket: "/tmp/a b.sock")

        XCTAssertEqual(
            try words(command.line), [executable, "terminal", "attach", "t 1;echo", "--host", "build box \"a\""])
        XCTAssertEqual(command.environment["HUMATA_SOCKET"], "/tmp/a b.sock")
    }

    func testATerminalOnThisMacNamesNoHostAndTheDefaultSocketSetsNone() throws {
        let command = TerminalCommand.attach(terminal(id: "t1", host: nil), executable: "/humata", socket: nil)

        XCTAssertEqual(try words(command.line), ["/humata", "terminal", "attach", "t1"])
        XCTAssertNil(command.environment["HUMATA_SOCKET"])
    }
}
