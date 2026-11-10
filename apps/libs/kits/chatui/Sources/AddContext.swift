import CaptureKit
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

#if os(iOS)
    /// What + opens on the iPhone: for a model that reads photos, the
    /// camera and the person's recent photos in a row, with Photos for the
    /// whole library, and Paste for a copied photo; then Add files, for documents whose text goes in as
    /// context.
    public struct AddContextSheet: View {
        /// What the sheet shows of the photos, for a model that reads them.
        public struct Photos: Hashable, Sendable {
            /// Whether the app may show the library. Without it, the
            /// system's picker shows in the row instead, which needs none.
            public var access: PhotosAccess
            /// The newest photos, once they're read.
            public var recents: [RecentPhoto]
            /// The recent photos attached to the next message.
            public var attached: Set<RecentPhoto.ID>
            /// Whether there's a camera to take a photo with.
            public var camera: Bool

            public init(access: PhotosAccess, recents: [RecentPhoto], attached: Set<RecentPhoto.ID>, camera: Bool) {
                self.access = access
                self.recents = recents
                self.attached = attached
                self.camera = camera
            }
        }

        let photos: Photos?
        let fileTypes: [UTType]
        let close: () -> Void
        let tapRecent: (RecentPhoto.ID) -> Void
        let photoPicked: (Data?) -> Void
        let photoTaken: (Data) -> Void
        let filesPicked: (Result<[URL], any Error>) -> Void

        @State private var picksLibrary = false
        @State private var takesPhoto = false
        @State private var importsFiles = false
        @State private var libraryPick: PhotosPickerItem?
        @State private var inlinePick: PhotosPickerItem?

        /// `photos` is nil for a model that doesn't read them. A photo
        /// picked from the library comes to `photoPicked`, nil when it
        /// can't be read; `fileTypes` are the files Add files offers.
        public init(
            photos: Photos?, fileTypes: [UTType], close: @escaping () -> Void,
            tapRecent: @escaping (RecentPhoto.ID) -> Void, photoPicked: @escaping (Data?) -> Void,
            photoTaken: @escaping (Data) -> Void, filesPicked: @escaping (Result<[URL], any Error>) -> Void
        ) {
            self.photos = photos
            self.fileTypes = fileTypes
            self.close = close
            self.tapRecent = tapRecent
            self.photoPicked = photoPicked
            self.photoTaken = photoTaken
            self.filesPicked = filesPicked
        }

        public var body: some View {
            VStack(alignment: .leading, spacing: 16) {
                header
                if let photos {
                    row(photos)
                }
                HStack(spacing: 10) {
                    Button {
                        importsFiles = true
                    } label: {
                        Label("Add files", systemImage: "arrow.up.doc")
                            .font(.body.weight(.medium))
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.horizontal, 16)
                            .frame(height: 52)
                            .background(.fill.tertiary, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                            .contentShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
                    }
                    .buttonStyle(.plain)
                    if photos != nil {
                        PasteButton(payloadType: PastedPhoto.self) { pasted in
                            let data = pasted.first?.data
                            Task { @MainActor in photoPicked(data) }
                        }
                        .labelStyle(.titleAndIcon)
                        .controlSize(.large)
                        .buttonBorderShape(.roundedRectangle(radius: 16))
                        .frame(height: 52)
                    }
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 20)
            .padding(.top, 16)
            .presentationDetents(photos == nil ? [.height(150)] : [.medium])
            .presentationCornerRadius(28)
            .fileImporter(
                isPresented: $importsFiles, allowedContentTypes: fileTypes, allowsMultipleSelection: true,
                onCompletion: filesPicked
            )
            .photosPicker(
                isPresented: $picksLibrary, selection: $libraryPick, matching: .images,
                preferredItemEncoding: .compatible
            )
            .onChange(of: libraryPick) { _, item in load(item) { libraryPick = nil } }
            .onChange(of: inlinePick) { _, item in load(item) { inlinePick = nil } }
            .fullScreenCover(isPresented: $takesPhoto) {
                CameraPicker(taken: photoTaken)
                    .ignoresSafeArea()
            }
        }

        /// Close on the left, the title, and Photos on the right for the
        /// whole library.
        private var header: some View {
            ZStack {
                Text("Add context")
                    .font(.headline)
                    .accessibilityAddTraits(.isHeader)
                HStack {
                    Button(action: close) {
                        Label("Close", systemImage: "xmark")
                    }
                    .buttonStyle(RoundButtonStyle())
                    Spacer()
                    if photos != nil {
                        Button("Photos") { picksLibrary = true }
                            .font(.body.weight(.semibold))
                    }
                }
            }
        }

        /// The camera, then the recent photos, or the system's picker when
        /// the app may not see them.
        private func row(_ photos: Photos) -> some View {
            ScrollView(.horizontal) {
                HStack(spacing: 10) {
                    if photos.camera {
                        Button {
                            takesPhoto = true
                        } label: {
                            VStack(spacing: 8) {
                                Image(systemName: "camera.fill")
                                    .font(.title2)
                                Text("Camera")
                                    .font(.callout.weight(.medium))
                            }
                            .frame(width: Self.side, height: Self.side)
                            .background(.fill.tertiary, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
                            .contentShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
                        }
                        .buttonStyle(.plain)
                    }
                    switch photos.access {
                    case .denied:
                        PhotosPicker(
                            "Photos", selection: $inlinePick, matching: .images, preferredItemEncoding: .compatible
                        )
                        .photosPickerStyle(.inline)
                        .photosPickerDisabledCapabilities([.selectionActions, .search, .collectionNavigation])
                        .photosPickerAccessoryVisibility(.hidden, edges: .all)
                        .frame(width: 320, height: Self.side)
                        .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
                    case .notAsked:
                        ProgressView()
                            .frame(width: Self.side, height: Self.side)
                    case .allowed:
                        ForEach(photos.recents) { photo in
                            thumbnail(photo, attached: photos.attached.contains(photo.id))
                        }
                    }
                }
            }
            .scrollIndicators(.hidden)
            .frame(height: Self.side)
        }

        private func thumbnail(_ photo: RecentPhoto, attached: Bool) -> some View {
            Button {
                tapRecent(photo.id)
            } label: {
                Group {
                    if let image = UIImage(data: photo.thumbnail) {
                        Image(uiImage: image)
                            .resizable()
                            .scaledToFill()
                    } else {
                        Color.secondary.opacity(0.2)
                    }
                }
                .frame(width: Self.side, height: Self.side)
                .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
                .overlay {
                    if attached {
                        RoundedRectangle(cornerRadius: 18, style: .continuous)
                            .strokeBorder(.tint, lineWidth: 3)
                    }
                }
                .overlay(alignment: .topTrailing) {
                    if attached {
                        Image(systemName: "checkmark.circle.fill")
                            .font(.title3)
                            .symbolRenderingMode(.palette)
                            .foregroundStyle(.white, .tint)
                            .padding(8)
                    }
                }
                .contentShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
            }
            .buttonStyle(.plain)
            .accessibilityLabel(Text("Photo"))
            .accessibilityAddTraits(attached ? .isSelected : [])
            .accessibilityHint(attached ? Text("Removes it from the message") : Text("Attaches it to the message"))
        }

        private func load(_ item: PhotosPickerItem?, done: @escaping () -> Void) {
            guard let item else { return }
            done()
            Task {
                photoPicked(try? await item.loadTransferable(type: Data.self))
            }
        }

        private static let side: CGFloat = 112
    }

    /// The system's camera, taking one photo, given as JPEG.
    public struct CameraPicker: UIViewControllerRepresentable {
        let taken: (Data) -> Void
        @Environment(\.dismiss) private var dismiss

        public init(taken: @escaping (Data) -> Void) {
            self.taken = taken
        }

        public func makeUIViewController(context: Context) -> UIImagePickerController {
            let picker = UIImagePickerController()
            picker.sourceType = .camera
            picker.cameraCaptureMode = .photo
            picker.delegate = context.coordinator
            return picker
        }

        public func updateUIViewController(_ picker: UIImagePickerController, context: Context) {}

        public func makeCoordinator() -> Coordinator {
            Coordinator(taken: taken, dismiss: { dismiss() })
        }

        public final class Coordinator: NSObject, UIImagePickerControllerDelegate, UINavigationControllerDelegate {
            let taken: (Data) -> Void
            let dismiss: () -> Void

            init(taken: @escaping (Data) -> Void, dismiss: @escaping () -> Void) {
                self.taken = taken
                self.dismiss = dismiss
            }

            public func imagePickerController(
                _ picker: UIImagePickerController,
                didFinishPickingMediaWithInfo info: [UIImagePickerController.InfoKey: Any]
            ) {
                if let image = info[.originalImage] as? UIImage, let data = image.jpegData(compressionQuality: 0.9) {
                    taken(data)
                }
                dismiss()
            }

            public func imagePickerControllerDidCancel(_ picker: UIImagePickerController) {
                dismiss()
            }
        }
    }

    /// A photo on the clipboard, as Paste takes it.
    private struct PastedPhoto: Transferable {
        let data: Data

        static var transferRepresentation: some TransferRepresentation {
            DataRepresentation(importedContentType: .image) { PastedPhoto(data: $0) }
        }
    }
#endif
