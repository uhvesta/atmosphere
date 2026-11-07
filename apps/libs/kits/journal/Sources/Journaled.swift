import Foundation

/// A record the journal can track, compare field by field, revert and
/// merge. An app with several kinds of record usually conforms one enum
/// over all of them, so a journal entry can hold any of its records.
public protocol Journaled: Hashable, Codable, Sendable {
    /// The kinds of record, such as task or project.
    associatedtype Kind: Hashable & Codable & Sendable
    /// The fields a change can touch. Bookkeeping every change bumps, such
    /// as an updated-at time, shouldn't be one, or every field merge would
    /// conflict.
    associatedtype Field: Hashable & Codable & Sendable
    /// A field's value, compared to tell whether the field changed.
    associatedtype Value: Hashable & Sendable

    var id: RecordID<Kind> { get }

    /// The fields of `kind`'s records, in the order people see them.
    static func fields(of kind: Kind) -> [Field]

    /// The record's fields that have a value.
    var fieldValues: [Field: Value] { get }

    /// This record with `field` copied from `other`, a version of the same
    /// record.
    func copying(_ field: Field, from other: Self) -> Self

    /// The fields that merge as one with `field`, such as where a task is
    /// filed when that's several fields. By default, `field` alone.
    static func group(of field: Field, in kind: Kind) -> [Field]

    /// The field set while a record is in the trash, or nil when records
    /// aren't trashed. Trashing on one device and editing on another is a
    /// conflict about the whole record. By default, nil.
    static var trashField: Field? { get }

    /// This version, the result of a merge, with bookkeeping taken from
    /// `other`, such as the later of two activity times. By default, as it
    /// is.
    func keepingBookkeeping(of other: Self) -> Self
}

extension Journaled {
    public static func group(of field: Field, in kind: Kind) -> [Field] { [field] }
    public static var trashField: Field? { nil }
    public func keepingBookkeeping(of other: Self) -> Self { self }

    /// Whether the record is in the trash.
    public var isTrashed: Bool { Self.trashField.map { fieldValues[$0] != nil } ?? false }
}

/// Which record: its kind and ID.
public struct RecordID<Kind: Hashable & Codable & Sendable>: Hashable, Codable, Sendable {
    public var kind: Kind
    public var id: UUID

    public init(_ kind: Kind, _ id: UUID) {
        self.kind = kind
        self.id = id
    }
}
