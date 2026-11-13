import ComposableArchitecture
import Foundation
import HumataTerminal
import XCTest

@MainActor
final class TerminalSettingsFeatureTests: XCTestCase {
    /// A store over a kept style, with every style saved since.
    private func makeStore(
        kept: TerminalStyle = TerminalStyle(), fonts: [String] = ["Courier", "Menlo", "Monaco"]
    ) -> (TestStoreOf<TerminalSettingsFeature>, LockIsolated<[TerminalStyle]>) {
        let saved = LockIsolated<[TerminalStyle]>([])
        let store = TestStore(initialState: TerminalSettingsFeature.State()) {
            TerminalSettingsFeature()
        } withDependencies: {
            $0.terminalStyle = TerminalStyleClient(
                load: { kept }, save: { style in saved.withValue { $0.append(style) } }, fontFamilies: { fonts })
        }
        return (store, saved)
    }

    func testThePaneShowsTheKeptStyleAndTheFonts() async {
        let kept = TerminalStyle(fontFamily: "Monaco", fontSize: 15, theme: .harbor)
        let (store, saved) = makeStore(kept: kept)

        await store.send(.task) { $0.style = kept }
        await store.receive(\.fontFamiliesListed) { $0.fontFamilies = ["Courier", "Menlo", "Monaco"] }
        XCTAssertEqual(saved.value, [])
    }

    func testAKeptFontThatIsNoLongerInstalledIsStillOffered() async {
        let kept = TerminalStyle(fontFamily: "Departed Mono")
        let (store, _) = makeStore(kept: kept)

        await store.send(.task) { $0.style = kept }
        await store.receive(\.fontFamiliesListed) {
            $0.fontFamilies = ["Departed Mono", "Courier", "Menlo", "Monaco"]
        }
    }

    func testEachChangeIsKept() async {
        let (store, saved) = makeStore()

        await store.send(.fontFamilyPicked("Courier")) { $0.style.fontFamily = "Courier" }
        await store.send(.fontSizeChanged(18)) { $0.style.fontSize = 18 }
        await store.send(.themePicked(TerminalTheme.highContrast.id)) { $0.style.theme = .highContrast }
        await store.finish()

        XCTAssertEqual(saved.value.last, TerminalStyle(fontFamily: "Courier", fontSize: 18, theme: .highContrast))
        XCTAssertEqual(saved.value.count, 3)
    }

    func testTheSizeStaysWithinWhatSettingsOffers() async {
        let (store, saved) = makeStore()

        await store.send(.fontSizeChanged(2)) { $0.style.fontSize = TerminalStyle.fontSizes.lowerBound }
        await store.send(.fontSizeChanged(90)) { $0.style.fontSize = TerminalStyle.fontSizes.upperBound }
        await store.finish()

        XCTAssertEqual(
            saved.value.map(\.fontSize), [TerminalStyle.fontSizes.lowerBound, TerminalStyle.fontSizes.upperBound])
    }

    func testAThemeThatDoesNotExistChangesNothing() async {
        let (store, saved) = makeStore()

        await store.send(.themePicked("no-such-theme"))
        await store.finish()

        XCTAssertEqual(saved.value, [])
    }
}
