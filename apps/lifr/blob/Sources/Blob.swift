import Foundation
import libzstd

/// How a blob's value is written before it's compressed.
package enum BlobEncoding: UInt8, Hashable, Sendable {
    case json = 1
    // 2 is kept for protocol buffers.
}

/// How a blob's payload is compressed.
package enum BlobCodec: UInt8, Hashable, Sendable {
    case none = 0
    case zstd = 1
}

/// What a blob says about its own bytes, so a reader can tell any variant
/// it knows from one it doesn't, and formats can change without migrating
/// what's already saved.
package struct BlobFormat: Hashable, Sendable {
    package var encoding: BlobEncoding
    package var codec: BlobCodec
    /// The zstd dictionary the payload was compressed with; 0 for none.
    package var dictionary: UInt64

    package init(encoding: BlobEncoding, codec: BlobCodec, dictionary: UInt64 = 0) {
        self.encoding = encoding
        self.codec = codec
        self.dictionary = dictionary
    }

    /// What Lifr writes now: JSON, compressed with zstd at its highest level.
    package static let current = BlobFormat(encoding: .json, codec: .zstd)
    /// What Lifr wrote before blobs had a header: plain JSON.
    package static let legacy = BlobFormat(encoding: .json, codec: .none)
}

/// Why a blob couldn't be written or read.
package enum BlobError: Error, CustomStringConvertible {
    /// The value couldn't be encoded.
    case encodeFailed(any Error)
    /// The bytes decompressed, but don't hold the type asked for.
    case decodeFailed(any Error)
    /// zstd couldn't compress the payload; `reason` is its error name.
    case compressFailed(reason: String)
    /// zstd couldn't decompress the payload; `reason` is its error name.
    case decompressFailed(reason: String)
    /// The header names an encoding this version doesn't know.
    case unknownEncoding(UInt8)
    /// The header names a compression this version doesn't know.
    case unknownCodec(UInt8)
    /// The header names a zstd dictionary this version doesn't have.
    case unknownDictionary(UInt64)
    /// The bytes have neither a blob header nor JSON, or end early.
    case unreadable

    package var description: String {
        switch self {
        case .encodeFailed(let error):
            "Lifr couldn't save this record (\(error)). Nothing was changed; try again, and report it if it keeps "
                + "happening."
        case .decodeFailed(let error):
            "Lifr couldn't read a saved record (\(error)). It may have been written by a newer Lifr: update Lifr "
                + "on this device. If it's up to date, report it; the record is left as it is."
        case .compressFailed(let reason):
            "Lifr couldn't compress a record (\(reason)). Nothing was changed; try again."
        case .decompressFailed(let reason):
            "A saved record is damaged (\(reason)) and can't be read. It's left as it is; other records still work."
        case .unknownEncoding(let byte):
            "A saved record uses an encoding (\(byte)) this version of Lifr doesn't know. Update Lifr on this "
                + "device to read it; it's left as it is until then."
        case .unknownCodec(let byte):
            "A saved record uses a compression (\(byte)) this version of Lifr doesn't know. Update Lifr on this "
                + "device to read it; it's left as it is until then."
        case .unknownDictionary(let id):
            "A saved record was compressed with a dictionary (\(id)) this version of Lifr doesn't have. Update "
                + "Lifr on this device to read it; it's left as it is until then."
        case .unreadable:
            "A saved record is damaged and can't be read. It's left as it is; other records still work."
        }
    }
}

