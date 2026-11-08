import Foundation

#if canImport(UIKit)
    import Photos
    import UIKit
#endif

/// Whether the app may show the person's photos.
public enum PhotosAccess: Hashable, Sendable {
    /// The app hasn't asked yet.
    case notAsked
    /// All of them, or those the person chose.
    case allowed
    /// The person said no, or the device doesn't allow it.
    case denied
}

/// One of the person's recent photos, as a small square to pick.
public struct RecentPhoto: Identifiable, Hashable, Sendable {
    /// The photo library's identifier for it.
    public var id: String
    /// A JPEG of it, about the size it's shown at.
    public var thumbnail: Data

    public init(id: String, thumbnail: Data) {
        self.id = id
        self.thumbnail = thumbnail
    }
}

/// Why a photo from the library couldn't be read.
public enum PhotoLibraryError: Error, Hashable, Sendable, CustomStringConvertible {
    /// The photo is gone, or the app may no longer see it.
    case missing
    /// The library couldn't give its data, with the system's reason.
    case unreadable(String)

    public var description: String {
        switch self {
        case .missing:
            String(localized: "That photo is no longer in your library, or access to it was turned off. Pick another.")
        case .unreadable(let reason):
            String(
                localized: """
                    Couldn't read that photo (\(reason)). If it's in iCloud, check you're online so it can \
                    download, then pick it again.
                    """)
        }
    }
}

/// The person's photo library on the iPhone, read only when they pick from
/// it. On the Mac it's never asked, as photos come from the system picker.
public enum PhotoLibrary {
    /// What the app may see now, without asking.
    public static var access: PhotosAccess {
        #if canImport(UIKit)
            switch PHPhotoLibrary.authorizationStatus(for: .readWrite) {
            case .authorized, .limited: .allowed
            case .notDetermined: .notAsked
            default: .denied
            }
        #else
            .denied
        #endif
    }

    /// Asks to see the photos if the app hasn't yet, then says what it may.
    public static func requestAccess() async -> PhotosAccess {
        #if canImport(UIKit)
            switch await PHPhotoLibrary.requestAuthorization(for: .readWrite) {
            case .authorized, .limited: .allowed
            case .notDetermined: .notAsked
            default: .denied
            }
        #else
            .denied
        #endif
    }

    /// The newest `limit` photos, as squares `side` points across.
    public static func recent(limit: Int, side: Double) async -> [RecentPhoto] {
        #if canImport(UIKit)
            let options = PHFetchOptions()
            options.sortDescriptors = [NSSortDescriptor(key: "creationDate", ascending: false)]
            options.fetchLimit = limit
            let assets = PHAsset.fetchAssets(with: .image, options: options)
            var found: [PHAsset] = []
            assets.enumerateObjects { asset, _, _ in found.append(asset) }
            let scale = await MainActor.run { UIScreen.main.scale }
            let size = CGSize(width: side * scale, height: side * scale)
            var photos: [RecentPhoto] = []
            for asset in found {
                if let data = await thumbnail(of: asset, size: size) {
                    photos.append(RecentPhoto(id: asset.localIdentifier, thumbnail: data))
                }
            }
            return photos
        #else
            []
        #endif
    }

    /// The photo itself, in the library's format, downloading it from
    /// iCloud if it has to.
    public static func data(of id: String) async throws(PhotoLibraryError) -> Data {
        #if canImport(UIKit)
            guard let asset = PHAsset.fetchAssets(withLocalIdentifiers: [id], options: nil).firstObject else {
                throw .missing
            }
            let options = PHImageRequestOptions()
            options.isNetworkAccessAllowed = true
            options.deliveryMode = .highQualityFormat
            options.version = .current
            let result: Result<Data, PhotoLibraryError> = await withCheckedContinuation { continuation in
                let manager = PHImageManager.default()
                manager.requestImageDataAndOrientation(for: asset, options: options) { data, _, _, info in
                    if let data {
                        continuation.resume(returning: .success(data))
                    } else {
                        let error = info?[PHImageErrorKey] as? NSError
                        continuation.resume(returning: .failure(.unreadable(error?.localizedDescription ?? "no data")))
                    }
                }
            }
            return try result.get()
        #else
            throw .missing
        #endif
    }

    #if canImport(UIKit)
        private static func thumbnail(of asset: PHAsset, size: CGSize) async -> Data? {
            let options = PHImageRequestOptions()
            options.deliveryMode = .highQualityFormat
            options.resizeMode = .fast
            options.isNetworkAccessAllowed = true
            options.isSynchronous = false
            return await withCheckedContinuation { continuation in
                PHImageManager.default().requestImage(
                    for: asset, targetSize: size, contentMode: .aspectFill, options: options
                ) { image, _ in
                    continuation.resume(returning: image?.jpegData(compressionQuality: 0.8))
                }
            }
        }
    #endif
}

/// The device's camera, for taking a photo to send.
public enum Camera {
    /// Whether there's a camera to take one with: never on the Mac or the
    /// simulator.
    @MainActor public static var isAvailable: Bool {
        #if canImport(UIKit)
            UIImagePickerController.isSourceTypeAvailable(.camera)
        #else
            false
        #endif
    }
}
