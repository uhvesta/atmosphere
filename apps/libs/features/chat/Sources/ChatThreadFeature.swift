import CaptureKit
import ChatKit
import ComposableArchitecture
import Foundation
import ModelsClient
import NaturalLanguage

/// A chat with a model, for an app to put anywhere: the person writes, the
/// model replies as it writes and can think first, and each reply can be
/// stopped, written again or read aloud. Editing a message or writing a
/// reply again keeps the old one on its own branch. Photos, files and
/// dictation go with the next message, and each chat picks its own model.
///
/// The app says what the chat knows in `instructions`, which use's model
/// it runs on by default, and keeps the thread when it's `changed`.
@Reducer
public struct ChatThreadFeature {
    @ObservableState
    public struct State: Equatable, Identifiable {
        public var thread: ChatThread
        /// The thread's, so an app can keep several chats side by side.
        public var id: UUID { thread.id }
        /// What the chat runs on in place of its use's default.
        public var model: ModelChoice
        /// Which of the app's uses gives the default model.
        public var use: ModelUseKey
        /// What the model is told before the messages: who it is and what
        /// the chat is about.
        public var instructions: String
        public var input = ""
        /// The reply as it's written; nil when none is.
        public var reply: String?
        /// What the model thinks before the reply, as it's written.
        public var replyThinking: ThreadThinking?
        public var showsReplyThinking = false
        /// The replies whose thinking is open.
        public var shownThinking: Set<UUID> = []
        public var problem: String?
        /// The sheet picking what the chat runs on.
        @Presents public var modelSheet: ChatModelFeature.State?
        /// Apple's model and the downloaded ones, for the composer's pill.
        public var models: [RunnableModel] = []
        /// The use's model in the Models screen.
        public var standard: ModelID?
        /// How much of the model's window the last reply took; nil before
        /// one, and once the model or its window changes.
        public var context: ContextUse?
        /// The files that go with the next message.
        public var pending: [ThreadFile] = []
        /// The photos that go with the next message.
        public var pendingPhotos: [ThreadPhoto] = []
        /// Whether the file picker is open. + opens it straight away for a
        /// model that reads only text.
        public var picksFiles = false
        /// Whether + shows the camera, recent photos and files, on the
        /// iPhone, for a model that reads photos.
        public var addsContext = false
        /// Whether the app may show the person's recent photos in +.
        public var photosAccess: PhotosAccess = .notAsked
        /// The person's newest photos, for + to attach with a tap.
        public var recentPhotos: [RecentPhoto] = []
        /// The recent photos attached to the next message, with the photo
        /// each became.
        public var attachedRecents: [RecentPhoto.ID: UUID] = [:]
        /// Whether there's a camera to take a photo with.
        public var hasCamera = false
        /// Whether a photo is being kept for the next message.
        public var attaching = false
        /// The file of each photo in the chat that's on this device, to
        /// show it.
        public var photoFiles: [UUID: URL] = [:]
        /// Whether the microphone is writing what's said into the field.
        public var dictating = false
        /// What the field held when dictation started, which what's said
        /// follows; nil once the dictation has ended.
        public var dictatedAfter: String?
        /// The message being edited, whose new text is in the field.
        public var editing: UUID?
        /// The reply being read aloud.
        public var speaking: UUID?
        /// Whether the bar to find words in the messages is open.
        public var finds = false
        public var findQuery = ""
        /// Which of the matches is shown, the oldest first.
        public var findPosition = 0

        public init(thread: ChatThread, model: ModelChoice = .defaults, use: ModelUseKey, instructions: String) {
            self.thread = thread
            self.model = model
            self.use = use
            self.instructions = instructions
        }

        public var canSend: Bool {
            reply == nil && dictatedAfter == nil && !attaching
                && (!input.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !pending.isEmpty
                    || !pendingPhotos.isEmpty)
        }

        /// Whether the chat's model reads photos, so + offers them.
        public var readsPhotos: Bool { current?.inputs.contains(.image) == true }

        /// Whether the chat can go to another branch or write a reply
        /// again: not while one is written.
        public var canGo: Bool { reply == nil && !thread.messages.isEmpty }

        /// Whether the reply to `message`, or `message` itself for a reply,
        /// can be written again: any reply, or the last message when its
        /// reply didn't come.
        public func canRetry(_ message: ThreadMessage) -> Bool {
            guard canGo else { return false }
            return message.role == .assistant || message.id == thread.messages.last?.id
        }

