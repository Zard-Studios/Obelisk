import Foundation

/// Only the refresh token is persisted. Access tokens remain in the Rust session.
struct GogKeychain: CredentialStore {
    private let store: KeychainStore<StoredGogCredentials>

    init(service: String = AppIdentity.keychainService + ".gog") {
        store = KeychainStore(service: service, account: "gog-session", label: L10n.string("model.brand.gog"))
    }

    func load() async throws -> GogCredentials? {
        try await store.load().map {
            GogCredentials(userId: $0.userId, username: $0.username, refreshToken: $0.refreshToken)
        }
    }

    func save(_ credentials: GogCredentials) async throws {
        try await store.save(StoredGogCredentials(
            userId: credentials.userId,
            username: credentials.username,
            refreshToken: credentials.refreshToken
        ))
    }

    func remove() async throws {
        try await store.remove()
    }
}

private struct StoredGogCredentials: Codable {
    let userId: String
    let username: String
    let refreshToken: String
}
