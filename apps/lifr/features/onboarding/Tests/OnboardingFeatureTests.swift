import ComposableArchitecture
import Foundation
import LifrCore
import LifrLockClient
import LifrOnboarding
import LifrRemindersClient
import LifrStoreClient
import XCTest

@MainActor
final class OnboardingFeatureTests: XCTestCase {
    /// 2026-11-03 10:00 UTC, a Tuesday.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private func onboarding(_ workspace: Workspace = Workspace()) -> TestStoreOf<OnboardingFeature> {
        let store = TestStore(initialState: OnboardingFeature.State()) {
            OnboardingFeature()
        } withDependencies: { [now] in
            $0.store = .inMemory(workspace)
            $0.reminders = RemindersClient.inMemory(permission: .notDetermined).client
            $0.lock = .always()
            $0.date.now = now
            $0.uuid = .incrementing
            $0.withRandomNumberGenerator = WithRandomNumberGenerator(SystemRandomNumberGenerator())
        }
        store.exhaustivity = .off
        return store
    }

    func testFinishingWithTwoAvenuesCreatesExactlyThosePlusInboxAndAStarterTask() async throws {
        let store = onboarding()

        await store.send(.continueTapped)
        await store.send(.suggestionToggled("HOME"))
        await store.send(.suggestionToggled("WORK"))
        await store.send(.suggestionToggled("HEALTH"))
        await store.send(.suggestionToggled("HEALTH"))
        await store.send(.continueTapped)
        await store.send(.continueTapped)
        await store.receive(\.finished.success)
        await store.receive(\.delegate.finished)

        let workspace = try await store.dependencies.store.load()
        XCTAssertEqual(Set(workspace.avenues.values.map(\.key.rawValue)), ["INBOX", "HOME", "WORK"])
        XCTAssertEqual(workspace.tasks.count, 1)
        XCTAssertEqual(workspace.tasks.values.first?.avenueID, Avenue.inboxID)
        XCTAssertNotNil(workspace.tasks.values.first?.dueDate)
    }

    func testANamedAvenueGetsAKeyFromItsLetters() async throws {
        let store = onboarding()

        await store.send(.continueTapped)
        await store.send(\.binding.custom, "  Café plans ")
        await store.send(.continueTapped)
        await store.send(.continueTapped)
        await store.receive(\.delegate.finished)

        let workspace = try await store.dependencies.store.load()
        let named = workspace.avenues.values.first { $0.id != Avenue.inboxID }
        XCTAssertEqual(named?.name, "Café plans")
        XCTAssertEqual(named?.key.rawValue, "CAFEPLANS")
    }

    func testSkippingCreatesNothing() async throws {
        let store = onboarding()

        await store.send(.continueTapped)
        await store.send(.suggestionToggled("HOME"))
        await store.send(.skipTapped)
        await store.receive(\.delegate.finished)

        let workspace = try await store.dependencies.store.load()
        XCTAssertTrue(workspace.tasks.isEmpty)
        XCTAssertTrue(workspace.avenues.values.allSatisfy { $0.id == Avenue.inboxID })
    }

    func testRecordsArrivingFromAnotherDeviceSkipToFoundAndStartSavesNothing() async throws {
        let store = onboarding()

        await store.send(.continueTapped)
        await store.send(.foundExisting) { $0.step = .found }
        await store.send(.continueTapped)
        await store.receive(\.delegate.finished)

        let workspace = try await store.dependencies.store.load()
        XCTAssertTrue(workspace.tasks.isEmpty)
    }

    func testTurningOnTheLockNeedsTheDeviceToConfirm() async {
        let store = onboarding()
        store.dependencies.lock = .always(failing: .cancelled)

        await store.send(.lockTapped)
        await store.receive(\.lockAnswered)
        XCTAssertFalse(store.state.locksApp)

        store.dependencies.lock = .always()
        await store.send(.lockTapped)
        await store.receive(\.lockAnswered)
        XCTAssertTrue(store.state.locksApp)
    }

    func testAKeyIsTheNamesLettersOrMineWhenItHasTooFew() {
        XCTAssertEqual(OnboardingFeature.key(for: "Side projects and more"), "SIDEPROJEC")
        XCTAssertEqual(OnboardingFeature.key(for: "1 2"), "MINE")
    }
}
