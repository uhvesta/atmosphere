import ComposableArchitecture
import Foundation

/// The device and app a report is made on, apart from the assistant's
/// model, which the report fills in from the model library.
package struct DeviceInfoClient: Sendable {
    package var current: @Sendable () -> DeviceInfo

    package init(current: @escaping @Sendable () -> DeviceInfo) {
        self.current = current
    }
}

extension DeviceInfoClient: DependencyKey {
    package static let liveValue = DeviceInfoClient {
        let model = hardwareModel()
        let info = Bundle.main.infoDictionary ?? [:]
        let version = info["CFBundleShortVersionString"] as? String ?? "?"
        let build = info["CFBundleVersion"] as? String ?? "?"
        #if os(iOS)
            let platform = "iOS"
        #else
            let platform = "macOS"
        #endif
        let os = ProcessInfo.processInfo.operatingSystemVersion
        return DeviceInfo(
            model: model,
            system: "\(platform) \(os.majorVersion).\(os.minorVersion).\(os.patchVersion)",
            appVersion: "\(version) (\(build))",
            language: Locale.preferredLanguages.first ?? Locale.current.identifier,
            timeZone: TimeZone.current.identifier,
            assistantModel: "")
    }

    package static let testValue = DeviceInfoClient {
        DeviceInfo(
            model: "iPhone18,1", system: "iOS 27.0.0", appVersion: "1.0 (1)", language: "en-US",
            timeZone: "America/Los_Angeles", assistantModel: "")
    }
}

extension DependencyValues {
    package var deviceInfo: DeviceInfoClient {
        get { self[DeviceInfoClient.self] }
        set { self[DeviceInfoClient.self] = newValue }
    }
}

/// "iPhone18,1" on an iPhone, "Mac16,10" on a Mac.
private func hardwareModel() -> String {
    #if os(macOS)
        var size = 0
        sysctlbyname("hw.model", nil, &size, nil, 0)
        var bytes = [UInt8](repeating: 0, count: size)
        sysctlbyname("hw.model", &bytes, &size, nil, 0)
    #else
        var system = utsname()
        uname(&system)
        let bytes = withUnsafeBytes(of: &system.machine) { Array($0) }
    #endif
    return String(bytes: bytes.prefix { $0 != 0 }, encoding: .utf8) ?? "unknown"
}
