import ChatKit
import Foundation
import TestingKit

func threadTests() -> [TestCase] {
    [
        TestCase("given a message kept before files, photos and thinking, then it reads as having none") {
            let saved = Data(
                #"{"id":"6F9619FF-8B86-D011-B42D-00C04FC964FF","role":"person","text":"Hola","at":0}"#.utf8)
            let message = try JSONDecoder().decode(ThreadMessage.self, from: saved)
            try expectEqual(message.text, "Hola")
            try expectEqual(message.files, [])
            try expectEqual(message.photos, [])
            try expectEqual(message.thinking, nil)
        },
        TestCase("given a message with a file, a photo and thinking, then it reads back the same") {
            let message = ThreadMessage(
                role: .assistant, text: "Friend.", at: Date(timeIntervalSince1970: 0),
                files: [ThreadFile(name: "notes.md", text: "Amigo")], photos: [ThreadPhoto(id: UUID())],
                thinking: ThreadThinking(text: "It's Spanish.", duration: .seconds(2)))
            try expectEqual(
                try JSONDecoder().decode(ThreadMessage.self, from: JSONEncoder().encode(message)), message)
        },
    ]
}
