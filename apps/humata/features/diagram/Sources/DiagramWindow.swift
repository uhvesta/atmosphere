import AppKit
import SwiftUI

/// Where a part of a diagram goes: the message box of the session open in
/// the main window.
package struct DiagramSender {
    /// The session's title, to say where it goes.
    package var title: String
    /// Puts the image, as PNG, with the note into that session's message box.
    package var send: @MainActor (_ png: Data, _ note: String) -> Void

    package init(title: String, send: @escaping @MainActor (_ png: Data, _ note: String) -> Void) {
        self.title = title
        self.send = send
    }
}

/// A diagram large in its own window: zoom with a pinch or the buttons,
/// scroll to pan, Fit to see it whole. Dragging across it picks a region
/// that can go to the chat as an image, with a note.
package struct DiagramWindow: View {
    let source: String
    let sender: DiagramSender?
    @Environment(\.colorScheme) private var colorScheme
    @State private var drawn: Result<NSImage, DiagramError>?
    @State private var zoom: CGFloat = 1
    @State private var pinchStart: CGFloat?
    @State private var fitted = false
    /// The region dragged out, in fractions of the diagram's size.
    @State private var region: CGRect?
    @State private var dragStart: CGPoint?
    @State private var note = ""
    @State private var sent = false

    /// Drawn this many times its size, so zooming in stays sharp.
    package static let scale = 3.0
    static let zooms: ClosedRange<CGFloat> = 0.1...8

    package init(source: String, sender: DiagramSender?) {
        self.source = source
        self.sender = sender
    }

    package var body: some View {
        Group {
            switch drawn {
            case .success(let image):
                GeometryReader { space in
                    canvas(image, in: space.size)
                        .onAppear {
                            guard !fitted else { return }
                            fitted = true
                            zoom = Self.fit(image.size, in: space.size)
                        }
                        .toolbar { toolbar(image, in: space.size) }
                }
                .safeAreaInset(edge: .bottom) {
                    if region != nil { sendBar(image) }
                }
            case .failure(let error):
                ScrollView { Broken(source: source, error: error).padding() }
            case nil:
                ProgressView("Drawing the diagram…")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(minWidth: 480, minHeight: 360)
        .task(id: colorScheme) {
            do throws(DiagramError) {
                drawn = .success(
                    try await MermaidRenderer.shared.image(source, dark: colorScheme == .dark, scale: Self.scale))
            } catch {
                drawn = .failure(error)
            }
        }
    }

    private func canvas(_ image: NSImage, in space: CGSize) -> some View {
        let size = CGSize(width: image.size.width * zoom, height: image.size.height * zoom)
        return ScrollView([.horizontal, .vertical]) {
            Image(nsImage: image)
                .resizable()
                .interpolation(.high)
                .frame(width: size.width, height: size.height)
                .overlay(alignment: .topLeading) {
                    if let region {
                        Rectangle()
                            .strokeBorder(Color.accentColor, style: StrokeStyle(lineWidth: 2, dash: [6, 3]))
                            .background(Color.accentColor.opacity(0.12))
                            .frame(width: region.width * size.width, height: region.height * size.height)
                            .offset(x: region.minX * size.width, y: region.minY * size.height)
                            .allowsHitTesting(false)
                    }
                }
                .contentShape(Rectangle())
                .gesture(select(in: size))
                .accessibilityLabel("Diagram")
                .accessibilityHint("Drag across it to pick a part to send to the chat")
                .accessibilityIdentifier("diagram.large")
                .padding(24)
                .frame(minWidth: space.width, minHeight: space.height)
        }
        .scrollIndicators(.automatic)
        .gesture(
            MagnifyGesture()
                .onChanged { value in
                    let start = pinchStart ?? zoom
                    pinchStart = start
                    zoom = Self.clamped(start * value.magnification)
                }
                .onEnded { _ in pinchStart = nil })
    }

    private func select(in size: CGSize) -> some Gesture {
        DragGesture(minimumDistance: 4)
            .onChanged { value in
                let start = dragStart ?? value.startLocation
                dragStart = start
                let rect = CGRect(
                    x: min(start.x, value.location.x), y: min(start.y, value.location.y),
                    width: abs(value.location.x - start.x), height: abs(value.location.y - start.y))
                region = Self.fraction(rect, of: size)
                sent = false
            }
            .onEnded { _ in
                dragStart = nil
                if let region, region.width < 0.01 || region.height < 0.01 { self.region = nil }
            }
    }

    @ToolbarContentBuilder
    private func toolbar(_ image: NSImage, in space: CGSize) -> some ToolbarContent {
        ToolbarItemGroup {
            Button {
                zoom = Self.clamped(zoom / 1.25)
            } label: {
                Label("Zoom Out", systemImage: "minus.magnifyingglass")
            }
            .keyboardShortcut("-", modifiers: .command)
            .help("Zoom out (⌘-)")
            Text(zoom, format: .percent.precision(.fractionLength(0)))
                .monospacedDigit()
                .frame(minWidth: 44)
                .accessibilityLabel("Zoom")
            Button {
                zoom = Self.clamped(zoom * 1.25)
            } label: {
                Label("Zoom In", systemImage: "plus.magnifyingglass")
            }
            .keyboardShortcut("=", modifiers: .command)
            .help("Zoom in (⌘=)")
            Button {
                zoom = Self.fit(image.size, in: space)
            } label: {
                Label("Fit", systemImage: "arrow.up.left.and.down.right.magnifyingglass")
            }
            .keyboardShortcut("0", modifiers: .command)
            .help("Fit the diagram in the window (⌘0)")
        }
    }

    private func sendBar(_ image: NSImage) -> some View {
        HStack(spacing: 8) {
            TextField("A note to go with it", text: $note)
                .textFieldStyle(.roundedBorder)
                .onSubmit { send(image) }
            Button("Clear") {
                region = nil
                sent = false
            }
            if sent {
                Label("In the message box", systemImage: "checkmark")
                    .foregroundStyle(.secondary)
            }
            Button(sender.map { "Send to “\($0.title)”" } ?? "Send to Chat") { send(image) }
                .buttonStyle(.borderedProminent)
                .disabled(sender == nil)
                .help(
                    sender == nil
                        ? "Open a session in the main window to send this to its chat"
                        : "Put this part of the diagram and the note in the session's message box")
        }
        .padding(10)
        .background(.bar)
    }

    private func send(_ image: NSImage) {
        guard let sender, let region, let png = Self.png(of: image, region: region) else { return }
        sender.send(png, note)
        note = ""
        sent = true
    }

    static func clamped(_ zoom: CGFloat) -> CGFloat {
        min(max(zoom, zooms.lowerBound), zooms.upperBound)
    }

    /// The zoom that shows all of a diagram of `size` in `space`, no larger
    /// than it was drawn, so it stays sharp.
    static func fit(_ size: CGSize, in space: CGSize) -> CGFloat {
        guard size.width > 0, size.height > 0 else { return 1 }
        let room = CGSize(width: max(space.width - 48, 1), height: max(space.height - 48, 1))
        return clamped(min(scale, room.width / size.width, room.height / size.height))
    }

    /// `rect` as fractions of `size`, kept inside it.
    static func fraction(_ rect: CGRect, of size: CGSize) -> CGRect {
        guard size.width > 0, size.height > 0 else { return .zero }
        let unit = CGRect(
            x: rect.minX / size.width, y: rect.minY / size.height,
            width: rect.width / size.width, height: rect.height / size.height)
        return unit.intersection(CGRect(x: 0, y: 0, width: 1, height: 1))
    }

    /// The part of `image` in `region`, a fraction of it, as PNG.
    static func png(of image: NSImage, region: CGRect) -> Data? {
        guard let cg = image.cgImage(forProposedRect: nil, context: nil, hints: nil) else { return nil }
        let pixels = CGRect(
            x: region.minX * CGFloat(cg.width), y: region.minY * CGFloat(cg.height),
            width: region.width * CGFloat(cg.width), height: region.height * CGFloat(cg.height)
        ).integral
        guard let part = cg.cropping(to: pixels) else { return nil }
        return NSBitmapImageRep(cgImage: part).representation(using: .png, properties: [:])
    }
}
