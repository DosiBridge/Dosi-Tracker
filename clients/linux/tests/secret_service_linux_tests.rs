#![cfg(target_os = "linux")]

//! Real-zbus private-session-bus fake SecretService regression test.
//!
//! Spawns an ephemeral `dbus-daemon` on a private UNIX socket (no connection to real user bus or keyring),
//! serves a fake Freedesktop Secret Service daemon (`org.freedesktop.secrets`) that checks caller unique
//! bus names (`#[zbus(header)] header: Header<'_>`), verifies caller-bound session validation (rejecting
//! mismatched or disconnected callers), and verifies the production `SecretServiceStore` save/load/update/clear
//! lifecycle and cross-scope isolation.

mod support;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use dosi_tracker_linux::credentials::secret_service_available;
use dosi_tracker_linux::{
    CredentialError, CredentialStore, Credentials, SecretServiceStore, SecretServiceTransport,
    ZbusSecretServiceTransport,
};
use support::private_bus::PrivateTestBus;
use zbus::blocking::Connection;
use zbus::interface;
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Str, Value};

#[derive(Clone, Debug)]
struct StoredItem {
    label: String,
    attributes: HashMap<String, String>,
    secret: Vec<u8>,
    locked: bool,
}

#[derive(Clone, Debug)]
struct StoredSession {
    owner_unique_name: String,
    active: bool,
}

#[derive(Clone, Default)]
struct ServerState {
    sessions: Arc<Mutex<HashMap<String, StoredSession>>>,
    items: Arc<Mutex<HashMap<String, StoredItem>>>,
    session_counter: Arc<AtomicU64>,
    item_counter: Arc<AtomicU64>,
    /// The collection is locked: Unlock answers with a prompt.
    require_prompt: Arc<AtomicBool>,
    /// The user dismisses the prompt.
    dismiss_prompt: Arc<AtomicBool>,
    prompts_shown: Arc<AtomicU64>,
    prompt_window_ids: Arc<Mutex<Vec<String>>>,
}

/// The prompt object the fake hands out for a locked collection.
const FAKE_PROMPT_PATH: &str = "/org/freedesktop/secrets/prompt/1";

struct FakeSecretService {
    state: ServerState,
}

#[interface(name = "org.freedesktop.Secret.Service")]
impl FakeSecretService {
    fn search_items(
        &self,
        attributes: HashMap<String, String>,
    ) -> (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) {
        let items = self.state.items.lock().unwrap();
        let mut unlocked = Vec::new();
        let mut locked = Vec::new();
        for (path_str, item) in items.iter() {
            let mut matches = true;
            for (k, v) in &attributes {
                if item.attributes.get(k) != Some(v) {
                    matches = false;
                    break;
                }
            }
            if matches {
                if let Ok(p) = OwnedObjectPath::try_from(path_str.as_str()) {
                    if item.locked {
                        locked.push(p);
                    } else {
                        unlocked.push(p);
                    }
                }
            }
        }
        unlocked.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        locked.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        (unlocked, locked)
    }

    fn unlock(&self, objects: Vec<OwnedObjectPath>) -> (Vec<OwnedObjectPath>, OwnedObjectPath) {
        if self.state.require_prompt.load(Ordering::SeqCst) {
            return (
                Vec::new(),
                OwnedObjectPath::try_from(FAKE_PROMPT_PATH).unwrap(),
            );
        }
        let mut items = self.state.items.lock().unwrap();
        let mut unlocked = Vec::new();
        for obj in objects {
            if let Some(item) = items.get_mut(obj.as_str()) {
                item.locked = false;
                unlocked.push(obj);
            }
        }
        let prompt = OwnedObjectPath::try_from("/").unwrap();
        (unlocked, prompt)
    }

    fn open_session(
        &self,
        _algorithm: &str,
        _input: Value<'_>,
        #[zbus(header)] header: Header<'_>,
    ) -> zbus::fdo::Result<(OwnedValue, OwnedObjectPath)> {
        let sender = header
            .sender()
            .ok_or_else(|| {
                zbus::fdo::Error::Failed("Missing sender unique name in D-Bus header".into())
            })?
            .as_str()
            .to_string();

        let id = self.state.session_counter.fetch_add(1, Ordering::Relaxed) + 1;
        let session_path_str = format!("/org/freedesktop/secrets/session/{id}");
        let session_path = OwnedObjectPath::try_from(session_path_str.as_str())
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;

        {
            let mut sessions = self.state.sessions.lock().unwrap();
            sessions.insert(
                session_path_str,
                StoredSession {
                    owner_unique_name: sender,
                    active: true,
                },
            );
        }

        let output = OwnedValue::from(Str::from(""));
        Ok((output, session_path))
    }
}

