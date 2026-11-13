import ChatFeatures
import ChatKit
import ComposableArchitecture
import Foundation
import InferenceKit
import JSONKit
import LangReadrCore
import LangReadrModels
import LangReadrServices
import LangReadrServicesFake
import XCTest

@testable import LangReadrReading

@MainActor
final class ReaderFeatureTests: XCTestCase {
    private func reader(
        _ book: Book, steps: [ScriptedRuntime.Step] = [], spoken: LockIsolated<[String]> = LockIsolated([]),
        fails: SpeechError? = nil
    ) async -> TestStoreOf<ReaderFeature> {
        let store = TestStore(initialState: ReaderFeature.State(book: book)) {
            ReaderFeature()
        } withDependencies: {
            $0.tutor = tutor(steps)
            $0.chatRuntime = .always(ScriptedRuntime(steps: steps))
            $0.readerServices = .fake()
            $0.speech.speak = { text, _ throws(SpeechError) in
                if let fails { throw fails }
                spoken.withValue { $0.append(text) }
            }
            $0.date.now = epoch
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off
        await store.send(.task)
        return store
    }

    private func word(_ text: String, in store: TestStoreOf<ReaderFeature>) throws -> (UUID, WordSpan) {
        let paragraph = try store.state.book.paragraphs.first.unwrap()
        return (paragraph.id, try (store.state.words[paragraph.id]?.first { $0.text == text }).unwrap())
    }

    func testSwitchingTranslationOnTranslatesEachParagraphInView() async throws {
        let book = try spanish("Hola amigo.\n\nBuenos días.")
        let store = await reader(
            book, steps: [ScriptedRuntime.answer("Hello friend."), ScriptedRuntime.answer("Good morning.")])
        await store.send(.binding(.set(\.showsTranslation, true)))
        for paragraph in book.paragraphs { await store.send(.paragraphAppeared(paragraph.id)) }
        await store.settle()
        XCTAssertEqual(
            book.paragraphs.map { store.state.book.annotations[$0.id]?.translation },
            ["Hello friend.", "Good morning."])
        XCTAssertEqual(store.state.book.translatedInto, Language("en"))
        XCTAssertTrue(store.state.writing.isEmpty)
    }

    func testTheSwitchesAreKeptWithTheBookForItsNextOpening() async throws {
        let store = await reader(try spanish())
        await store.send(.binding(.set(\.showsTranslation, true)))
        await store.send(.binding(.set(\.showsTransliteration, true)))

        let reopened = ReaderFeature.State(book: store.state.book)
        XCTAssertTrue(reopened.showsTranslation)
        XCTAssertTrue(reopened.showsTransliteration)
        XCTAssertFalse(ReaderFeature.State(book: try spanish()).showsTranslation, "a new book opens with them off")
    }

    func testTheTopParagraphOnScreenIsWhereTheBookOpensNext() async throws {
        let book = try spanish("Hola amigo.\n\nBuenos días.\n\nAdiós.")
        let ids = book.paragraphs.map(\.id)
        let store = await reader(book)
        await store.send(.paragraphsShown(Array(ids[1...])))
        XCTAssertEqual(store.state.book.lastRead, ids[1])
        await store.send(.paragraphsShown([UUID(), ids[2]]))
        XCTAssertEqual(store.state.book.lastRead, ids[2], "what isn't a paragraph, such as a chapter title, is skipped")
    }

    func testLookedUpWordsAreListedInReadingOrderAndOpenTheirCard() async throws {
        var book = try spanish()
        let paragraph = try book.paragraphs.first.unwrap().id
        for text in ["amigo", "Hola"] {
            book.words[WordKey(paragraph: paragraph, word: text)] = WordNote(
                word: text, transliteration: nil, meaning: "-", inSentence: "")
        }
        let store = await reader(book)
        XCTAssertEqual(store.state.lookedUp.map(\.word.text), ["Hola", "amigo"])

        await store.send(.lookedUpWordTapped(WordKey(paragraph: paragraph, word: "amigo")))
        await store.receive(\.wordTapped)
        XCTAssertEqual(store.state.destination?.word?.word.text, "amigo")
    }

    func testAFailedParagraphSaysSoAndTryingAgainTranslatesIt() async throws {
        let book = try spanish("Hola.")
        let paragraph = try book.paragraphs.first.unwrap()
        let store = await reader(book, steps: [ScriptedRuntime.fail(.failed("busy")), ScriptedRuntime.answer("Hi.")])
        await store.send(.binding(.set(\.showsTranslation, true)))
        await store.send(.paragraphAppeared(paragraph.id))
        await store.receive(\.annotated)
        XCTAssertNotNil(store.state.problems[paragraph.id])

        await store.send(.paragraphAppeared(paragraph.id))
        XCTAssertTrue(store.state.writing.isEmpty, "a failed paragraph waits for the person to try again")

        await store.send(.retryTapped(paragraph.id))
        await store.receive(\.annotated)
        XCTAssertEqual(store.state.book.annotations[paragraph.id]?.translation, "Hi.")
        XCTAssertNil(store.state.problems[paragraph.id])
    }

    func testATappedWordIsSaidAndItsCardFillsWithWhatItMeans() async throws {
        let spoken = LockIsolated<[String]>([])
        let note: JSONValue = .object(["reading": "amigo", "meaning": "friend", "inSentence": "Who's greeted."])
        let store = await reader(try spanish(), steps: [ScriptedRuntime.answer(note)], spoken: spoken)
        let (paragraph, amigo) = try word("amigo", in: store)
        await store.send(.wordTapped(paragraph: paragraph, amigo))
        await store.finish()
        XCTAssertEqual(spoken.value, ["amigo"])
        XCTAssertEqual(store.state.destination?.word?.sentence, "Hola amigo.")

        await store.send(.destination(.presented(.word(.task))))
        await store.receive(\.destination.presented.word.delegate.lookedUp)
        let card = try (store.state.destination?.word).unwrap()
        XCTAssertEqual(card.note?.meaning, "friend")
        XCTAssertNil(card.note?.transliteration, "a reading that's the word itself isn't shown")
        XCTAssertEqual(store.state.book.words[WordKey(paragraph: paragraph, word: "amigo")]?.meaning, "friend")
    }

    func testATappedWordThatCantBeSaidSaysWhyOnItsCard() async throws {
        let store = await reader(try spanish(), fails: .audioUnavailable(reason: "busy"))
        let (paragraph, amigo) = try word("amigo", in: store)

        await store.send(.wordTapped(paragraph: paragraph, amigo))
        await store.receive(\.destination.presented.word.spoke)

        XCTAssertNotNil(store.state.destination?.word?.speechProblem)
    }

    func testTwoTapsWhileHighlightingAskAboutThePassageBetweenThem() async throws {
        let store = await reader(try spanish())
        let (paragraph, amigo) = try word("amigo", in: store)
        let (_, dias) = try word("días", in: store)
        await store.send(.binding(.set(\.isHighlighting, true)))
        await store.send(.wordTapped(paragraph: paragraph, dias))
        await store.send(.wordTapped(paragraph: paragraph, amigo))
        XCTAssertEqual(store.state.picked?.text, "amigo. Buenos días")

        await store.send(.askTapped)
        XCTAssertFalse(store.state.isHighlighting)
        let chat = try store.state.book.chats.first.unwrap()
        XCTAssertEqual(chat.passage.text, "amigo. Buenos días")
        XCTAssertEqual(store.state.destination?.chat?.id, chat.id)
        XCTAssertEqual(store.state.destination?.chat?.compact, true)
    }

    func testCancellingAHighlightPicksNothingAndAsksNothing() async throws {
        let store = await reader(try spanish())
        let (paragraph, amigo) = try word("amigo", in: store)
        await store.send(.binding(.set(\.isHighlighting, true)))
        await store.send(.wordTapped(paragraph: paragraph, amigo))
        await store.send(.cancelHighlightTapped)
        XCTAssertFalse(store.state.isHighlighting)
        XCTAssertNil(store.state.picked)
        await store.send(.askTapped)
        XCTAssertTrue(store.state.book.chats.isEmpty)
        XCTAssertNil(store.state.destination)
    }

    func testAPassagesChatOpensFullFromItsSheet() async throws {
        let store = await reader(try spanish("Hola amigo."))
        let (paragraph, hola) = try word("Hola", in: store)
        await store.send(.binding(.set(\.isHighlighting, true)))
        await store.send(.wordTapped(paragraph: paragraph, hola))
        await store.send(.askTapped)
        XCTAssertEqual(store.state.destination?.chat?.compact, true)
        await store.send(.destination(.presented(.chat(.openFullTapped))))
        XCTAssertEqual(store.state.destination?.chat?.compact, false)
    }

    func testAParagraphsBubbleReopensItsLatestChat() async throws {
        let store = await reader(try spanish("Hola amigo."))
        let (paragraph, hola) = try word("Hola", in: store)
        let (_, amigo) = try word("amigo", in: store)
        await store.send(.chatBubbleTapped(paragraph: paragraph))
        XCTAssertNil(store.state.destination)

        for span in [hola, amigo] {
            await store.send(.binding(.set(\.isHighlighting, true)))
            await store.send(.wordTapped(paragraph: paragraph, span))
            await store.send(.askTapped)
            await store.send(.destination(.dismiss))
        }
        let latest = try store.state.book.chats(in: paragraph).last.unwrap()
        XCTAssertEqual(latest.passage.text, "amigo")
        await store.send(.chatBubbleTapped(paragraph: paragraph))
        XCTAssertEqual(store.state.destination?.chat?.id, latest.id)
        XCTAssertEqual(store.state.destination?.chat?.compact, true)
    }

    func testTheChatsListOpensTheOnePickedFull() async throws {
        let store = await reader(try spanish("Hola amigo."))
        let (paragraph, hola) = try word("Hola", in: store)
        await store.send(.binding(.set(\.isHighlighting, true)))
        await store.send(.wordTapped(paragraph: paragraph, hola))
        await store.send(.askTapped)
        await store.send(.destination(.dismiss))
        let chat = try store.state.book.chats.first.unwrap()

        await store.send(.chatsTapped)
        XCTAssertEqual(store.state.destination?.chats?.chats.map(\.id), [chat.id])
        await store.send(.destination(.presented(.chats(.chatTapped(chat.id)))))
        XCTAssertEqual(store.state.destination?.chat?.id, chat.id)
        XCTAssertEqual(store.state.destination?.chat?.compact, false)
    }

    func testAReplyToAQuestionIsKeptInThePassagesChat() async throws {
        let store = await reader(
            try spanish("Hola amigo."), steps: [ScriptedRuntime.answer("It's a greeting.", partials: ["It's"])])
        let (paragraph, hola) = try word("Hola", in: store)
        await store.send(.binding(.set(\.isHighlighting, true)))
        await store.send(.wordTapped(paragraph: paragraph, hola))
        await store.send(.askTapped)
        XCTAssertEqual(store.state.destination?.chat?.thread.canSend, false)

        await store.send(.destination(.presented(.chat(.thread(.binding(.set(\.input, "What does it mean?")))))))
        await store.send(.destination(.presented(.chat(.thread(.sendTapped)))))
        await store.receive(\.destination.presented.chat.thread.replied.success)
        await store.settle()
        let messages = try store.state.book.chats.first.unwrap().conversation.messages
        XCTAssertEqual(messages.map(\.text), ["What does it mean?", "It's a greeting."])
        XCTAssertEqual(messages.map(\.role), [.person, .assistant])
        XCTAssertNil(store.state.destination?.chat?.thread.reply)
    }

    func testAnotherLanguageDropsWhatWasWrittenForTheOldOne() async throws {
        var book = try spanish("Ciao.")
        let paragraph = try book.paragraphs.first.unwrap()
        book.annotate(paragraph.id, translation: "Hi.")
        let store = await reader(book)
        await store.send(.languagePicked(Language("it")))
        XCTAssertEqual(store.state.book.language, Language("es"), "it asks first, as what was written is lost")
        XCTAssertNotNil(store.state.alert)

        await store.send(.alert(.presented(.changeLanguage(Language("it")))))
        XCTAssertEqual(store.state.book.language, Language("it"))
        XCTAssertTrue(store.state.book.annotations.isEmpty)
    }

    func testAnotherLanguageForABookWithNothingWrittenNeedsNoAsking() async throws {
        let store = await reader(try spanish("Ciao."))
        await store.send(.languagePicked(Language("it")))
        XCTAssertEqual(store.state.book.language, Language("it"))
        XCTAssertNil(store.state.alert)
    }

    func testCancellingAnotherLanguageKeepsWhatWasWritten() async throws {
        var book = try spanish("Hola amigo.")
        let paragraph = try book.paragraphs.first.unwrap()
        book.words[WordKey(paragraph: paragraph.id, word: "amigo")] = WordNote(
            word: "amigo", transliteration: nil, meaning: "friend", inSentence: "")
        let store = await reader(book)
        await store.send(.languagePicked(Language("it")))
        await store.send(.alert(.dismiss))
        XCTAssertEqual(store.state.book.language, Language("es"))
        XCTAssertEqual(store.state.book.words.count, 1)
    }

    func testAReadingIsKeptWhenTheTranslationFailsAndTryingAgainOnlyTranslates() async throws {
        let book = try Importer.text("こんにちは。", title: "話").book(in: Language("ja"), at: epoch)
        let paragraph = try book.paragraphs.first.unwrap()
        let store = await reader(
            book,
            steps: [
                ScriptedRuntime.fail(.busy), ScriptedRuntime.answer("Konnichiwa."), ScriptedRuntime.answer("Hello."),
            ])
        await store.send(.binding(.set(\.showsTranslation, true)))
        await store.send(.binding(.set(\.showsTransliteration, true)))
        await store.send(.paragraphAppeared(paragraph.id))
        await store.receive(\.annotated)
        XCTAssertEqual(store.state.book.annotations[paragraph.id]?.transliteration, "Konnichiwa.")
        XCTAssertNil(store.state.book.annotations[paragraph.id]?.translation)
        XCTAssertNotNil(store.state.problems[paragraph.id])

        await store.send(.retryTapped(paragraph.id))
        await store.receive(\.annotated)
        XCTAssertEqual(store.state.book.annotations[paragraph.id]?.translation, "Hello.")
        XCTAssertEqual(store.state.book.annotations[paragraph.id]?.transliteration, "Konnichiwa.")
        XCTAssertNil(store.state.problems[paragraph.id])
    }

    func testTransliterateIsOfferedWhenAnyParagraphIsInAnotherScript() async throws {
        let mixed = try Importer.text("Chapter One\n\nこんにちは。", title: "話").book(in: Language("ja"), at: epoch)
        XCTAssertTrue(ReaderFeature.State(book: mixed).canTransliterate)
        XCTAssertFalse(ReaderFeature.State(book: try spanish()).canTransliterate)
    }

    func testAWordLookedUpElsewhereShowsItsMeaningWithoutAskingTheModel() async throws {
        var book = try spanish("Hola amigo.\n\nAdiós amigo.")
        let first = try book.paragraphs.first.unwrap().id
        book.words[WordKey(paragraph: first, word: "amigo")] = WordNote(
            word: "amigo", transliteration: nil, meaning: "friend", inSentence: "Who's greeted.")
        let store = await reader(book)
        let second = try book.paragraphs.last.unwrap()
        let amigo = try store.state.words[second.id].unwrap().first { $0.text == "amigo" }.unwrap()
        await store.send(.wordTapped(paragraph: second.id, amigo))
        let card = try (store.state.destination?.word).unwrap()
        XCTAssertEqual(card.known?.meaning, "friend")
        XCTAssertNil(card.note, "what it means in this sentence is looked up only when asked")
    }
}
