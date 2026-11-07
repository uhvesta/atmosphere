import ComposableArchitecture
import Foundation
import LifrAssistantClient
import LifrModelsClient
import LifrSkills
import XCTest

@MainActor
final class SkillsFeatureTests: XCTestCase {
    private func skills(_ client: SkillsClient) async -> TestStoreOf<SkillsFeature> {
        let store = TestStore(initialState: SkillsFeature.State()) {
            SkillsFeature()
        } withDependencies: {
            $0.skills = client
            $0.assistant = .scripted()
            $0.models = InMemoryModelLibrary().client
        }
        store.exhaustivity = .off
        await store.send(.task)
        await store.receive(\.loaded)
        return store
    }

    func testTurningABuiltInOffIsKept() async throws {
        let client = SkillsClient.inMemory()
        let store = await skills(client)

        await store.send(.toggled("undo", false))
        await store.receive(\.saved)

        XCTAssertEqual(store.state.skills.first { $0.id == "undo" }?.isOn, false)
        let stored = try await client.load()
        XCTAssertEqual(stored.first { $0.id == "undo" }?.isOn, false)
    }

    func testWhetherABuiltInIsPickedAutomaticallyIsKept() async throws {
        let client = SkillsClient.inMemory()
        let store = await skills(client)

        await store.send(.autoPicksToggled("breakdown", false))
        await store.receive(\.saved)
        await store.send(.autoPicksToggled("undo", true))
        await store.receive(\.saved)

        let stored = try await client.load()
        XCTAssertEqual(stored.first { $0.id == "breakdown" }?.autoPicks, false)
        XCTAssertEqual(stored.first { $0.id == "undo" }?.autoPicks, true)
    }

    func testANewSkillCanBePickedAutomaticallyWhenItFits() async throws {
        let client = SkillsClient.inMemory()
        let store = await skills(client)

        await store.send(.newTapped)
        await store.send(.editor(.presented(.binding(.set(\.name, "Weekly review")))))
        await store.send(.editor(.presented(.binding(.set(\.instructions, "Go through the week.")))))
        await store.send(.editor(.presented(.binding(.set(\.autoPicks, true)))))
        await store.send(.editor(.presented(.binding(.set(\.useWhen, " I ask about my week ")))))
        await store.send(.editor(.presented(.saveTapped)))
        await store.skipReceivedActions()

        let stored = try await client.load()
        let saved = try XCTUnwrap(stored.first { $0.id == "user:weekly-review" })
        XCTAssertTrue(saved.autoPicks)
        XCTAssertEqual(saved.useWhen, "I ask about my week")
    }

    func testANewSkillIsSavedUnderItsUserID() async throws {
        let client = SkillsClient.inMemory()
        let store = await skills(client)

        await store.send(.newTapped)
        await store.send(.editor(.presented(.binding(.set(\.name, "Weekly review")))))
        XCTAssertNil(store.state.editor?.skill)
        await store.send(.editor(.presented(.binding(.set(\.instructions, "Go through what's overdue.")))))
        await store.send(.editor(.presented(.toolToggled(.recentActivity, true))))
        await store.send(.editor(.presented(.saveTapped)))
        await store.skipReceivedActions()

        XCTAssertNil(store.state.editor)
        let saved = try XCTUnwrap(store.state.skills.first { $0.id == "user:weekly-review" })
        XCTAssertTrue(saved.tools.contains(.recentActivity))
        let stored = try await client.load()
        XCTAssertEqual(stored.last, saved)
    }

    func testANameThatsTakenCantBeSaved() async throws {
        let mine = try XCTUnwrap(
            Skill(userNamed: "Weekly", summary: "", instructions: "Be brief.", tools: [], draftsPlans: false))
        let store = await skills(.inMemory([mine]))

        await store.send(.newTapped)
        await store.send(.editor(.presented(.binding(.set(\.name, "weekly")))))
        await store.send(.editor(.presented(.binding(.set(\.instructions, "Something else.")))))

        XCTAssertNotNil(store.state.editor?.problem)
        XCTAssertEqual(store.state.editor?.canSave, false)
    }

    func testDeletingRemovesOnlyTheirOwnSkills() async throws {
        let mine = try XCTUnwrap(
            Skill(userNamed: "Weekly", summary: "", instructions: "Be brief.", tools: [], draftsPlans: false))
        let client = SkillsClient.inMemory([mine])
        let store = await skills(client)

        await store.send(.deleteTapped([mine.id, "query"]))
        await store.receive(\.saved)

        XCTAssertEqual(store.state.skills.map(\.id), Skill.builtIn.map(\.id))
    }

