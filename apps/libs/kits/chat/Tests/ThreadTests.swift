import ChatKit
import Foundation
import TestingKit

func threadTests() -> [TestCase] {
    [
        TestCase("given a message kept before files, photos, thinking and speed, then it reads as having none") {
            let saved = Data(
                #"{"id":"6F9619FF-8B86-D011-B42D-00C04FC964FF","role":"person","text":"Hola","at":0}"#.utf8)
            let message = try JSONDecoder().decode(ThreadMessage.self, from: saved)
            try expectEqual(message.text, "Hola")
            try expectEqual(message.files, [])
            try expectEqual(message.photos, [])
            try expectEqual(message.thinking, nil)
            try expectEqual(message.model, nil)
            try expectEqual(message.speed, nil)
        },
        TestCase("given a message with a file, a photo, thinking and speed, then it reads back the same") {
            let message = ThreadMessage(
                role: .assistant, text: "Friend.", at: Date(timeIntervalSince1970: 0),
                files: [ThreadFile(name: "notes.md", text: "Amigo")], photos: [ThreadPhoto(id: UUID())],
                thinking: ThreadThinking(text: "It's Spanish.", duration: .seconds(2)), model: "Gemma 4 E4B",
                speed: ThreadSpeed(read: 120, written: 8, reading: .milliseconds(300), writing: .seconds(1)))
            try expectEqual(
                try JSONDecoder().decode(ThreadMessage.self, from: JSONEncoder().encode(message)), message)
        },
        TestCase("given a speed, then its rates are tokens a second, and none for no time") {
            let speed = ThreadSpeed(read: 120, written: 30, reading: .milliseconds(500), writing: .seconds(2))
            try expectEqual(speed.readPerSecond, 240)
            try expectEqual(speed.writtenPerSecond, 15)
            try expectEqual(ThreadSpeed(read: 1, written: 1, reading: .zero, writing: .zero).writtenPerSecond, nil)
        },
    ]
}
