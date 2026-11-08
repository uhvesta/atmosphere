import ComposableArchitecture
import FlowKit
import LifrAPI
import LifrAssistantClient
import LifrAssistantLive
import LifrBugReport
import LifrConversations
import LifrCore
import LifrGlance
import LifrGlanceClient
import LifrLockClient
import LifrModelsClient
import LifrRemindersClient
import LifrRoot
import LifrSearchClient
import LifrSessionClient
import LifrShortcuts
import LifrStoreClient
import LifrSyncClient
import LifrWidgetViews
import SwiftUI
import XCTest

/// Settings on the iPhone: a bug report, the settings, reviewing sync and the lock.
@MainActor
final class PhoneSettingsFlowTests: PhoneFlowTestCase {
    /// Shaking to record the steps to a bug: finishing a task while it
    /// records, shaking again to stop, describing it, copying it without
    /// the task's title, saving it, then finding it in Settings.
    func testBugReport() async throws {
        let (store, flow) = app("bug-report")
        await flow.step("launch", image: false)
        store.send(.shaken)
        XCTAssertTrue(store.isBugMenuShown)
        store.isBugMenuShown = false
        store.send(.recordStepsTapped)
        await flow.step("recording")
        let water = try task("Water the plants", in: store)
        store.send(.today(.list(.toggled(water))))
        await flow.step("acted")
        store.send(.shaken)
        await flow.step("describe")
        XCTAssertFalse(store.isRecordingSteps)
        XCTAssertEqual(store.bugReport?.report.recording?.steps.isEmpty, false)
        store.send(.bugReport(.presented(.binding(.set(\.report.happened, "Finishing a task hid the list.")))))
        store.send(.bugReport(.presented(.binding(.set(\.report.expected, "The list stays.")))))
        await flow.step("described")
        store.send(.bugReport(.presented(.copyTapped)))
        await flow.step("copied", image: false)
        let text = try XCTUnwrap(copied.value)
        XCTAssertTrue(text.contains("Finishing a task hid the list."))
        XCTAssertFalse(text.contains("Water the plants"))
        store.send(.bugReport(.presented(.saveTapped)))
        await flow.step("saved", image: false)
        XCTAssertNil(store.bugReport)
        store.isSettingsShown = true
        store.send(.settings(.bugReportsTapped))
        await flow.step("list")
        let saved = try await bugReports.load().reports
        XCTAssertEqual(saved.map(\.happened), ["Finishing a task hid the list."])
        flow.finish()
    }

    /// Settings, its Activity, Models (with a larger window for Gemma) and
    /// Skills.
    func testSettings() async {
        let (store, flow) = app("settings")
        store.isSettingsShown = true
        await flow.step("opened")
        store.send(.syncChanged(SyncState(status: .failed(.noAccount))))
        await flow.step("sync-failed")
        store.send(.settings(.activityTapped))
        await flow.step("activity")
        store.send(.settings(.activity(.presented(.olderTapped))))
        await flow.step("activity-older")
        store.send(.settings(.activity(.dismiss)))
        store.send(.settings(.modelsTapped))
        await flow.step("models")
        store.send(.settings(.models(.presented(.windowPicked(32_768, for: ModelCatalog.gemma4E2B.id)))))
        await flow.step("models-window")
        store.send(.settings(.models(.presented(.measureTapped(ModelCatalog.gemma4E2B.id)))))
        flow.scrollToEnd()
        await flow.step("models-measured")
        store.send(.settings(.models(.presented(.backendPicked(.cpu, for: ModelCatalog.gemma4E2B.id)))))
        await flow.step("models-cpu")
        store.send(.settings(.models(.dismiss)))
        store.send(.settings(.skillsTapped))
        await flow.step("skills")
        store.send(.settings(.skills(.presented(.newTapped))))
        await flow.step("new-skill")
        store.send(.settings(.skills(.dismiss)))
        store.send(.settings(.instructionsTapped))
        await flow.step("instructions")
        store.send(.settings(.instructions(.presented(.binding(.set(\.draft, "Answer in Spanish."))))))
        store.send(.settings(.instructions(.presented(.saveTapped))))
        await flow.step("instructions-saved")
        store.send(.settings(.instructions(.presented(.useTapped(nil)))))
        await flow.step("instructions-default")
        flow.finish()
    }

    /// Two records changed on this device and the Mac in ways that couldn't
    /// merge: the banner on Today, the review, a choice, then Later.
    func testSyncReview() async throws {
        let sample = Review.sample(now: now, timeZone: .gmt)
        let state = SyncState(lastSynced: now, review: sample.items)
        let sync = SyncClient(
            states: { AsyncStream { $0.yield(state) } }, sync: { state }, resolve: { _ in state }, changed: {})
        let (store, flow) = app("sync-review", workspace: sample.workspace, sync: sync)
        store.send(.syncChanged(state))
        await flow.step("banner")
        store.send(.syncReviewTapped)
        await flow.step("review")
        store.send(.syncReview(.presented(.chose(sample.items[0].id, .avenue, .theirs))))
        await flow.step("chosen")
        store.send(.syncReview(.presented(.laterTapped)))
        await flow.step("later")
        flow.finish()
    }
    /// The lock: turning it on in Settings, coming back after a few
    /// minutes away to the locked app, a try Face ID turns down, then in.
    func testLock() async {
        let (store, flow) = app("lock", lock: .scripted([nil, .cancelled, .failed("Face ID didn't recognise you")]))
        store.isSettingsShown = true
        store.send(.settings(.lockToggled(true)))
        await flow.step("turned-on")
        store.isSettingsShown = false
        store.send(.backgrounded)
        time.setValue(now + 5 * 60)
        store.send(.foregrounded)
        await flow.step("locked")
        store.send(.unlockTapped)
        await flow.step("not-recognised")
        store.send(.unlockTapped)
        await flow.step("unlocked", image: false)
        flow.finish()
    }
}
