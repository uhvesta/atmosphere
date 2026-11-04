#if os(macOS)
    import Foundation
    import LifrAPI
    import LifrSessionClient
    import Security
    import os

    /// A grant you approved, and the launcher it's bound to.
    struct StoredGrant: Hashable, Codable, Sendable {
        /// The launcher's code-signing identifier.
        var identity: String
        /// The launcher's team, so a lookalike signed by someone else is refused.
        var team: String?
        var grant: Grant
        var lastUsedAt: Date?
    }

    /// Where grants are kept between launches.
    struct GrantStorage: Sendable {
        var load: @Sendable () throws(SessionError) -> [StoredGrant]
        var save: @Sendable (_ grants: [StoredGrant]) throws(SessionError) -> Void
    }

    extension GrantStorage {
        /// One generic-password item in the login keychain, readable only by
        /// this app's signature. It never syncs.
        static func keychain(service: String) -> GrantStorage {
            let query: [CFString: Any] = [
                kSecClass: kSecClassGenericPassword, kSecAttrService: service, kSecAttrAccount: "grants",
            ]
            return GrantStorage(
                load: { () throws(SessionError) in
                    var item: CFTypeRef?
                    var search = query
                    search[kSecReturnData] = true
                    search[kSecMatchLimit] = kSecMatchLimitOne
                    let status = SecItemCopyMatching(search as CFDictionary, &item)
                    if status == errSecItemNotFound { return [] }
                    guard status == errSecSuccess, let data = item as? Data else {
                        throw .grantsNotSaved(reason: message(status))
                    }
                    do {
                        return try Wire.decoder.decode([StoredGrant].self, from: data)
                    } catch {
                        throw .grantsNotSaved(reason: "the saved grants can't be read (\(error))")
                    }
                },
                save: { (grants) throws(SessionError) in
                    let data: Data
                    do {
                        data = try Wire.encoder.encode(grants)
                    } catch {
                        throw .grantsNotSaved(reason: "the grants can't be encoded (\(error))")
                    }
                    var status = SecItemUpdate(query as CFDictionary, [kSecValueData: data] as CFDictionary)
                    if status == errSecItemNotFound {
                        var add = query
                        add[kSecValueData] = data
                        status = SecItemAdd(add as CFDictionary, nil)
                    }
                    guard status == errSecSuccess else { throw .grantsNotSaved(reason: message(status)) }
                })
        }

        /// Kept in memory, for tests.
        static func inMemory(_ grants: [StoredGrant] = []) -> GrantStorage {
            let stored = OSAllocatedUnfairLock(initialState: grants)
            return GrantStorage(
                load: { stored.withLock { $0 } },
                save: { grants in stored.withLock { $0 = grants } })
        }

        private static func message(_ status: OSStatus) -> String {
            (SecCopyErrorMessageString(status, nil) as String?) ?? "error \(status)"
        }
    }
#endif