/// The one way Lifr turns values into the bytes it persists and back.
///
/// A blob is "LB", an encoding byte, a codec byte, the dictionary ID as a
/// varint, then the payload. Bytes without the header that start with `{`
/// or `[` are JSON from before blobs had one.
package enum Blob {
    static let magic: [UInt8] = [UInt8(ascii: "L"), UInt8(ascii: "B")]
    /// zstd's highest level: the blobs are small and written rarely, so
    /// size matters more than time.
    static let level: Int32 = 22

    /// `value` in `format`. A compressed payload that isn't smaller than
    /// the plain one is stored uncompressed, which the header records.
    package static func encode(_ value: some Encodable, as format: BlobFormat = .current) throws(BlobError) -> Data {
        let payload: Data
        do {
            switch format.encoding {
            case .json: payload = try JSONEncoder().encode(value)
            }
        } catch {
            throw .encodeFailed(error)
        }
        guard format.dictionary == 0 else { throw .unknownDictionary(format.dictionary) }
        var written = BlobFormat(encoding: format.encoding, codec: .none)
        var body = payload
        if format.codec == .zstd {
            let compressed = try compress(payload)
            if compressed.count < payload.count {
                written.codec = .zstd
                body = compressed
            }
        }
        var data = Data(magic)
        data.append(written.encoding.rawValue)
        data.append(written.codec.rawValue)
        appendVarint(written.dictionary, to: &data)
        data.append(body)
        return data
    }

    /// The `T` in `data`, in any format this version knows.
    package static func decode<T: Decodable>(_ type: T.Type, from data: Data) throws(BlobError) -> T {
        let (format, start) = try header(of: data)
        var payload = Data(data[start...])
        switch format.codec {
        case .none: break
        case .zstd: payload = try decompress(payload)
        }
        do {
            switch format.encoding {
            case .json: return try JSONDecoder().decode(type, from: payload)
            }
        } catch {
            throw .decodeFailed(error)
        }
    }

    /// How `data` was written: `.legacy` for headerless JSON.
    package static func format(of data: Data) throws(BlobError) -> BlobFormat {
        try header(of: data).format
    }

    private static func header(of data: Data) throws(BlobError) -> (format: BlobFormat, payload: Data.Index) {
        let bytes = [UInt8](data.prefix(16))
        if let first = bytes.first, first == UInt8(ascii: "{") || first == UInt8(ascii: "[") {
            return (.legacy, data.startIndex)
        }
        guard bytes.count >= 4, Array(bytes[0..<2]) == magic else { throw .unreadable }
        guard let encoding = BlobEncoding(rawValue: bytes[2]) else { throw .unknownEncoding(bytes[2]) }
        guard let codec = BlobCodec(rawValue: bytes[3]) else { throw .unknownCodec(bytes[3]) }
        var index = 4
        guard let dictionary = readVarint(bytes, at: &index) else { throw .unreadable }
        guard dictionary == 0 else { throw .unknownDictionary(dictionary) }
        return (BlobFormat(encoding: encoding, codec: codec, dictionary: dictionary), data.startIndex + index)
    }

    // MARK: - Varints

    /// Little-endian base 128, as protocol buffers write them.
    static func appendVarint(_ value: UInt64, to data: inout Data) {
        var value = value
        while value >= 0x80 {
            data.append(UInt8(value & 0x7F) | 0x80)
            value >>= 7
        }
        data.append(UInt8(value))
    }

    /// The varint at `index`, moving `index` past it; nil when the bytes
    /// end first or it doesn't fit 64 bits.
    static func readVarint(_ bytes: [UInt8], at index: inout Int) -> UInt64? {
        var value: UInt64 = 0
        var shift: UInt64 = 0
        while index < bytes.count, shift < 64 {
            let byte = bytes[index]
            index += 1
            value |= UInt64(byte & 0x7F) << shift
            if byte & 0x80 == 0 { return value }
            shift += 7
        }
        return nil
    }

    // MARK: - zstd

    private static func compress(_ input: Data) throws(BlobError) -> Data {
        let capacity = ZSTD_compressBound(input.count)
        var output = Data(count: capacity)
        let size = output.withUnsafeMutableBytes { out in
            input.withUnsafeBytes { source in
                ZSTD_compress(out.baseAddress, capacity, source.baseAddress, input.count, level)
            }
        }
        if ZSTD_isError(size) != 0 { throw .compressFailed(reason: String(cString: ZSTD_getErrorName(size))) }
        output.count = size
        return output
    }

    /// Frames Lifr writes record their size; one that doesn't, or claims
    /// more than `limit`, isn't one of Lifr's.
    private static let limit: UInt64 = 256 << 20

    private static func decompress(_ input: Data) throws(BlobError) -> Data {
        let expected = input.withUnsafeBytes { ZSTD_getFrameContentSize($0.baseAddress, input.count) }
        // ZSTD_CONTENTSIZE_UNKNOWN and ZSTD_CONTENTSIZE_ERROR are the two largest values.
        guard expected <= limit else { throw .decompressFailed(reason: "Frame size unknown or too large") }
        var output = Data(count: Int(expected))
        let size = output.withUnsafeMutableBytes { out in
            input.withUnsafeBytes { source in
                ZSTD_decompress(out.baseAddress, Int(expected), source.baseAddress, input.count)
            }
        }
        if ZSTD_isError(size) != 0 { throw .decompressFailed(reason: String(cString: ZSTD_getErrorName(size))) }
        guard size == Int(expected) else { throw .decompressFailed(reason: "Frame shorter than it says") }
        return output
    }
}
