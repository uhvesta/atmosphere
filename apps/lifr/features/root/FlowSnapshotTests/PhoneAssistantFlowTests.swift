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

/// The assistant on the iPhone drafting tasks: following up, repairing what the model writes, and breaking down a brain dump.
@MainActor
final class PhoneAssistantFlowTests: PhoneFlowTestCase {
    /// The assistant: a new conversation, the / list, a skill typed, a reply with a draft,
    /// adding it, renaming, then the list.
    func testAssistant() async throws {
        let (store, flow) = app("assistant")
        store.tab = .conversations
        await flow.step("list")
        store.send(.conversations(.newTapped))
        await flow.step("new")
        let id = try XCTUnwrap(store.conversations.openID)
        store.send(.conversations(.chats(.element(id: id, action: .core(.binding(.set(\.input, "/")))))))
        await flow.step("slash")
        // "/review " turns into the skill's name at the start of the field.
        store.send(.conversations(.chats(.element(id: id, action: .core(.binding(.set(\.input, "/review ")))))))
        await flow.step("skill-picked")
        store.send(.conversations(.chats(.element(id: id, action: .clearSkillTapped))))
        store.send(.conversations(.chats(.element(id: id, action: .core(.binding(.set(\.input, "Tidy my weekend")))))))
        store.send(.conversations(.chats(.element(id: id, action: .core(.sendTapped)))))
        await flow.step("replied-with-draft")
        store.send(.conversations(.chats(.element(id: id, action: .editDraftTapped))))
        await flow.step("edit-draft")
        store.send(.conversations(.chats(.element(id: id, action: .binding(.set(\.editsDraft, false))))))
        await flow.step("edited", image: false)
        store.send(.conversations(.chats(.element(id: id, action: .addTapped))))
        await flow.step("added")
        store.send(.conversations(.chats(.element(id: id, action: .renameTapped))))
        await flow.step("rename")
        store.send(.conversations(.rename(.dismiss)))
        store.send(.conversations(.closed))
        await flow.step("list-after")
        store.send(.conversations(.archiveTapped(id)))
        store.send(.conversations(.archiveShown(true)))
        await flow.step("archive")
        flow.finish()
    }

    /// Adding an Avenue from a chat, then asking for more in the same chat.
    /// The model's next draft repeats the Avenue just added and puts a task
    /// in an Avenue that isn't there. The draft leaves out what's already
    /// added, and removing the bad row in the editor lets the rest be added.
    func testAssistantFollowUpDraft() async throws {
        let fitness = Plan.AvenueDraft(key: "FIT", name: "Fitness", color: "green", symbol: "figure.run")
        let (store, flow) = app(
            "assistant-follow-up",
            replies: [
                AssistantReply(
                    text: "Here's a Fitness Avenue. Tap Add to make it.", plan: Plan(avenues: [fitness]),
                    isComplete: true),
                AssistantReply(
                    text: "I added Reading, and a run to start with.",
                    plan: Plan(
                        avenues: [fitness, .init(key: "READ", name: "Reading", color: "purple", symbol: "book")],
                        tasks: [.init(title: "Go for a run", place: "SPORTS")]),
                    isComplete: true),
            ])
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        store.send(
            .conversations(.chats(.element(id: id, action: .core(.binding(.set(\.input, "Add a Fitness Avenue")))))))
        store.send(.conversations(.chats(.element(id: id, action: .core(.sendTapped)))))
        await flow.step("first-draft")
        flow.tap("Add")
        await flow.step("first-added", image: false)
        XCTAssertNotNil(store.workspace?.avenues.values.first { $0.key.rawValue == "FIT" }, "Fitness wasn't added")

        store.send(
            .conversations(.chats(.element(id: id, action: .core(.binding(.set(\.input, "Now add a Reading Avenue"))))))
        )
        store.send(.conversations(.chats(.element(id: id, action: .core(.sendTapped)))))
        await flow.step("second-draft")
        let draft = try XCTUnwrap(store.conversations.chats[id: id]?.conversation.draft)
        XCTAssertEqual(draft.avenues.map(\.key), ["READ"], "The draft still has the Avenue that was added")

        flow.tap("Edit")
        await flow.step("edit-second-draft")
        // The Reading row's Remove, then the run's.
        flow.tap("Remove", occurrence: 1)
        await flow.step("bad-row-removed")
        let chat = try XCTUnwrap(store.conversations.chats[id: id])
        XCTAssertNil(chat.problem, "Removing the row the problem was about left the problem")
        XCTAssertTrue(chat.canAdd)
        store.send(.conversations(.chats(.element(id: id, action: .binding(.set(\.editsDraft, false))))))
        flow.tap("Add")
        await flow.step("second-added")
        XCTAssertNotNil(store.workspace?.avenues.values.first { $0.key.rawValue == "READ" }, "Reading wasn't added")
        flow.finish()
    }

