#![cfg(target_os = "macos")]

use std::sync::Arc;

use mcp_platform::{BundleSecretStore, KeychainSecretStore, SecretStore};

// The bundle must stay a single generic-password item that keyring upserts in
// place — that item identity is exactly what the ACL prompt attaches to.
#[test]
fn bundle_store_uses_a_single_real_keychain_item() {
    let service = format!("net.p1kachu.mcp-manager.test.{}", std::process::id());
    let account = "secrets-single-item";
    let raw = Arc::new(KeychainSecretStore::with_service(&service));
    let store = BundleSecretStore::with_account(raw.clone(), account);

    store.set("server:srv:bearer", "tok-1").unwrap();
    store.set("server:srv:env:API_TOKEN", "sk-1").unwrap();

    let persisted = raw.get(account).unwrap().expect("bundle item exists");
    let value: serde_json::Value = serde_json::from_str(&persisted).unwrap();
    assert_eq!(value["version"], 1);
    assert_eq!(value["entries"]["server:srv:bearer"], "tok-1");
    assert_eq!(value["entries"]["server:srv:env:API_TOKEN"], "sk-1");

    store.set("server:srv:bearer", "tok-2").unwrap();
    let persisted = raw.get(account).unwrap().unwrap();
    let value: serde_json::Value = serde_json::from_str(&persisted).unwrap();
    assert_eq!(value["entries"]["server:srv:bearer"], "tok-2");

    store.delete("server:srv:env:API_TOKEN").unwrap();
    let persisted = raw.get(account).unwrap().unwrap();
    let value: serde_json::Value = serde_json::from_str(&persisted).unwrap();
    assert!(value["entries"].get("server:srv:env:API_TOKEN").is_none());

    raw.delete(account).unwrap();
}

#[test]
fn bundle_lazily_loads_from_the_real_keychain() {
    let service = format!("net.p1kachu.mcp-manager.test.{}", std::process::id());
    let account = "secrets-lazy-load";
    let raw = Arc::new(KeychainSecretStore::with_service(&service));
    BundleSecretStore::with_account(raw.clone(), account)
        .set("server:srv:bearer", "tok")
        .unwrap();

    // A brand-new store over a fresh backend reads the persisted bundle.
    let reopened = BundleSecretStore::with_account(
        Arc::new(KeychainSecretStore::with_service(&service)),
        account,
    );
    assert_eq!(
        reopened.get("server:srv:bearer").unwrap().as_deref(),
        Some("tok")
    );

    raw.delete(account).unwrap();
}