        /// The model the chat runs on, once it's looked up.
        public var current: RunnableModel? {
            let id = model.model ?? standard
            return models.first { $0.id == id }
        }

        /// The messages shown that have every word being found, oldest
        /// first.
        public var findMatches: [UUID] {
            let query = ConversationQuery(words: findQuery.split(whereSeparator: \.isWhitespace).map(String.init))
            guard !query.isEmpty else { return [] }
            return thread.messages.filter { query.matches($0.text) }.map(\.id)
        }

        /// The match shown now, when there's one.
        public var currentMatch: UUID? {
            let matches = findMatches
            return matches.indices.contains(findPosition) ? matches[findPosition] : nil
        }
    }

    public enum Action: BindableAction {
        case binding(BindingAction<State>)
        case task
        case modelsLoaded([RunnableModel], standard: ModelID)
        case sendTapped
        case stopTapped
        case retryTapped(messageID: UUID)
        case editTapped(messageID: UUID)
        case cancelEditTapped
        case branchTapped(messageID: UUID, by: Int)
        case latestTapped
        case thinkingToggled(messageID: UUID?)
        case speakTapped(messageID: UUID)
        case spoke(messageID: UUID, problem: String?)
        case findTapped
        case findMoved(by: Int)
        case findClosed
        case modelTapped
        case modelSheet(PresentationAction<ChatModelFeature.Action>)
        case progressed(ReplyProgress)
        case replied(Result<ChatReply, ModelRuntimeError>)
        /// + on the iPhone, or for a model that reads only text: the
        /// camera, recent photos and files, or the file picker.
        case addContextTapped
        case photosLoaded(access: PhotosAccess, recents: [RecentPhoto], camera: Bool)
        /// Attaches a recent photo, or takes it off if it's attached.
        case recentPhotoTapped(RecentPhoto.ID)
        /// A photo the person picked, in any format the system reads; nil
        /// when it couldn't be read.
        case photoPicked(Data?)
        /// A photo the person took with the camera.
        case photoTaken(Data)
        case photoAttached(ThreadPhoto, recent: RecentPhoto.ID?)
        case photoFailed(String)
        case removePhotoTapped(UUID)
        case filesPicked(Result<[URL], any Error>)
        case filesRead([ThreadFile], photos: [ThreadPhoto], failures: [String])
        case removeFileTapped(Int)
        case micTapped
        case dictated(String)
        case dictationEnded
        case dictationFailed(String)
        case delegate(Delegate)

        @CasePathable
        public enum Delegate: Equatable {
            /// A message or the chat's model changed, for the app to keep.
            case changed(ChatThread, model: ModelChoice)
        }
    }

    private enum CancelID { case reply, dictation, speech }

    @Dependency(\.chatRuntime) private var chatRuntime
    @Dependency(\.models) private var library
    @Dependency(\.speech) private var speech
    @Dependency(\.date.now) private var now
    @Dependency(\.uuid) private var uuid
    @Dependency(\.dictation) private var dictation
    @Dependency(\.fileReader) private var fileReader
    @Dependency(\.photoLibrary) private var photoLibrary
    @Dependency(\.chatPhotos) private var chatPhotos

    public init() {}

