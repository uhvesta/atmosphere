import CaptureKit
import ChatFeatures
import ChatKit
import ComposableArchitecture
import CoreGraphics
import Foundation
import ImageIO
import InferenceKit
import JSONKit
import LocalLogChatrChats
import LocalLogChatrModels

/// A model that answers the flows' questions, and the chats it runs in.
enum Sample {
    /// In Markdown, as a model writes it.
    static let pasta = """
        Here's a quick one for two:

        1. Boil **200 g of spaghetti** in salted water.
        2. Fry two cloves of garlic in olive oil until golden.
        3. Toss the pasta in the pan with chili flakes and parsley.

        Ready in about *15 minutes*.
        """

    static let monad = """
        A **monad** wraps a value with a way to chain steps that each return a wrapped value, \
        like `Optional`'s `flatMap`.
        """

    static let sunset = """
        A sunset over rolling **green hills**: the sun sits low on the horizon, and the sky fades from orange \
        to pink above it.
        """

    /// Answers by what the last message asks, or by the photo sent with it,
    /// thinking first when asked to.
    private static let answer: ScriptedRuntime.Step = ScriptedRuntime.think(
        "They want something short and practical.", for: .seconds(2)
    ) { turn, partial throws(ModelRuntimeError) in
        let value = JSONValue.string(
            !turn.attachments.isEmpty ? sunset : turn.prompt.contains("monad") ? monad : pasta)
        partial(value)
        return value
    }

    static var runtime: ScriptedRuntime {
        ScriptedRuntime(inputs: [.image], steps: Array(repeating: answer, count: 100))
    }

    /// The model that reads photos.
    static let seeing = ModelCatalog.gemma4E4B

    /// The person's newest photos: a sunset, then skies of other colors.
    static let recents = [0.08, 0.6, 0.3, 0.8, 0.45, 0.15].enumerated().compactMap { index, hue in
        photo(hue: hue).map { RecentPhoto(id: "photo-\(index)", thumbnail: $0) }
    }

    /// No chats yet, with a model that thinks and one that reads photos
    /// downloaded beside Apple's.
    @MainActor
    static func chats(speech: SpeechClient = .inMemory) -> StoreOf<ChatsFeature> {
        Store(initialState: ChatsFeature.State()) {
            ChatsFeature()
        } withDependencies: {
            $0.chatShelf = .inMemory()
            $0.chatRuntime = .always(runtime)
            $0.speech = speech
            $0.models = InMemoryModelLibrary(installed: [ModelCatalog.qwen35Small, seeing]).client
            $0.photoLibrary = .scripted(recents, camera: true)
            $0.chatPhotos = .inMemory()
            $0.date.now = Date(timeIntervalSince1970: 1_793_700_000)
            $0.uuid = .incrementing
            $0.dictation = .scripted()
            $0.fileReader = .scripted([:])
        }
    }

    /// Opens a new chat in `chats` on the model that reads photos.
    @MainActor
    static func seeingChat(in chats: StoreOf<ChatsFeature>) async throws -> UUID {
        chats.send(.newTapped)
        guard let id = chats.openID else { throw Unopened() }
        chats.send(.chats(.element(id: id, action: .modelTapped)))
        try await waitFor { chats.chats[id: id]?.modelSheet?.models.isEmpty == false }
        chats.send(.chats(.element(id: id, action: .modelSheet(.presented(.binding(.set(\.model, seeing.id)))))))
        chats.send(.chats(.element(id: id, action: .modelSheet(.dismiss))))
        try await waitFor { chats.chats[id: id]?.readsPhotos == true }
        return id
    }

    /// A sunset over hills, `hue` the sky's color, as a camera gives it.
    static func photo(hue: Double, side: Int = 480) -> Data? {
        guard
            let context = CGContext(
                data: nil, width: side, height: side, bitsPerComponent: 8, bytesPerRow: 0,
                space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
        else { return nil }
        let size = CGFloat(side)
        let sky = color(hue: hue, brightness: 0.95)
        let low = color(hue: hue + 0.9, brightness: 0.85)
        if let gradient = CGGradient(
            colorsSpace: CGColorSpaceCreateDeviceRGB(), colors: [low, sky] as CFArray, locations: [0, 1])
        {
            context.drawLinearGradient(gradient, start: .zero, end: CGPoint(x: 0, y: size), options: [])
        }
        context.setFillColor(CGColor(red: 1, green: 0.85, blue: 0.4, alpha: 1))
        context.fillEllipse(in: CGRect(x: size * 0.55, y: size * 0.3, width: size * 0.22, height: size * 0.22))
        for (index, shade) in [0.45, 0.32].enumerated() {
            context.setFillColor(CGColor(red: 0.15, green: shade, blue: 0.2, alpha: 1))
            let top = size * (0.38 - 0.1 * CGFloat(index))
            context.move(to: .zero)
            context.addCurve(
                to: CGPoint(x: size, y: top * 0.7), control1: CGPoint(x: size * 0.3, y: top * 1.4),
                control2: CGPoint(x: size * 0.6, y: top * 0.2))
            context.addLine(to: CGPoint(x: size, y: 0))
            context.fillPath()
        }
        guard let image = context.makeImage() else { return nil }
        let data = NSMutableData()
        guard
            let destination = CGImageDestinationCreateWithData(data as CFMutableData, "public.jpeg" as CFString, 1, nil)
        else { return nil }
        CGImageDestinationAddImage(destination, image, nil)
        return CGImageDestinationFinalize(destination) ? data as Data : nil
    }

    private static func color(hue: Double, brightness: Double) -> CGColor {
        // From hue to RGB, fully saturated toward white.
        let hue = hue.truncatingRemainder(dividingBy: 1) * 6
        let part = hue - hue.rounded(.down)
        let (red, green, blue): (Double, Double, Double) =
            switch Int(hue) {
            case 0: (1, part, 0)
            case 1: (1 - part, 1, 0)
            case 2: (0, 1, part)
            case 3: (0, 1 - part, 1)
            case 4: (part, 0, 1)
            default: (1, 0, 1 - part)
            }
        let soft = { (value: Double) in (0.35 + 0.65 * value) * brightness }
        return CGColor(red: soft(red), green: soft(green), blue: soft(blue), alpha: 1)
    }

    /// Opens a new chat in `chats`, sends `text` and waits for the reply.
    @MainActor
    static func ask(_ text: String, in chats: StoreOf<ChatsFeature>) async throws {
        if chats.openID == nil { chats.send(.newTapped) }
        guard let id = chats.openID else { throw Unopened() }
        chats.send(.chats(.element(id: id, action: .binding(.set(\.input, text)))))
        chats.send(.chats(.element(id: id, action: .sendTapped)))
        try await waitFor {
            chats.chats[id: id]?.reply == nil && chats.chats[id: id]?.thread.messages.last?.role == .assistant
        }
    }

    /// Waits for what the model is writing to land, failing after five
    /// seconds.
    @MainActor
    static func waitFor(_ done: @MainActor () -> Bool) async throws {
        for _ in 0..<500 {
            if done() { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        throw Unsettled()
    }

    struct Unopened: Error {}
    struct Unsettled: Error {}
}
