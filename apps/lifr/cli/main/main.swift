import Foundation
import LifrAPI
import LifrCLI

setvbuf(stdout, nil, _IOLBF, 0)
let cli = CLI(
    connect: { () throws(APIError) in try SocketConnection.open(client: "lifr \(CLI.version)") },
    standardOutput: { FileHandle.standardOutput.write(Data($0.utf8)) },
    standardError: { FileHandle.standardError.write(Data($0.utf8)) },
    readLine: { Swift.readLine(strippingNewline: true) })
exit(cli.run(Array(CommandLine.arguments.dropFirst())))