    public var body: some ReducerOf<Self> {
        BindingReducer()
        Reduce { state, action in
            switch action {
            case .binding(\.findQuery):
                // The newest match first, nearest where the chat ends.
                state.findPosition = max(state.findMatches.count - 1, 0)
                return .none

            case .binding:
                return .none

            case .task:
                for photo in (state.thread.messages + state.thread.hidden).flatMap(\.photos) {
                    state.photoFiles[photo.id] = chatPhotos.file(photo, state.thread.id)
                }
                return .run { [library, use = state.use] send in
                    await send(.modelsLoaded(await library.runnable(), standard: library.selection(use)))
                }

            case .modelsLoaded(let models, let standard):
                state.models = models
                state.standard = standard
                return .none

            case .sendTapped:
                guard state.canSend else { return .none }
                return send(in: &state)

            case .stopTapped:
                guard let reply = state.reply else { return .none }
                let thinking = state.replyThinking
                return .merge(.cancel(id: CancelID.reply), keep(reply, thinking: thinking, in: &state))

            case .retryTapped(let id):
                let messages = state.thread.messages
                guard let index = messages.firstIndex(where: { $0.id == id }), state.canRetry(messages[index]),
                    let asked = messages[...index].last(where: { $0.role == .person })
                else { return .none }
                state.thread.go(to: asked.id)
                guard state.thread.messages.last?.id == asked.id else { return .none }
                state.problem = nil
                return .merge(changed(state), write(&state))

            case .editTapped(let id):
                guard state.canGo,
                    let message = state.thread.messages.first(where: { $0.id == id && $0.role == .person })
                else { return .none }
                state.editing = id
                state.input = message.text
                return .none

            case .cancelEditTapped:
                state.editing = nil
                state.input = ""
                return .none

            case .branchTapped(let id, let offset):
                guard state.canGo else { return .none }
                let branches = state.thread.branches(at: id)
                guard let index = branches.firstIndex(where: { $0.id == id }),
                    branches.indices.contains(index + offset)
                else { return .none }
                state.thread.go(to: state.thread.newest(through: branches[index + offset].id))
                return changed(state)

            case .latestTapped:
                guard state.canGo, let newest = state.thread.newest else { return .none }
                state.thread.go(to: newest)
                return changed(state)

            case .thinkingToggled(let id?):
                if state.shownThinking.remove(id) == nil { state.shownThinking.insert(id) }
                return .none

            case .thinkingToggled(nil):
                state.showsReplyThinking.toggle()
                return .none

            case .speakTapped(let id):
                if state.speaking == id {
                    state.speaking = nil
                    return .merge(.cancel(id: CancelID.speech), .run { [speech] _ in await speech.stop() })
                }
                guard let message = state.thread.messages.first(where: { $0.id == id && $0.role == .assistant })
                else { return .none }
                let text = Self.spoken(message.text)
                guard !text.isEmpty else { return .none }
                state.speaking = id
                return .run { [speech] send in
                    do throws(SpeechError) {
                        try await speech.speak(text, Self.language(of: text))
                        await send(.spoke(messageID: id, problem: nil))
                    } catch {
                        await send(.spoke(messageID: id, problem: error.description))
                    }
                }
                .cancellable(id: CancelID.speech, cancelInFlight: true)

            case .spoke(let id, let problem):
                if state.speaking == id { state.speaking = nil }
                if let problem { state.problem = problem }
                return .none

            case .findTapped:
                guard !state.thread.messages.isEmpty else { return .none }
                state.finds = true
                return .none

            case .findMoved(let step):
                let count = state.findMatches.count
                guard count > 0 else { return .none }
                state.findPosition = ((state.findPosition + step) % count + count) % count
                return .none

            case .findClosed:
                state.finds = false
                state.findQuery = ""
                state.findPosition = 0
                return .none

            case .modelTapped:
                state.modelSheet = ChatModelFeature.State(choice: state.model, use: state.use)
                return .none

            case .modelSheet:
                if let sheet = state.modelSheet, !sheet.models.isEmpty {
                    state.models = sheet.models
                    state.standard = sheet.standard
                }
                // Kept as it's picked, so the next message runs on it.
                guard let choice = state.modelSheet?.choice, choice != state.model else { return .none }
                if choice.model != state.model.model || choice.window != state.model.window {
                    state.context = nil
                }
                state.model = choice
                return changed(state)

            case .progressed(.text(let text)):
                if state.reply != nil { state.reply = text }
                return .none

            case .progressed(.thinking(let thinking)):
                if state.reply != nil { state.replyThinking = thinking }
                return .none

            case .replied(.success(let reply)):
                state.context = reply.context
                return keep(reply.text, thinking: reply.thinking, in: &state)

            case .replied(.failure(let error)):
                state.reply = nil
                state.replyThinking = nil
                state.showsReplyThinking = false
                state.problem = error.description
                return .none

            case .addContextTapped:
                guard state.readsPhotos else {
                    state.picksFiles = true
                    return .none
                }
                state.addsContext = true
                return .run { [photoLibrary] send in
                    var access = photoLibrary.access()
                    // Asked the first time + opens, not before.
                    if access == .notAsked { access = await photoLibrary.requestAccess() }
                    let recents = access == .allowed ? await photoLibrary.recents() : []
                    await send(.photosLoaded(access: access, recents: recents, camera: await photoLibrary.hasCamera()))
                }

            case .photosLoaded(let access, let recents, let camera):
                state.photosAccess = access
                state.recentPhotos = recents
                state.hasCamera = camera
                return .none

            case .recentPhotoTapped(let id):
                if let attached = state.attachedRecents[id] {
                    return remove(photo: attached, in: &state)
                }
                guard state.readsPhotos, !state.attaching else { return .none }
                state.attaching = true
                return .run { [photoLibrary, chatPhotos, chat = state.thread.id] send in
                    do throws(PhotoLibraryError) {
                        let data = try await photoLibrary.photo(id)
                        do throws(ChatPhotoError) {
                            await send(.photoAttached(try await chatPhotos.save(data, chat), recent: id))
                        } catch {
                            await send(.photoFailed(error.description))
                        }
                    } catch {
                        await send(.photoFailed(error.description))
                    }
                }

            case .photoPicked(let data?), .photoTaken(let data):
                // The library and the camera cover +, so it closes for them.
                state.addsContext = false
                guard state.readsPhotos, !state.attaching else { return .none }
                state.attaching = true
                return .run { [chatPhotos, chat = state.thread.id] send in
                    do throws(ChatPhotoError) {
                        await send(.photoAttached(try await chatPhotos.save(data, chat), recent: nil))
                    } catch {
                        await send(.photoFailed(error.description))
                    }
                }

            case .photoPicked(nil):
                state.addsContext = false
                state.problem = ChatPhotoError.unreadable.description
                return .none

            case .photoAttached(let photo, let recent):
                state.attaching = false
                state.pendingPhotos.append(photo)
                state.photoFiles[photo.id] = chatPhotos.file(photo, state.thread.id)
                if let recent { state.attachedRecents[recent] = photo.id }
                return .none

            case .photoFailed(let problem):
                state.attaching = false
                state.problem = problem
                return .none

            case .removePhotoTapped(let id):
                return remove(photo: id, in: &state)

            case .filesPicked(.failure(let error)):
                state.addsContext = false
                state.picksFiles = false
                state.problem = String(
                    localized: "Couldn't open that file: \(error.localizedDescription) Try picking it again.")
                return .none

            case .filesPicked(.success(let urls)):
                state.addsContext = false
                state.picksFiles = false
                guard !urls.isEmpty, !state.attaching else { return .none }
                state.attaching = true
                let photos = state.readsPhotos
                let model = state.current?.name ?? String(localized: "This chat's model")
                return .run { [fileReader, chatPhotos, chat = state.thread.id] send in
                    var read: [ThreadFile] = []
                    var kept: [ThreadPhoto] = []
                    var failures: [String] = []
                    for url in urls {
                        do {
                            switch try await fileReader.read(url) {
                            case .text(let file):
                                read.append(ThreadFile(name: file.name, text: file.text))
                            case .photo(let data) where photos:
                                do throws(ChatPhotoError) {
                                    kept.append(try await chatPhotos.save(data, chat))
                                } catch {
                                    failures.append(error.description)
                                }
                            case .photo:
                                failures.append(
                                    String(
                                        localized: """
                                            \(url.lastPathComponent) is a photo, and \(model) reads only text. Pick \
                                            a model that reads photos with the model button, or add a text, \
                                            Markdown or PDF file.
                                            """))
                            }
                        } catch {
                            failures.append(String(describing: error))
                        }
                    }
                    await send(.filesRead(read, photos: kept, failures: failures))
                }

            case .filesRead(let files, let photos, let failures):
                state.attaching = false
                state.pending += files
                state.pendingPhotos += photos
                for photo in photos { state.photoFiles[photo.id] = chatPhotos.file(photo, state.thread.id) }
                state.problem = failures.isEmpty ? nil : failures.joined(separator: "\n")
                return .none

            case .removeFileTapped(let index):
                guard state.pending.indices.contains(index) else { return .none }
                state.pending.remove(at: index)
                return .none

            case .micTapped:
                if state.dictating {
                    // The last words still come, until the dictation ends.
                    state.dictating = false
                    return .run { [dictation] _ in await dictation.stop() }
                }
                guard state.dictatedAfter == nil else { return .none }
                state.dictating = true
                state.problem = nil
                state.dictatedAfter = state.input.trimmingCharacters(in: .whitespacesAndNewlines)
                return .run { [dictation] send in
                    do {
                        for try await heard in try await dictation.start() {
                            await send(.dictated(heard))
                        }
                        await send(.dictationEnded)
                    } catch is CancellationError {
                        await send(.dictationEnded)
                    } catch {
                        await send(.dictationFailed(String(describing: error)))
                    }
                }
                .cancellable(id: CancelID.dictation, cancelInFlight: true)

            case .dictated(let heard):
                guard let before = state.dictatedAfter else { return .none }
                state.input = before.isEmpty || heard.isEmpty ? before + heard : before + " " + heard
                return .none

            case .dictationEnded:
                state.dictating = false
                state.dictatedAfter = nil
                return .none

            case .dictationFailed(let reason):
                state.dictating = false
                state.dictatedAfter = nil
                state.problem = reason
                return .none

            case .delegate:
                return .none
            }
        }
        .ifLet(\.$modelSheet, action: \.modelSheet) { ChatModelFeature() }
    }