struct FakeCollection {
    state: ServerState,
}

#[interface(name = "org.freedesktop.Secret.Collection")]
impl FakeCollection {
    fn create_item(
        &self,
        properties: HashMap<String, OwnedValue>,
        secret: (OwnedObjectPath, Vec<u8>, Vec<u8>, String),
        replace: bool,
        #[zbus(header)] header: Header<'_>,
    ) -> zbus::fdo::Result<(OwnedObjectPath, OwnedObjectPath)> {
        let sender = header
            .sender()
            .ok_or_else(|| {
                zbus::fdo::Error::Failed("Missing sender unique name in D-Bus header".into())
            })?
            .as_str();

        let (session_path, _params, secret_bytes, _content_type) = secret;
        let session_str = session_path.as_str();

        // Validate caller unique name binding to session
        {
            let sessions = self.state.sessions.lock().unwrap();
            let session = sessions.get(session_str).ok_or_else(|| {
                zbus::fdo::Error::Failed(format!("Session {session_str} does not exist"))
            })?;
            if !session.active {
                return Err(zbus::fdo::Error::Failed(format!(
                    "Session {session_str} is closed"
                )));
            }
            if session.owner_unique_name != sender {
                return Err(zbus::fdo::Error::AccessDenied(format!(
                    "Caller unique bus name mismatch: session bound to {} but called by {sender}",
                    session.owner_unique_name
                )));
            }
        }

        let label = properties
            .get("org.freedesktop.Secret.Item.Label")
            .and_then(|v| match &**v {
                Value::Str(s) => Some(s.as_str().to_string()),
                _ => None,
            })
            .unwrap_or_default();

        let mut attributes = HashMap::new();
        if let Some(attr_val) = properties.get("org.freedesktop.Secret.Item.Attributes") {
            if let Value::Dict(dict) = &**attr_val {
                for (k, v) in dict.iter() {
                    if let (Value::Str(ks), Value::Str(vs)) = (k, v) {
                        attributes.insert(ks.as_str().to_string(), vs.as_str().to_string());
                    }
                }
            }
        }

        let mut items = self.state.items.lock().unwrap();
        if replace {
            for (p_str, item) in items.iter_mut() {
                let mut matches = true;
                for (k, v) in &attributes {
                    if item.attributes.get(k) != Some(v) {
                        matches = false;
                        break;
                    }
                }
                if matches {
                    item.label = label;
                    item.secret = secret_bytes;
                    item.locked = false;
                    let p = OwnedObjectPath::try_from(p_str.as_str())
                        .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
                    let prompt = OwnedObjectPath::try_from("/").unwrap();
                    return Ok((p, prompt));
                }
            }
        }

        let id = self.state.item_counter.fetch_add(1, Ordering::Relaxed) + 1;
        let item_path_str = format!("/org/freedesktop/secrets/collection/default/{id}");
        items.insert(
            item_path_str.clone(),
            StoredItem {
                label,
                attributes,
                secret: secret_bytes,
                locked: false,
            },
        );

        let p = OwnedObjectPath::try_from(item_path_str.as_str())
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        let prompt = OwnedObjectPath::try_from("/").unwrap();
        Ok((p, prompt))
    }
}

struct FakeItem {
    path: String,
    state: ServerState,
}

#[interface(name = "org.freedesktop.Secret.Item")]
impl FakeItem {
    fn get_secret(
        &self,
        session: OwnedObjectPath,
        #[zbus(header)] header: Header<'_>,
    ) -> zbus::fdo::Result<(OwnedObjectPath, Vec<u8>, Vec<u8>, String)> {
        let sender = header
            .sender()
            .ok_or_else(|| {
                zbus::fdo::Error::Failed("Missing sender unique name in D-Bus header".into())
            })?
            .as_str();

        let session_str = session.as_str();
        {
            let sessions = self.state.sessions.lock().unwrap();
            let sess = sessions.get(session_str).ok_or_else(|| {
                zbus::fdo::Error::Failed(format!("Session {session_str} does not exist"))
            })?;
            if !sess.active {
                return Err(zbus::fdo::Error::Failed(format!(
                    "Session {session_str} is closed"
                )));
            }
            if sess.owner_unique_name != sender {
                return Err(zbus::fdo::Error::AccessDenied(format!(
                    "Caller unique bus name mismatch: session bound to {} but called by {sender}",
                    sess.owner_unique_name
                )));
            }
        }

        let items = self.state.items.lock().unwrap();
        let item = items
            .get(&self.path)
            .ok_or_else(|| zbus::fdo::Error::Failed(format!("Item {} not found", self.path)))?;

        if item.locked {
            return Err(zbus::fdo::Error::Failed(format!(
                "Item {} is locked",
                self.path
            )));
        }

        Ok((
            session,
            Vec::new(),
            item.secret.clone(),
            "application/json".to_string(),
        ))
    }

