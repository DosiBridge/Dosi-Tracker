//! Fake `org.kde.StatusNotifierWatcher` recording tray item registrations,
//! plus helpers that read a registered item and its DBusMenu the way a tray
//! host does (properties of `/StatusNotifierItem`, `GetLayout` and `Event`
//! on `/MenuBar`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zbus::blocking::Connection;
use zbus::interface;
use zbus::zvariant::{OwnedValue, Value};

pub const NAME: &str = "org.kde.StatusNotifierWatcher";
pub const PATH: &str = "/StatusNotifierWatcher";
pub const ITEM_PATH: &str = "/StatusNotifierItem";
pub const ITEM_INTERFACE: &str = "org.kde.StatusNotifierItem";
pub const MENU_PATH: &str = "/MenuBar";
pub const MENU_INTERFACE: &str = "com.canonical.dbusmenu";

struct Watcher {
    items: Arc<Mutex<Vec<String>>>,
}

#[interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    fn register_status_notifier_item(&self, service: &str) {
        self.items.lock().unwrap().push(service.to_owned());
    }

    fn register_status_notifier_host(&self, _service: &str) {}

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.items.lock().unwrap().clone()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }
}

/// A running fake watcher; it leaves the bus (its name goes) when dropped.
pub struct FakeWatcher {
    _conn: Connection,
    items: Arc<Mutex<Vec<String>>>,
}

impl FakeWatcher {
    pub fn start(bus_address: &str) -> Self {
        let items = Arc::new(Mutex::new(Vec::new()));
        let conn = zbus::blocking::connection::Builder::address(bus_address)
            .unwrap()
            .name(NAME)
            .unwrap()
            .serve_at(
                PATH,
                Watcher {
                    items: items.clone(),
                },
            )
            .unwrap()
            .build()
            .expect("fake StatusNotifierWatcher");
        Self { _conn: conn, items }
    }

    /// Every registration so far, oldest first.
    pub fn items(&self) -> Vec<String> {
        self.items.lock().unwrap().clone()
    }

    /// The first registered item, waiting at most `timeout`.
    pub fn wait_item(&self, timeout: Duration) -> Option<String> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(item) = self.items().first().cloned() {
                return Some(item);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// A client connection on the bus, as a tray host has.
pub fn host(bus_address: &str) -> Connection {
    zbus::blocking::connection::Builder::address(bus_address)
        .unwrap()
        .build()
        .expect("host connection")
}

/// Property `name` of the item served by `service`.
pub fn item_property(conn: &Connection, service: &str, name: &str) -> OwnedValue {
    let reply = conn
        .call_method(
            Some(service),
            ITEM_PATH,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(ITEM_INTERFACE, name),
        )
        .unwrap_or_else(|e| panic!("Get {name}: {e}"));
    reply.body().deserialize::<OwnedValue>().unwrap()
}

/// `value` without the variant wrappers around it.
pub fn unwrap_variant<'a>(value: &'a Value<'a>) -> &'a Value<'a> {
    match value {
        Value::Value(inner) => unwrap_variant(inner),
        other => other,
    }
}

pub fn string(value: &Value<'_>) -> String {
    match unwrap_variant(value) {
        Value::Str(s) => s.to_string(),
        other => panic!("not a string: {other:?}"),
    }
}

fn int(value: &Value<'_>) -> i32 {
    match unwrap_variant(value) {
        Value::I32(n) => *n,
        other => panic!("not an int: {other:?}"),
    }
}

fn bytes(value: &Value<'_>) -> Vec<u8> {
    match unwrap_variant(value) {
        Value::Array(array) => array
            .iter()
            .map(|b| match b {
                Value::U8(b) => *b,
                other => panic!("not a byte: {other:?}"),
            })
            .collect(),
        other => panic!("not bytes: {other:?}"),
    }
}

/// An `a(iiay)` pixmap list as (width, height, data).
pub fn pixmaps(value: &Value<'_>) -> Vec<(i32, i32, Vec<u8>)> {
    match unwrap_variant(value) {
        Value::Array(array) => array
            .iter()
            .map(|item| match item {
                Value::Structure(s) => {
                    let f = s.fields();
                    (int(&f[0]), int(&f[1]), bytes(&f[2]))
                }
                other => panic!("not a pixmap: {other:?}"),
            })
            .collect(),
        other => panic!("not a pixmap list: {other:?}"),
    }
}

/// The `(sa(iiay)ss)` tooltip as (icon name, title, description).
pub fn tooltip(value: &Value<'_>) -> (String, String, String) {
    match unwrap_variant(value) {
        Value::Structure(s) => {
            let f = s.fields();
            (string(&f[0]), string(&f[2]), string(&f[3]))
        }
        other => panic!("not a tooltip: {other:?}"),
    }
}

/// One DBusMenu item: its id, label, whether it is enabled, and its
/// children.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuNode {
    pub id: i32,
    pub label: String,
    pub enabled: bool,
    pub checked: Option<bool>,
    pub children: Vec<MenuNode>,
}

