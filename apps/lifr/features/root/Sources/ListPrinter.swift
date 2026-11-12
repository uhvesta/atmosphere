#if os(macOS)
    import AppKit

    /// Prints a list: its title, then its Markdown checklist with a box for
    /// each task, ticked once closed, and its section headings in bold.
    @MainActor
    package enum ListPrinter {
        /// Shows the print panel for the list.
        static func print(title: String, markdown: String) {
            operation(title: title, markdown: markdown, info: NSPrintInfo.shared).run()
        }

        /// Prints the list to a PDF at `url` without a panel, as Save as PDF
        /// in the print panel would. Returns whether it was written.
        package static func savePDF(title: String, markdown: String, to url: URL) -> Bool {
            guard let info = NSPrintInfo.shared.copy() as? NSPrintInfo else { return false }
            info.jobDisposition = .save
            info.dictionary()[NSPrintInfo.AttributeKey.jobSavingURL] = url
            let operation = operation(title: title, markdown: markdown, info: info)
            operation.showsPrintPanel = false
            operation.showsProgressPanel = false
            return operation.run()
        }

        private static func operation(title: String, markdown: String, info: NSPrintInfo) -> NSPrintOperation {
            let width = info.paperSize.width - info.leftMargin - info.rightMargin
            let view = NSTextView(frame: NSRect(x: 0, y: 0, width: width, height: 1))
            view.textStorage?.setAttributedString(page(title: title, markdown: markdown))
            view.isVerticallyResizable = true
            view.sizeToFit()
            let operation = NSPrintOperation(view: view, printInfo: info)
            operation.jobTitle = title
            return operation
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
