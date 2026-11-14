import ComposableArchitecture

/// Waits for the store's effects to end, then takes in the actions they
/// sent, so a non-exhaustive store's state shows what they did.
@MainActor
func settle<State: Equatable, Action>(_ store: TestStore<State, Action>) async {
    await store.finish()
    await store.skipReceivedActions(strict: false)
}