impl MenuNode {
    /// The first item (depth first) labelled `label`.
    pub fn find(&self, label: &str) -> Option<&MenuNode> {
        if self.label == label {
            return Some(self);
        }
        self.children.iter().find_map(|child| child.find(label))
    }

    /// The labels of the top level, separators as "".
    pub fn labels(&self) -> Vec<String> {
        self.children.iter().map(|c| c.label.clone()).collect()
    }
}

fn node(id: i32, props: &HashMap<String, OwnedValue>, children: &[OwnedValue]) -> MenuNode {
    let label = props.get("label").map(|v| string(v)).unwrap_or_default();
    let enabled = props
        .get("enabled")
        .map(|v| matches!(unwrap_variant(v), Value::Bool(true)))
        .unwrap_or(true);
    let checked = props.get("toggle-state").map(|v| int(v) == 1);
    let children = children
        .iter()
        .map(|child| match unwrap_variant(child) {
            Value::Structure(s) => {
                let f = s.fields();
                let id = int(&f[0]);
                let props: HashMap<String, OwnedValue> = match &f[1] {
                    Value::Dict(dict) => dict
                        .iter()
                        .map(|(k, v)| {
                            (
                                string(k),
                                OwnedValue::try_from(v.try_clone().unwrap()).unwrap(),
                            )
                        })
                        .collect(),
                    other => panic!("not properties: {other:?}"),
                };
                let kids: Vec<OwnedValue> = match &f[2] {
                    Value::Array(array) => array
                        .iter()
                        .map(|v| OwnedValue::try_from(v.try_clone().unwrap()).unwrap())
                        .collect(),
                    other => panic!("not children: {other:?}"),
                };
                node(id, &props, &kids)
            }
            other => panic!("not a menu item: {other:?}"),
        })
        .collect();
    MenuNode {
        id,
        label,
        enabled,
        checked,
        children,
    }
}

/// A DBusMenu item as `GetLayout` sends it: id, properties, children.
type RawItem = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

/// The whole menu of the item served by `service` (`GetLayout(0, -1, [])`).
pub fn menu_layout(conn: &Connection, service: &str) -> MenuNode {
    let reply = conn
        .call_method(
            Some(service),
            MENU_PATH,
            Some(MENU_INTERFACE),
            "GetLayout",
            &(0i32, -1i32, Vec::<String>::new()),
        )
        .expect("GetLayout");
    let (_revision, (id, props, children)): (u32, RawItem) =
        reply.body().deserialize().expect("layout");
    node(id, &props, &children)
}

/// Click menu item `id` (`Event(id, "clicked", 0, 0)`).
pub fn click(conn: &Connection, service: &str, id: i32) {
    conn.call_method(
        Some(service),
        MENU_PATH,
        Some(MENU_INTERFACE),
        "Event",
        &(id, "clicked", Value::I32(0), 0u32),
    )
    .expect("Event clicked");
}
