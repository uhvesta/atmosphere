#if os(macOS)
    import AppKit

    /// Prints a list: its title, then its Markdown checklist with a box for
    /// each task, ticked once closed, and its section headings in bold.
    @MainActor
    enum ListPrinter {
        static func print(title: String, markdown: String) {
            let info = NSPrintInfo.shared
            let width = info.paperSize.width - info.leftMargin - info.rightMargin
            let view = NSTextView(frame: NSRect(x: 0, y: 0, width: width, height: 1))
            view.textStorage?.setAttributedString(page(title: title, markdown: markdown))
            view.isVerticallyResizable = true
            view.sizeToFit()
            let operation = NSPrintOperation(view: view, printInfo: info)
            operation.jobTitle = title
            operation.run()
        }

        private static func page(title: String, markdown: String) -> NSAttributedString {
            let page = NSMutableAttributedString()
            func add(_ text: some StringProtocol, size: CGFloat = 12, bold: Bool = false) {
                let font = bold ? NSFont.boldSystemFont(ofSize: size) : NSFont.systemFont(ofSize: size)
                page.append(NSAttributedString(string: String(text) + "\n", attributes: [.font: font]))
            }
            add(title + "\n", size: 20, bold: true)
            for line in markdown.components(separatedBy: "\n") {
                if line.hasPrefix("## ") {
                    add("\n" + line.dropFirst(3), size: 14, bold: true)
                } else if line.hasPrefix("- [x] ") {
                    add("☑ " + line.dropFirst(6))
                } else if line.hasPrefix("- [ ] ") {
                    add("☐ " + line.dropFirst(6))
                } else if !line.isEmpty {
                    add(line)
                }
            }
            return page
        }
    }
#endif