    fn delete(&self, #[zbus(header)] _header: Header<'_>) -> zbus::fdo::Result<OwnedObjectPath> {
        let mut items = self.state.items.lock().unwrap();
        items.remove(&self.path);
        let prompt = OwnedObjectPath::try_from("/").unwrap();
        Ok(prompt)
    }
}

struct FakeSession {
    path: String,
    state: ServerState,
}

#[interface(name = "org.freedesktop.Secret.Session")]
impl FakeSession {
    fn close(&self, #[zbus(header)] header: Header<'_>) -> zbus::fdo::Result<()> {
        let sender = header
            .sender()
            .ok_or_else(|| zbus::fdo::Error::Failed("Missing sender unique name".into()))?
            .as_str();

        let mut sessions = self.state.sessions.lock().unwrap();
        if let Some(sess) = sessions.get_mut(&self.path) {
            if sess.owner_unique_name != sender {
                return Err(zbus::fdo::Error::AccessDenied(
                    "Cannot close session owned by another connection".into(),
                ));
            }
            sess.active = false;
        }
        Ok(())
    }
}

/// `org.freedesktop.Secret.Prompt`: completing it unlocks every item.
struct FakePrompt {
    state: ServerState,
}

#[interface(name = "org.freedesktop.Secret.Prompt")]
impl FakePrompt {
    async fn prompt(
        &self,
        window_id: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        self.state
            .prompt_window_ids
            .lock()
            .unwrap()
            .push(window_id.to_string());
        self.state.prompts_shown.fetch_add(1, Ordering::SeqCst);
        let dismissed = self.state.dismiss_prompt.load(Ordering::SeqCst);
        if !dismissed {
            self.state.require_prompt.store(false, Ordering::SeqCst);
            for item in self.state.items.lock().unwrap().values_mut() {
                item.locked = false;
            }
        }
        Self::completed(&emitter, dismissed, Value::from(""))
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
    }

    #[zbus(signal)]
    async fn completed(
        emitter: &SignalEmitter<'_>,
        dismissed: bool,
        result: Value<'_>,
    ) -> zbus::Result<()>;
}

fn start_fake_secret_service(address: &str) -> Connection {
    start_fake_secret_service_with_state(address).0
}

fn start_fake_secret_service_with_state(address: &str) -> (Connection, ServerState) {
    let state = ServerState::default();

    let fake_service = FakeSecretService {
        state: state.clone(),
    };
    let fake_collection = FakeCollection {
        state: state.clone(),
    };

    let conn = zbus::blocking::connection::Builder::address(address)
        .expect("Valid address")
        .name("org.freedesktop.secrets")
        .expect("Valid name")
        .serve_at("/org/freedesktop/secrets", fake_service)
        .expect("Valid service path")
        .serve_at("/org/freedesktop/secrets/aliases/default", fake_collection)
        .expect("Valid collection path")
        .serve_at(
            FAKE_PROMPT_PATH,
            FakePrompt {
                state: state.clone(),
            },
        )
        .expect("Valid prompt path")
        .build()
        .expect("Fake SecretService connection built successfully");

    for i in 1..=50 {
        let item_path = format!("/org/freedesktop/secrets/collection/default/{i}");
        let sess_path = format!("/org/freedesktop/secrets/session/{i}");
        conn.object_server()
            .at(
                item_path.clone(),
                FakeItem {
                    path: item_path,
                    state: state.clone(),
                },
            )
            .expect("Register FakeItem");
        conn.object_server()
            .at(
                sess_path.clone(),
                FakeSession {
                    path: sess_path,
                    state: state.clone(),
                },
            )
            .expect("Register FakeSession");
    }

    (conn, state)
}