    /// Sends what's in the field, with its files, in place of the message
    /// being edited if there's one, and asks for the reply.
    private func send(in state: inout State) -> Effect<Action> {
        let text = state.input.trimmingCharacters(in: .whitespacesAndNewlines)
        state.input = ""
        state.problem = nil
        var files = state.pending
        var photos = state.pendingPhotos
        state.pending = []
        state.pendingPhotos = []
        state.attachedRecents = [:]
        if let editing = state.editing {
            // The new message keeps the photos and files sent with the old
            // one.
            let old = (state.thread.messages + state.thread.hidden).first { $0.id == editing }
            files = (old?.files ?? []) + files
            photos = (old?.photos ?? []) + photos
            state.thread.go(before: editing)
            state.editing = nil
        }
        state.thread.append(
            ThreadMessage(id: uuid(), role: .person, text: text, at: now, files: files, photos: photos))
        return .merge(changed(state), write(&state))
    }

    /// Writes the reply to the last message, sending it as it's written.
    private func write(_ state: inout State) -> Effect<Action> {
        state.reply = ""
        state.replyThinking = nil
        state.showsReplyThinking = false
        let model = chatRuntime.runtime(state.use, state.model)
        let tokens = state.model.thinkingLevel.tokens
        // The newest photos sent, so the model can be asked about them
        // again, for a model that reads them.
        let sent = state.thread.messages.last { !$0.photos.isEmpty }
        let photos =
            model.inputs.contains(.image)
            ? (sent?.photos ?? []).compactMap { chatPhotos.file($0, state.thread.id) } : []
        let turn = ReplyTurn(
            thread: state.thread, instructions: state.instructions, budget: model.budget, thinkingTokens: tokens,
            photos: photos, photosFrom: sent?.id)
        return .run { send in
            let (updates, continuation) = AsyncStream.makeStream(
                of: ReplyProgress.self, bufferingPolicy: .bufferingNewest(8))
            async let written = outcome { () throws(ModelRuntimeError) in
                defer { continuation.finish() }
                return try await turn.ask(model, thinks: tokens != nil) { continuation.yield($0) }
            }
            for await update in updates { await send(.progressed(update)) }
            await send(.replied(await written))
        }
        .cancellable(id: CancelID.reply, cancelInFlight: true)
    }

