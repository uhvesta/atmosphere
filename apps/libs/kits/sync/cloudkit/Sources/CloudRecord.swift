import CloudKit
import Foundation
import SyncKit

/// How an item is kept in CloudKit: one record per item, named as the item
/// is, with the item and the device that sent it in encrypted fields, so
/// iCloud never sees what's in them.
struct CloudRecord<Snapshot: SyncSnapshot>: Sendable {
    static var type: String { "Item" }

    private static var payload: String { "payload" }
    private static var device: String { "device" }
    private static var deviceID: String { "deviceID" }

    /// The zone in the private database that holds the app's records.
    let zone: CKRecordZone.ID

    init(zone: String) {
        self.zone = CKRecordZone.ID(zoneName: zone, ownerName: CKCurrentUserDefaultName)
    }

    func id(_ name: String) -> CKRecord.ID {
        CKRecord.ID(recordName: name, zoneID: zone)
    }

    /// `item` in a record. `systemFields`, from the last save or fetch,
    /// carries the change tag CloudKit checks, so a record another device
    /// changed since is refused instead of overwritten.
    func record(
        _ item: CloudItem<Snapshot>, device: String, deviceID: UUID, systemFields: Data?
    ) throws(CloudRecordError) -> CKRecord {
        let record =
            systemFields.flatMap(Self.decodeSystemFields) ?? CKRecord(recordType: Self.type, recordID: id(item.name))
        do {
            record.encryptedValues[Self.payload] = try JSONEncoder().encode(item)
        } catch {
            throw .encodeFailed(item.name, error)
        }
        record.encryptedValues[Self.device] = device
        record.encryptedValues[Self.deviceID] = deviceID.uuidString
        return record
    }

    /// The item in `record`, and which device sent it.
    func change(_ record: CKRecord) throws(CloudRecordError) -> (change: CloudChange<Snapshot>, deviceID: UUID?) {
        let name = record.recordID.recordName
        guard let data = record.encryptedValues[Self.payload] as? Data else { throw .noPayload(name) }
        let item: CloudItem<Snapshot>
        do {
            item = try JSONDecoder().decode(CloudItem<Snapshot>.self, from: data)
        } catch {
            throw .decodeFailed(name, error)
        }
        guard item.name == name else { throw .misnamed(name, item.name) }
        let device = record.encryptedValues[Self.device] as? String ?? String(localized: "another device")
        let deviceID = (record.encryptedValues[Self.deviceID] as? String).flatMap(UUID.init(uuidString:))
        return (CloudChange(item, from: device), deviceID)
    }

    static func systemFields(of record: CKRecord) -> Data {
        let coder = NSKeyedArchiver(requiringSecureCoding: true)
        record.encodeSystemFields(with: coder)
        coder.finishEncoding()
        return coder.encodedData
    }

    private static func decodeSystemFields(_ data: Data) -> CKRecord? {
        guard let coder = try? NSKeyedUnarchiver(forReadingFrom: data) else { return nil }
        coder.requiresSecureCoding = true
        defer { coder.finishDecoding() }
        return CKRecord(coder: coder)
    }
}

/// An item couldn't be put in a record or read back from one.
enum CloudRecordError: Error, CustomStringConvertible {
    case encodeFailed(String, any Error)
    case noPayload(String)
    case decodeFailed(String, any Error)
    /// The record's name and the item in it disagree.
    case misnamed(String, String)

    var description: String {
        switch self {
        case .encodeFailed(let name, let error):
            "Couldn't encode \(name) for iCloud (\(error.localizedDescription))."
        case .noPayload(let name):
            "iCloud's \(name) has nothing in it, so it was skipped. A newer version of the app on another "
                + "device may have written it; update the app here."
        case .decodeFailed(let name, let error):
            "Couldn't read iCloud's \(name) (\(error.localizedDescription)), so it was skipped. A newer "
                + "version of the app on another device may have written it; update the app here."
        case .misnamed(let name, let inside):
            "iCloud's \(name) holds \(inside) instead, so it was skipped."
        }
    }
}

extension SyncError {
    /// What a CloudKit error means for the person.
    init(_ error: any Error) {
        guard let error = error as? CKError else {
            self = .refused(error.localizedDescription)
            return
        }
        switch error.code {
        case .notAuthenticated:
            self = .noAccount
        case .quotaExceeded:
            self = .quotaExceeded
        case .networkUnavailable, .networkFailure, .serviceUnavailable, .requestRateLimited, .zoneBusy:
            self = .unreachable(error.localizedDescription)
        case .missingEntitlement, .badContainer:
            self = .notSetUp
        default:
            self = .refused(error.localizedDescription)
        }
    }
}