#[test]
fn test_linux_secret_service_session_caller_unique_name_binding_and_rejection() {
    let bus = PrivateTestBus::new();
    let _server = start_fake_secret_service(bus.address());

    // Connection 1 (Caller A)
    let transport_a = ZbusSecretServiceTransport::from_address(bus.address());
    let session_path = transport_a
        .open_session()
        .expect("Caller A opens session successfully");

    // Connection 2 (Caller B - completely different unique bus name)
    let transport_b = ZbusSecretServiceTransport::from_address(bus.address());

    // Caller B attempts to use Caller A's session in create_item -> MUST FAIL with AccessDenied / caller mismatch
    let mut attrs = HashMap::new();
    attrs.insert("application".into(), "dosi-tracker".into());
    attrs.insert("account_id".into(), "user@corp.com".into());
    let err = transport_b
        .create_item("test", &attrs, &session_path, b"secret_payload", true)
        .unwrap_err();

    assert!(
        matches!(err, CredentialError::SecretServiceUnavailable(_)),
        "Expected SecretServiceUnavailable, got: {:?}",
        err
    );
    let err_str = err.to_string();
    assert!(
        err_str.contains("mismatch") || err_str.contains("AccessDenied"),
        "Error message must indicate caller unique name mismatch or access denied, got: {err_str}"
    );

    // Now Caller A uses its OWN session -> SUCCEEDS
    let item_path = transport_a
        .create_item("test", &attrs, &session_path, b"secret_payload", true)
        .expect("Caller A create_item succeeds with its own session");

    // Caller B attempts to use Caller A's session in get_secret -> MUST FAIL with AccessDenied / caller mismatch
    let get_err = transport_b
        .get_secret(&item_path, &session_path)
        .unwrap_err();
    assert!(
        matches!(get_err, CredentialError::SecretServiceUnavailable(_)),
        "Expected SecretServiceUnavailable for foreign caller get_secret, got: {:?}",
        get_err
    );
    let get_err_str = get_err.to_string();
    assert!(
        get_err_str.contains("mismatch") || get_err_str.contains("AccessDenied"),
        "Error message must indicate caller unique name mismatch or access denied, got: {get_err_str}"
    );

    // Caller A uses its own session to read secret -> SUCCEEDS
    let secret = transport_a
        .get_secret(&item_path, &session_path)
        .expect("Caller A gets secret with own session");
    assert_eq!(secret, b"secret_payload");

    // Caller A closes session
    transport_a
        .close_session(&session_path)
        .expect("Caller A closes session");

    // Calling get_secret on closed session -> MUST FAIL
    let closed_err = transport_a
        .get_secret(&item_path, &session_path)
        .unwrap_err();
    assert!(
        matches!(closed_err, CredentialError::SecretServiceUnavailable(_)),
        "Expected error on closed session, got: {:?}",
        closed_err
    );
}