    /// Adding to a draft, then changing and removing one of its tasks. The
    /// model writes the draft's task again with the new one, and changes
    /// it by number with a priority of none it wasn't asked for, and the
    /// first task for "that".
    func testAssistantFollowUpEdits() async throws {
        let (store, flow) = app(
            "assistant-follow-up-edits",
            assistant: gemma([
                [
                    "plan": ["tasks": [["title": "buy milk", "place": "HOME"]]],
                    "message": "I added a task to buy milk in HOME.",
                ],
                [
                    "draftChanges": [["task": "1", "moveTo": "HOME", "rename": "", "priority": "none"]],
                    "plan": [
                        "tasks": [["title": "buy milk", "place": "HOME"], ["title": "buy eggs", "place": "HOME"]]
                    ],
                    "message": "I added a task to buy eggs in HOME.",
                ],
                // Gemma changes the first task for "that", the one added last.
                [
                    "draftChanges": [["task": "1", "due": "Friday", "priority": "none", "remove": false]],
                    "message": "I moved buy milk to Friday.",
                ],
                [
                    "draftChanges": [["task": "2", "remove": true, "priority": "none"]],
                    "message": "I took buy eggs out of the plan.",
                ],
            ]))
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        await flow.step("new-chat", image: false)
        send("Add a task to buy milk, in HOME", in: id, of: store)
        await flow.step("first-draft", image: false)

        send("Also add buy eggs", in: id, of: store)
        await flow.step("also-added")
        XCTAssertEqual(draftTitles(id, in: store), ["buy milk", "buy eggs"])

        send("Change that to Friday", in: id, of: store)
        await flow.step("changed")
        let draft = store.conversations.chats[id: id]?.conversation.draft
        XCTAssertEqual(draft?.tasks.map(\.due), [nil, "Friday"])
        XCTAssertEqual(draft?.tasks.map(\.priority), [Priority.none, .none])

        send("Remove the second one", in: id, of: store)
        await flow.step("removed")
        XCTAssertEqual(draftTitles(id, in: store), ["buy milk"])
        flow.tap("Add")
        await flow.step("added", image: false)
        XCTAssertEqual(store.workspace?.tasks.values.filter { $0.title == "buy milk" }.count, 1)
        flow.finish()
    }

    /// Drafts the model gets a little wrong, which Lifr puts right before
    /// they show: a new Project that's there already with a task restating
    /// its goal and blank dates, then a new Avenue whose task's steps have
    /// one twice and a due date of none, then a task with steps that the
    /// model writes again as a Project nobody asked for.
    func testAssistantRepairsDrafts() async throws {
        let (store, flow) = app(
            "assistant-repairs",
            assistant: gemma([
                [
                    "message": "I added Stretch after every run to your Half marathon project.",
                    "plan": [
                        "avenues": [],
                        "projects": [
                            [
                                "name": "Half marathon", "avenue": "HEALTH", "due": "",
                                "details": "Goal is to complete a half marathon.",
                            ]
                        ],
                        "tasks": [
                            [
                                "title": "Stretch after every run", "priority": "none", "due": "",
                                "place": "HEALTH/Half marathon",
                            ],
                            [
                                "title": "Run half marathon", "priority": "none", "due": "",
                                "place": "HEALTH/Half marathon",
                            ],
                        ],
                    ],
                ],
                [
                    "plan": [
                        "avenues": [["key": "GARDEN", "name": "Garden"]],
                        "tasks": [
                            [
                                "title": "Plant tomatoes", "place": "Garden/plant_tomatoes",
                                "subtasks": [
                                    ["title": "Buy seedlings", "due": "none"], ["title": "Dig the bed"],
                                    ["title": "buy seedlings"],
                                ],
                            ]
                        ],
                    ],
                    "message": "Here's a Garden Avenue with a task to plant tomatoes.",
                ],
                [
                    "message": "I added a task to plan the team offsite.",
                    "plan": [
                        "avenues": [["key": "WORK", "name": "Work"]],
                        "projects": [
                            [
                                "name": "Team Offsite Planning", "avenue": "WORK",
                                "tasks": [["title": "Book a venue"], ["title": "Send the invites"]],
                            ]
                        ],
                        "tasks": [
                            [
                                "title": "Plan the team offsite", "place": "WORK/Team Offsite",
                                "subtasks": [["title": "Book a venue"], ["title": "Send the invites"]],
                            ]
                        ],
                    ],
                ],
            ]))
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        await flow.step("new-chat", image: false)
        send("Add a task to stretch after every run to my Half marathon project", in: id, of: store)
        await flow.step("project-draft")
        let chat = try XCTUnwrap(store.conversations.chats[id: id])
        XCTAssertEqual(chat.conversation.draft?.tasks.map(\.title), ["Stretch after every run"])
        XCTAssertEqual(chat.conversation.draft?.projects, [])
        XCTAssertNil(chat.problem)
        flow.tap("Add")
        await flow.step("project-added", image: false)

        send("Make a Garden Avenue with a task to plant tomatoes: buy seedlings, dig the bed", in: id, of: store)
        await flow.step("garden-draft")
        let garden = try XCTUnwrap(store.conversations.chats[id: id])
        XCTAssertEqual(garden.conversation.draft?.tasks.map(\.place), ["GARDEN"])
        XCTAssertEqual(
            garden.conversation.draft?.tasks.first?.subtasks.map(\.title), ["Buy seedlings", "Dig the bed"])
        XCTAssertNil(garden.problem)
        flow.tap("Add")
        await flow.step("garden-added", image: false)
        XCTAssertNotNil(store.workspace?.avenues.values.first { $0.key.rawValue == "GARDEN" }, "Garden wasn't added")

        send("Add a task to plan the team offsite in WORK: book a venue, send the invites", in: id, of: store)
        await flow.step("offsite-draft")
        XCTAssertEqual(draftTitles(id, in: store), ["Plan the team offsite"])
        XCTAssertEqual(store.conversations.chats[id: id]?.conversation.draft?.projects, [])
        flow.finish()
    }

