import ChatUI
import ComposableArchitecture
import HumataAppFeature
import XCTest

@MainActor
final class TextSizeTests: XCTestCase {
    /// A store at `percent`, and every scale it keeps, oldest first.
    private func makeStore(at percent: Int = 100) -> (TestStoreOf<TextSizeFeature>, LockIsolated<[Int]>) {
        let kept = LockIsolated<[Int]>([])
        var state = TextSizeFeature.State()
        state.scale = TextScale(percent: percent)
        let store = TestStore(initialState: state) {
            TextSizeFeature()
        } withDependencies: {
            $0.textScaleKeeper = TextScaleKeeper(
                load: { .actual }, save: { scale in kept.withValue { $0.append(scale.percent) } })
        }
        return (store, kept)
    }

    /// Defaults of this test's own, emptied afterwards.
    private func emptyDefaults() throws -> UserDefaults {
        let suite = "TextSizeTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        addTeardownBlock { UserDefaults().removePersistentDomain(forName: suite) }
        return defaults
    }

    func testEachStepIsATenthOfTheUsualSizeAndIsKept() async {
        let (store, kept) = makeStore()

        await store.send(.biggerTapped) { $0.scale = TextScale(percent: 110) }
        await store.send(.biggerTapped) { $0.scale = TextScale(percent: 120) }
        await store.send(.smallerTapped) { $0.scale = TextScale(percent: 110) }
        await store.send(.smallerTapped) { $0.scale = .actual }
        await store.send(.smallerTapped) { $0.scale = TextScale(percent: 90) }
        await store.finish()

        XCTAssertEqual(kept.value, [110, 120, 110, 100, 90])
    }

    func testTextGrowsNoBiggerThanTheBiggest() async {
        let biggest = TextScale.percents.upperBound
        let (store, kept) = makeStore(at: biggest - 10)

        await store.send(.biggerTapped) { $0.scale = TextScale(percent: biggest) }
        XCTAssertFalse(store.state.scale.canGrow)
        await store.send(.biggerTapped)
        await store.finish()

        XCTAssertEqual(kept.value, [biggest])
    }

    func testTextShrinksNoSmallerThanTheSmallest() async {
        let smallest = TextScale.percents.lowerBound
        let (store, kept) = makeStore(at: smallest + 10)

        await store.send(.smallerTapped) { $0.scale = TextScale(percent: smallest) }
        XCTAssertFalse(store.state.scale.canShrink)
        await store.send(.smallerTapped)
        await store.finish()

        XCTAssertEqual(kept.value, [smallest])
    }

    func testActualSizeGoesBackToTheUsualSizeFromEitherSide() async {
        let (big, keptBig) = makeStore(at: 170)
        await big.send(.actualSizeTapped) { $0.scale = .actual }
        // Already there: nothing changes and nothing is kept again.
        await big.send(.actualSizeTapped)
        await big.finish()
        XCTAssertEqual(keptBig.value, [100])

        let (small, keptSmall) = makeStore(at: 80)
        await small.send(.actualSizeTapped) { $0.scale = .actual }
        await small.finish()
        XCTAssertEqual(keptSmall.value, [100])
    }

    func testAKeptScaleIsReadBack() throws {
        let defaults = try emptyDefaults()
        XCTAssertEqual(TextScaleKeeper.read(from: defaults), .actual)

        TextScaleKeeper.write(TextScale(percent: 130), to: defaults)
        XCTAssertEqual(TextScaleKeeper.read(from: defaults), TextScale(percent: 130))

        TextScaleKeeper.write(.actual, to: defaults)
        XCTAssertEqual(TextScaleKeeper.read(from: defaults), .actual)
    }

    func testAScaleBeyondTheLimitsIsHeldToThem() {
        XCTAssertEqual(TextScale(percent: 900).percent, TextScale.percents.upperBound)
        XCTAssertEqual(TextScale(percent: 5).percent, TextScale.percents.lowerBound)
        XCTAssertEqual(TextScale(percent: -40).percent, TextScale.percents.lowerBound)
    }

    func testTheWindowsFeatureKeepsTheScaleForItsViews() async {
        let store = TestStore(initialState: AppFeature.State()) {
            AppFeature()
        }
        await store.send(.textSize(.biggerTapped)) { $0.textSize.scale = TextScale(percent: 110) }
        await store.send(.textSize(.actualSizeTapped)) { $0.textSize.scale = .actual }
        await store.finish()
    }
}
