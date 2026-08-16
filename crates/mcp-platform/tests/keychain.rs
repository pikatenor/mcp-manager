#![cfg(target_os = "macos")]

use mcp_platform::{KeychainSecretStore, SecretStore};

// The app creates a fresh Entry for every set/get call, so the store must
// persist across entries — the keyring mock (no platform feature enabled)
// keeps data only inside a single entry, which silently drops every secret.
#[test]
fn keychain_store_persists_across_entries() {
    let service = format!("net.p1kachu.mcp-manager.test.{}", std::process::id());
    let account = "server:srv-test:bearer";

    KeychainSecretStore::with_service(&service)
        .set(account, "sk-value")
        .expect("set should succeed");

    let read = KeychainSecretStore::with_service(&service)
        .get(account)
        .expect("get should succeed");
    assert_eq!(read.as_deref(), Some("sk-value"));

    KeychainSecretStore::with_service(&service)
        .delete(account)
        .expect("delete should succeed");
    let after = KeychainSecretStore::with_service(&service)
        .get(account)
        .expect("get after delete should succeed");
    assert_eq!(after, None);
}
