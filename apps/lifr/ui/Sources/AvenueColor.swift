import SwiftUI

extension Color {
    /// An Avenue's color, stored by name.
    package init(avenue name: String) {
        self =
            switch name {
            case "red": .red
            case "orange": .orange
            case "yellow": .yellow
            case "green": .green
            case "mint": .mint
            case "teal": .teal
            case "cyan": .cyan
            case "indigo": .indigo
            case "purple": .purple
            case "pink": .pink
            case "brown": .brown
            case "gray": .gray
            default: .blue
            }
    }
}
