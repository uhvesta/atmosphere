import ComposableArchitecture
import LifrLockClient
import SwiftUI

/// What Lifr shows while it's locked: nothing of the records, and a way to
/// unlock.
struct LockView: View {
    var store: StoreOf<RootFeature>

    var body: some View {
        VStack(spacing: 16) {
            Image(systemName: "lock.fill")
                .font(.system(size: 44))
                .foregroundStyle(.secondary)
                .accessibilityHidden(true)
            Text("Lifr is locked")
                .font(.title2.bold())
            if let message = store.lockMessage {
                Text(message)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: 360)
            }
            Button(Self.unlockTitle(store.biometry)) { store.send(.unlockTapped) }
                .buttonStyle(.borderedProminent)
                .disabled(store.isUnlocking)
        }
        .padding(32)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(.background)
    }

    static func unlockTitle(_ biometry: Biometry) -> LocalizedStringKey {
        switch biometry {
        case .faceID: "Unlock with Face ID"
        case .touchID: "Unlock with Touch ID"
        case .opticID: "Unlock with Optic ID"
        case .passcode: "Unlock"
        }
    }
}

/// Covers the window while the iPhone is in the app switcher.
struct PrivacyCover: View {
    var body: some View {
        Image(systemName: "lock.fill")
            .font(.system(size: 44))
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(.background)
            .accessibilityHidden(true)
    }
}
