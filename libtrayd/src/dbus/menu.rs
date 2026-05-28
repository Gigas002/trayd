//! DBusMenu (`com.canonical.dbusmenu`) proxy and layout helpers.
//!
//! External callers use [`crate::TrayHost`]; these functions are internal.

use std::collections::HashMap;

use zbus::zvariant::OwnedValue;

use crate::error::TraydError;
use crate::model::MenuNode;

// ── DBusMenu proxy ────────────────────────────────────────────────────────────

/// Client proxy for `com.canonical.dbusmenu`.
#[zbus::proxy(
    interface = "com.canonical.dbusmenu",
    default_path = "/MenuBar",
    gen_blocking = false
)]
pub(crate) trait DbusMenu {
    /// Fetch the layout tree rooted at `parent_id`.
    ///
    /// `recursion_depth = 1` returns only direct children (grandchildren have
    /// empty `av`).  `property_names = []` returns all properties.
    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: &[&str],
    ) -> zbus::Result<(u32, (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>))>;

    /// Send an event to a menu node (e.g. `"clicked"`).
    fn event(
        &self,
        id: i32,
        event_id: &str,
        data: &zbus::zvariant::Value<'_>,
        timestamp: u32,
    ) -> zbus::Result<()>;

    /// Signal that a submenu is about to be shown; returns `true` if the
    /// layout needs to be refreshed before displaying.
    fn about_to_show(&self, id: i32) -> zbus::Result<bool>;

    /// Emitted when the menu layout or properties change.
    #[zbus(signal)]
    fn layout_updated(&self, revision: u32, parent: i32) -> zbus::Result<()>;
}

// ── Public helpers ────────────────────────────────────────────────────────────

/// Build a [`DbusMenuProxy`] at the given `(service, menu_path)`.
pub(crate) async fn menu_proxy<'a>(
    conn: &'a zbus::Connection,
    service: &'a str,
    menu_path: &'a str,
) -> Result<DbusMenuProxy<'a>, TraydError> {
    Ok(DbusMenuProxy::builder(conn)
        .destination(service)?
        .path(menu_path)?
        .build()
        .await?)
}

/// Fetch the direct children of `parent_id` from the DBusMenu at
/// `(service, menu_path)`.
///
/// Uses `recursion_depth = 1` so grandchildren are not included.
pub(crate) async fn get_menu_layout(
    conn: &zbus::Connection,
    service: &str,
    menu_path: &str,
    parent_id: i32,
) -> Result<Vec<MenuNode>, TraydError> {
    let proxy = menu_proxy(conn, service, menu_path).await?;
    let (_revision, (_root_id, _root_props, children)) =
        proxy.get_layout(parent_id, 1, &[]).await?;
    Ok(children.iter().filter_map(parse_node).collect())
}

/// Send a `"clicked"` event for `node_id` to the DBusMenu at
/// `(service, menu_path)`.
pub(crate) async fn send_menu_event(
    conn: &zbus::Connection,
    service: &str,
    menu_path: &str,
    node_id: i32,
) -> Result<(), TraydError> {
    use zbus::zvariant::Value;
    let proxy = menu_proxy(conn, service, menu_path).await?;
    // timestamp 0 is accepted by all known implementations
    proxy.event(node_id, "clicked", &Value::I32(0), 0).await?;
    Ok(())
}

/// Call `AboutToShow` for the given `node_id`; returns `true` if the caller
/// should refresh the layout before displaying the submenu.
pub(crate) async fn about_to_show(
    conn: &zbus::Connection,
    service: &str,
    menu_path: &str,
    node_id: i32,
) -> Result<bool, TraydError> {
    let proxy = menu_proxy(conn, service, menu_path).await?;
    Ok(proxy.about_to_show(node_id).await?)
}

// ── Internal parsing ──────────────────────────────────────────────────────────

/// Parse one `OwnedValue` from the `av` children array into a [`MenuNode`].
///
/// Each element is a D-Bus struct `(i32, a{sv}, av)` wrapped in a variant.
fn parse_node(ov: &OwnedValue) -> Option<MenuNode> {
    use zbus::zvariant::Value;

    // Unwrap the outer variant if present.
    let val = match &**ov {
        Value::Value(inner) => &**inner,
        other => other,
    };

    let fields = match val {
        Value::Structure(s) => s.fields(),
        _ => return None,
    };

    if fields.len() < 2 {
        return None;
    }

    let id = match &fields[0] {
        Value::I32(i) => *i,
        _ => return None,
    };

    let props: HashMap<String, OwnedValue> = match &fields[1] {
        Value::Dict(d) => {
            let mut map = HashMap::new();
            for (k, v) in d.iter() {
                let key = match k {
                    Value::Str(s) => s.to_string(),
                    _ => continue,
                };
                // Values in `{sv}` arrive wrapped in an extra Value::Value variant
                // layer.  Peel it so prop_str / prop_bool can match the inner type.
                let inner = match v {
                    Value::Value(boxed) => &**boxed,
                    other => other,
                };
                if let Ok(owned) = OwnedValue::try_from(inner.clone()) {
                    map.insert(key, owned);
                }
            }
            map
        }
        _ => HashMap::new(),
    };

    let label = prop_str(&props, "label").map(str::to_owned);
    let enabled = prop_bool(&props, "enabled", true);
    let visible = prop_bool(&props, "visible", true);
    let is_separator = prop_str(&props, "type")
        .map(|t| t == "separator")
        .unwrap_or(false);
    let children_display = prop_str(&props, "children-display").map(str::to_owned);

    Some(MenuNode {
        id,
        label,
        enabled,
        visible,
        is_separator,
        children_display,
    })
}

fn prop_str<'a>(props: &'a HashMap<String, OwnedValue>, key: &str) -> Option<&'a str> {
    use zbus::zvariant::Value;
    props.get(key).and_then(|v| match &**v {
        Value::Str(s) => Some(s.as_str()),
        _ => None,
    })
}

fn prop_bool(props: &HashMap<String, OwnedValue>, key: &str, default: bool) -> bool {
    use zbus::zvariant::Value;
    props
        .get(key)
        .and_then(|v| match &**v {
            Value::Bool(b) => Some(*b),
            _ => None,
        })
        .unwrap_or(default)
}