    /// Adding an urgent task, where the model first writes back only the
    /// WORK Avenue that's there already and says it added the task: Lifr
    /// asks again, and the draft has the task.
    func testAssistantAsksAgainForForgottenTasks() async throws {
        let (store, flow) = app(
            "assistant-forgotten-tasks",
            assistant: gemma([
                [
                    "plan": ["avenues": [["name": "Work", "key": "WORK"]]],
                    "message": "I have added an urgent task to your WORK avenue titled 'file my taxes'.",
                ],
                [
                    "plan": [
                        "tasks": [["title": "file my taxes", "place": "WORK", "priority": "urgent"]],
                        "avenues": [["key": "WORK", "name": "Work"]],
                    ],
                    "message": "I have added an urgent task to file my taxes under the WORK avenue for you.",
                ],
            ]))
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        // Once the chat has the records, so it knows WORK is there.
        await flow.step("new-chat", image: false)
        send("Add an urgent task to file my taxes, in WORK", in: id, of: store)
        await flow.step("draft")
        let chat = try XCTUnwrap(store.conversations.chats[id: id])
        XCTAssertEqual(chat.conversation.draft?.tasks.map(\.title), ["file my taxes"])
        XCTAssertEqual(chat.conversation.draft?.tasks.first?.priority, .urgent)
        XCTAssertEqual(chat.conversation.draft?.avenues, [])
        flow.tap("Add")
        await flow.step("added", image: false)
        let added = store.workspace?.tasks.values.first { $0.title == "file my taxes" }
        XCTAssertEqual(added?.priority, .urgent)
        flow.finish()
    }

    /// A brain dump sent without a skill: Lifr picks Break down, the
    /// assistant asks what it needs to, and once answered plans a Project
    /// for the move, its tasks in order with steps, and a task on its own.
    func testAssistantBreaksDownABrainDump() async throws {
        let (store, flow) = app(
            "assistant-breakdown",
            assistant: gemma(
                [
                    ["message": "Is the move by a date, and is the deposit for the new flat or the old one?"],
                    [
                        "plan": [
                            "projects": [["name": "Move flat", "avenue": "HOME", "due": "2026-12-05"]],
                            "tasks": [
                                [
                                    "title": "Get the old deposit back", "place": "HOME/Move flat",
                                    "due": "2026-11-20",
                                    "subtasks": [["title": "Photograph each room"], ["title": "Email the landlord"]],
                                ],
                                ["title": "Book a van", "place": "HOME/Move flat", "due": "2026-11-27"],
                                [
                                    "title": "Pack", "place": "HOME/Move flat", "due": "2026-12-04",
                                    "subtasks": [["title": "Buy boxes"], ["title": "Pack the kitchen last"]],
                                ],
                                ["title": "Call mum", "place": "HOME"],
                            ],
                        ],
                        "message": "Here's the move as a Project, in order, and calling your mum on its own.",
                    ],
                ], picking: Skill.breakdown.id))
        store.tab = .conversations
        store.send(.conversations(.newTapped))
        let id = try XCTUnwrap(store.conversations.openID)
        await flow.step("new-chat", image: false)
        send("ugh I have to move flat, get the deposit sorted, pack, book a van, and call mum", in: id, of: store)
        await flow.step("asks")
        let asked = try XCTUnwrap(store.conversations.chats[id: id])
        XCTAssertEqual(asked.conversation.messages.first?.skill, Skill.breakdown.id)
        XCTAssertNil(asked.conversation.draft, "it asks before it plans")

        send("By 5 December, and it's the old flat's deposit", in: id, of: store)
        await flow.step("planned")
        let planned = try XCTUnwrap(store.conversations.chats[id: id])
        XCTAssertEqual(planned.conversation.draft?.projects.map(\.name), ["Move flat"])
        XCTAssertEqual(
            planned.conversation.draft?.tasks.map(\.title),
            ["Get the old deposit back", "Book a van", "Pack", "Call mum"])
        XCTAssertNil(planned.problem)
        flow.tap("Add")
        await flow.step("added")
        XCTAssertNotNil(store.workspace?.projects.values.first { $0.name == "Move flat" }, "the Project wasn't added")
        flow.finish()
    }
}
