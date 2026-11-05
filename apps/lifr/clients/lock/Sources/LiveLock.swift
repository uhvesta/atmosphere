import Foundation
import LocalAuthentication

extension LockClient {
    /// Face ID, Touch ID or Optic ID, falling back to the device's passcode
    /// or the Mac's password.
    package static func live() -> Self {
        LockClient(
            biometry: {
                let context = LAContext()
                _ = context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: nil)
                switch context.biometryType {
                case .faceID: return .faceID
                case .touchID: return .touchID
                case .opticID: return .opticID
                case .none: return .passcode
                @unknown default: return .passcode
                }
            },
            authenticate: { reason in
                let context = LAContext()
                var problem: NSError?
                guard context.canEvaluatePolicy(.deviceOwnerAuthentication, error: &problem) else {
                    throw LockError(problem)
                }
                do {
                    _ = try await context.evaluatePolicy(.deviceOwnerAuthentication, localizedReason: reason)
                } catch {
                    throw LockError(error as NSError)
                }
            })
    }
}

extension LockError {
    fileprivate init(_ error: NSError?) {
        guard let error else {
            self = .failed(String(localized: "the device didn't say why"))
            return
        }
        switch LAError.Code(rawValue: error.code) {
        case .userCancel, .appCancel, .systemCancel, .userFallback:
            self = .cancelled
        case .passcodeNotSet:
            self = .noPasscode
        default:
            self = .failed(error.localizedDescription)
        }
    }
}