#[test]
fn test_linux_secret_service_store_production_crud_and_cross_scope_retention() {
    let bus = PrivateTestBus::new();
    let _server = start_fake_secret_service(bus.address());

    let transport = Arc::new(ZbusSecretServiceTransport::from_address(bus.address()));
    let store = SecretServiceStore::with_transport("dosi-tracker", transport);

    // 1. Save Alice in workspace alpha
    let mut creds_alice_alpha = Credentials::new("alice@corp.com", Some("alice-pass-alpha".into()));
    creds_alice_alpha.workspace_id = Some("ws-alpha".into());
    creds_alice_alpha.access_token = Some("tok-alice-alpha".into());
    store
        .save(&creds_alice_alpha)
        .expect("Save Alice ws-alpha succeeds");

    // 2. Save Alice in workspace beta (same account, different workspace)
    let mut creds_alice_beta = Credentials::new("alice@corp.com", Some("alice-pass-beta".into()));
    creds_alice_beta.workspace_id = Some("ws-beta".into());
    creds_alice_beta.access_token = Some("tok-alice-beta".into());
    store
        .save(&creds_alice_beta)
        .expect("Save Alice ws-beta succeeds");

    // 3. Save Bob in workspace alpha (different account, same workspace)
    let mut creds_bob_alpha = Credentials::new("bob@corp.com", Some("bob-pass-alpha".into()));
    creds_bob_alpha.workspace_id = Some("ws-alpha".into());
    creds_bob_alpha.access_token = Some("tok-bob-alpha".into());
    store
        .save(&creds_bob_alpha)
        .expect("Save Bob ws-alpha succeeds");

    // 4. Verify exact load matching and cross-scope retention
    let loaded_a_alpha = store
        .load("alice@corp.com", Some("ws-alpha"))
        .expect("Load Alice ws-alpha")
        .expect("Found");
    assert_eq!(
        loaded_a_alpha.access_token.as_deref(),
        Some("tok-alice-alpha")
    );

    let loaded_a_beta = store
        .load("alice@corp.com", Some("ws-beta"))
        .expect("Load Alice ws-beta")
        .expect("Found");
    assert_eq!(
        loaded_a_beta.access_token.as_deref(),
        Some("tok-alice-beta")
    );

    let loaded_b_alpha = store
        .load("bob@corp.com", Some("ws-alpha"))
        .expect("Load Bob ws-alpha")
        .expect("Found");
    assert_eq!(
        loaded_b_alpha.access_token.as_deref(),
        Some("tok-bob-alpha")
    );

    // 5. Update Alice in workspace alpha
    creds_alice_alpha.access_token = Some("tok-alice-alpha-updated".into());
    store
        .save(&creds_alice_alpha)
        .expect("Update Alice ws-alpha succeeds");

    let updated_a_alpha = store
        .load("alice@corp.com", Some("ws-alpha"))
        .expect("Load updated")
        .expect("Found");
    assert_eq!(
        updated_a_alpha.access_token.as_deref(),
        Some("tok-alice-alpha-updated")
    );

    // Verify other scopes remain untouched after update
    let loaded_b_alpha_after = store
        .load("bob@corp.com", Some("ws-alpha"))
        .expect("Load Bob")
        .expect("Found");
    assert_eq!(
        loaded_b_alpha_after.access_token.as_deref(),
        Some("tok-bob-alpha")
    );

    // 6. Clear Alice ws-alpha
    store
        .clear("alice@corp.com", Some("ws-alpha"))
        .expect("Clear Alice ws-alpha succeeds");
    assert_eq!(
        store
            .load("alice@corp.com", Some("ws-alpha"))
            .expect("Load cleared"),
        None
    );

    // Verify Alice ws-beta and Bob ws-alpha are still retained
    let still_a_beta = store
        .load("alice@corp.com", Some("ws-beta"))
        .expect("Load")
        .expect("Retained");
    assert_eq!(still_a_beta.access_token.as_deref(), Some("tok-alice-beta"));
    let still_b_alpha = store
        .load("bob@corp.com", Some("ws-alpha"))
        .expect("Load")
        .expect("Retained");
    assert_eq!(still_b_alpha.access_token.as_deref(), Some("tok-bob-alpha"));

    // 7. Clear remaining scopes
    store
        .clear("alice@corp.com", Some("ws-beta"))
        .expect("Clear Alice ws-beta succeeds");
    store
        .clear("bob@corp.com", Some("ws-alpha"))
        .expect("Clear Bob ws-alpha succeeds");
    assert_eq!(
        store.load("alice@corp.com", Some("ws-beta")).expect("Load"),
        None
    );
    assert_eq!(
        store.load("bob@corp.com", Some("ws-alpha")).expect("Load"),
        None
    );
}

#[test]
fn test_linux_secret_service_store_default_new_with_env_session_bus() {
    let bus = PrivateTestBus::new();
    let _server = start_fake_secret_service(bus.address());

    // Set DBUS_SESSION_BUS_ADDRESS to the private bus address
    std::env::set_var("DBUS_SESSION_BUS_ADDRESS", bus.address());

    let store = SecretServiceStore::new("dosi-tracker");
    let mut creds = Credentials::new("env_user@corp.com", Some("pass123".into()));
    creds.workspace_id = Some("env-ws".into());
    creds.access_token = Some("tok-env".into());

    store
        .save(&creds)
        .expect("Production store.save with default constructor succeeds");
    let loaded = store
        .load("env_user@corp.com", Some("env-ws"))
        .expect("load")
        .expect("found");
    assert_eq!(loaded.access_token.as_deref(), Some("tok-env"));

    store
        .clear("env_user@corp.com", Some("env-ws"))
        .expect("clear succeeds");
    assert_eq!(
        store
            .load("env_user@corp.com", Some("env-ws"))
            .expect("load"),
        None
    );
}