    /// Ends the reply, keeping what's written of it.
    private func keep(_ text: String, thinking: ThreadThinking?, in state: inout State) -> Effect<Action> {
        state.reply = nil
        state.replyThinking = nil
        let shows = state.showsReplyThinking
        state.showsReplyThinking = false
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return .none }
        let id = uuid()
        state.thread.append(ThreadMessage(id: id, role: .assistant, text: text, at: now, thinking: thinking))
        // Thinking open while the reply was written stays open.
        if shows, thinking != nil { state.shownThinking.insert(id) }
        return changed(state)
    }

    /// Takes `photo` off the next message, and deletes it.
    private func remove(photo id: UUID, in state: inout State) -> Effect<Action> {
        guard let index = state.pendingPhotos.firstIndex(where: { $0.id == id }) else { return .none }
        let photo = state.pendingPhotos.remove(at: index)
        state.photoFiles[id] = nil
        state.attachedRecents = state.attachedRecents.filter { $0.value != id }
        return .run { [chatPhotos, chat = state.thread.id] _ in await chatPhotos.remove(photo, chat) }
    }

    private func changed(_ state: State) -> Effect<Action> {
        .send(.delegate(.changed(state.thread, model: state.model)))
    }

    /// `text` without its Markdown, as it's read aloud.
    static func spoken(_ text: String) -> String {
        let options = AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace)
        let plain = (try? AttributedString(markdown: text, options: options)).map { String($0.characters) } ?? text
        return plain.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// The language `text` is written in, as a BCP 47 tag: the device's
    /// when it can't be told.
    static func language(of text: String) -> String {
        if let language = NLLanguageRecognizer.dominantLanguage(for: text), language != .undetermined {
            return language.rawValue
        }
        return Locale.current.language.languageCode?.identifier ?? "en"
    }
}