    func testTheAssistantDraftsOnlyWhenAskedAndNothingSavesUntilSave() async throws {
        let client = SkillsClient.inMemory()
        let store = await skills(client)

        await store.send(.newTapped)
        XCTAssertEqual(store.state.editor?.assists, false)
        await store.send(.editor(.presented(.assistTapped)))
        XCTAssertEqual(store.state.editor?.canDraft, false)
        await store.send(.editor(.presented(.binding(.set(\.request, "Review my week on Fridays")))))
        await store.send(.editor(.presented(.draftTapped)))
        await store.receive(\.editor.presented.drafted)

        XCTAssertEqual(store.state.editor?.instructions, SkillDraft.example.instructions)
        XCTAssertEqual(store.state.editor?.skill?.id, "user:weekly-review")
        let stored = try await client.load()
        XCTAssertEqual(stored.map(\.id), Skill.builtIn.map(\.id))
    }

    func testRedraftingAnExistingSkillKeepsItsName() async throws {
        let mine = try XCTUnwrap(
            Skill(userNamed: "Mine", summary: "", instructions: "Be brief.", tools: [], draftsPlans: false))
        let store = await skills(.inMemory([mine]))

        await store.send(.editTapped(mine.id))
        await store.send(.editor(.presented(.assistTapped)))
        await store.send(.editor(.presented(.binding(.set(\.request, "Make it about Fridays")))))
        await store.send(.editor(.presented(.draftTapped)))
        await store.receive(\.editor.presented.drafted)

        XCTAssertEqual(store.state.editor?.skill?.id, mine.id)
        XCTAssertEqual(store.state.editor?.summary, SkillDraft.example.summary)
    }

    func testADraftRunsOnTheModelPickedForItWithoutChangingTheDefault() async throws {
        let library = InMemoryModelLibrary(installed: [ModelCatalog.gemma4E2B])
        let used = LockIsolated<[ModelID?]>([])
        let store = TestStore(initialState: SkillsFeature.State()) {
            SkillsFeature()
        } withDependencies: {
            $0.skills = .inMemory()
            $0.assistant = .scripted()
            $0.assistant.draftSkill = { _, model in
                used.withValue { $0.append(model) }
                return .example
            }
            $0.models = library.client
        }
        store.exhaustivity = .off

        await store.send(.newTapped)
        await store.send(.editor(.presented(.assistTapped)))
        await store.receive(\.editor.presented.modelsLoaded)
        XCTAssertEqual(store.state.editor?.models.map(\.id), [.apple, ModelCatalog.gemma4E2B.id])
        await store.send(.editor(.presented(.binding(.set(\.model, ModelCatalog.gemma4E2B.id)))))
        await store.send(.editor(.presented(.binding(.set(\.request, "Review my week")))))
        await store.send(.editor(.presented(.draftTapped)))
        await store.receive(\.editor.presented.drafted)

        XCTAssertEqual(used.value, [ModelCatalog.gemma4E2B.id])
        XCTAssertEqual(library.client.selection(.skills), .apple)
    }

    func testWhenTheAssistantCantDraftItSaysWhyAndKeepsTheFields() async throws {
        let store = TestStore(initialState: SkillsFeature.State()) {
            SkillsFeature()
        } withDependencies: {
            $0.skills = .inMemory()
            $0.assistant.availability = { _ in .available }
            $0.assistant.draftSkill = { _, _ in throw AssistantError.declined }
            $0.models = InMemoryModelLibrary().client
        }
        store.exhaustivity = .off

        await store.send(.newTapped)
        await store.send(.editor(.presented(.binding(.set(\.instructions, "Mine.")))))
        await store.send(.editor(.presented(.assistTapped)))
        await store.send(.editor(.presented(.binding(.set(\.request, "Something")))))
        await store.send(.editor(.presented(.draftTapped)))
        await store.receive(\.editor.presented.drafted)

        XCTAssertNotNil(store.state.editor?.draftProblem)
        XCTAssertEqual(store.state.editor?.instructions, "Mine.")
        XCTAssertEqual(store.state.editor?.isDrafting, false)
    }
}