#[test]
fn test_linux_secret_service_account_level_item_is_one_item_per_account() {
    let bus = PrivateTestBus::new();
    let (_server, state) = start_fake_secret_service_with_state(bus.address());
    let transport = Arc::new(ZbusSecretServiceTransport::from_address(bus.address()));
    let store = SecretServiceStore::with_transport("dosi-tracker", transport);

    let mut creds = Credentials::new("Erin@Corp.com", Some("erin-pass".into()));
    creds.workspace_id = Some("ignored-for-account-items".into());
    creds.refresh_token = Some("refresh-1".into());
    store.save_account(&creds).expect("save account item");
    creds.refresh_token = Some("refresh-2".into());
    store.save_account(&creds).expect("replace account item");

    {
        let items = state.items.lock().unwrap();
        let mine: Vec<_> = items
            .values()
            .filter(|i| i.attributes.get("account_id").map(String::as_str) == Some("erin@corp.com"))
            .collect();
        assert_eq!(mine.len(), 1, "one item per account");
        assert_eq!(mine[0].attributes["workspace_id"], "");
        assert_eq!(mine[0].attributes["application"], "dosi-tracker");
    }
    let loaded = store.load_account("erin@corp.com").unwrap().unwrap();
    assert_eq!(
        loaded.refresh_token.as_deref(),
        Some("refresh-2"),
        "the rotated token"
    );
    assert_eq!(loaded.workspace_id, None);
    assert_eq!(loaded.password.as_deref(), Some("erin-pass"));

    // remember_password = false keeps the password out of the keyring.
    let forgetful = SecretServiceStore::with_transport(
        "dosi-tracker",
        Arc::new(ZbusSecretServiceTransport::from_address(bus.address())),
    )
    .with_remember_password(false);
    forgetful.save_account(&creds).unwrap();
    assert_eq!(
        forgetful
            .load_account("erin@corp.com")
            .unwrap()
            .unwrap()
            .password,
        None
    );
    let stored = state
        .items
        .lock()
        .unwrap()
        .values()
        .map(|i| String::from_utf8_lossy(&i.secret).into_owned())
        .collect::<String>();
    assert!(!stored.contains("erin-pass"));

    store.clear_account("erin@corp.com").unwrap();
    assert_eq!(store.load_account("erin@corp.com").unwrap(), None);
}

#[test]
fn test_linux_secret_service_locked_collection_needs_an_explicit_prompt() {
    let bus = PrivateTestBus::new();
    let (_server, state) = start_fake_secret_service_with_state(bus.address());
    let transport = Arc::new(ZbusSecretServiceTransport::from_address(bus.address()));
    let store = SecretServiceStore::with_transport("dosi-tracker", transport);
    let mut creds = Credentials::new("frank@corp.com", None);
    creds.refresh_token = Some("refresh".into());
    store.save_account(&creds).unwrap();

    // The keyring locks (screen lock, login without auto-unlock).
    for item in state.items.lock().unwrap().values_mut() {
        item.locked = true;
    }
    state.require_prompt.store(true, Ordering::SeqCst);

    match store.load_account("frank@corp.com") {
        Err(CredentialError::Locked { prompt }) => assert_eq!(prompt, FAKE_PROMPT_PATH),
        other => panic!("expected Locked, got {other:?}"),
    }
    assert_eq!(
        state.prompts_shown.load(Ordering::SeqCst),
        0,
        "loading never shows a prompt"
    );

    // The user dismisses the prompt: still locked.
    state.dismiss_prompt.store(true, Ordering::SeqCst);
    assert!(!store.unlock("frank@corp.com").expect("prompt completes"));
    assert!(matches!(
        store.load_account("frank@corp.com"),
        Err(CredentialError::Locked { .. })
    ));

    // The user unlocks: the item loads.
    state.dismiss_prompt.store(false, Ordering::SeqCst);
    assert!(store.unlock("frank@corp.com").unwrap());
    assert_eq!(state.prompts_shown.load(Ordering::SeqCst), 2);
    assert_eq!(
        *state.prompt_window_ids.lock().unwrap(),
        vec![String::new(), String::new()],
        "no parent window id"
    );
    let loaded = store.load_account("frank@corp.com").unwrap().unwrap();
    assert_eq!(loaded.refresh_token.as_deref(), Some("refresh"));
}

#[test]
fn test_linux_secret_service_availability_is_probed_on_the_bus() {
    let bus = PrivateTestBus::new();
    let conn = zbus::blocking::connection::Builder::address(bus.address())
        .unwrap()
        .build()
        .unwrap();
    assert!(
        !secret_service_available(&conn).unwrap(),
        "nobody owns the name yet"
    );
    let _server = start_fake_secret_service(bus.address());
    assert!(secret_service_available(&conn).unwrap());
}
