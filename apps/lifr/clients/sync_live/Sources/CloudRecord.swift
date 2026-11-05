import CloudKit
import Foundation
import LifrSyncClient

/// How an item is kept in CloudKit: one record per item, named as the item
/// is, with the item and the device that sent it in encrypted fields, so
/// iCloud never sees what's in them.
enum CloudRecord {
    static let type = "Item"
    static let zone = CKRecordZone.ID(zoneName: "Lifr", ownerName: CKCurrentUserDefaultName)

    private static let payload = "payload"
    private static let device = "device"
    private static let deviceID = "deviceID"

    static func id(_ name: String) -> CKRecord.ID {
        CKRecord.ID(recordName: name, zoneID: zone)
    }

    /// `item` in a record. `systemFields`, from the last save or fetch,
    /// carries the change tag CloudKit checks, so a record another device
    /// changed since is refused instead of overwritten.
    static func record(
        _ item: CloudItem, device: String, deviceID: UUID, systemFields: Data?
    ) throws(CloudRecordError) -> CKRecord {
        let record = systemFields.flatMap(decodeSystemFields) ?? CKRecord(recordType: type, recordID: id(item.name))
        do {
            record.encryptedValues[payload] = try JSONEncoder().encode(item)
        } catch {
            throw .encodeFailed(item.name, error)
        }
        record.encryptedValues[self.device] = device
        record.encryptedValues[self.deviceID] = deviceID.uuidString
        return record
    }

    /// The item in `record`, and which device sent it.
    static func change(_ record: CKRecord) throws(CloudRecordError) -> (change: CloudChange, deviceID: UUID?) {
        let name = record.recordID.recordName
        guard let data = record.encryptedValues[payload] as? Data else { throw .noPayload(name) }
        let item: CloudItem
        do {
            item = try JSONDecoder().decode(CloudItem.self, from: data)
        } catch {
            throw .decodeFailed(name, error)
        }
        guard item.name == name else { throw .misnamed(name, item.name) }
        let device = record.encryptedValues[self.device] as? String ?? String(localized: "another device")
        let deviceID = (record.encryptedValues[self.deviceID] as? String).flatMap(UUID.init(uuidString:))
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
            "iCloud's \(name) has nothing in it, so it was skipped. A newer Lifr on another device may have "
                + "written it; update Lifr here."
        case .decodeFailed(let name, let error):
            "Couldn't read iCloud's \(name) (\(error.localizedDescription)), so it was skipped. A newer Lifr "
                + "on another device may have written it; update Lifr here."
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
