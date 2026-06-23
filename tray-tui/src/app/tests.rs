use super::{App, MenuLevel, View};
use crate::ipc::{MenuItem, MinimalTrayItem};

fn make_tray_item(app_id: &str) -> MinimalTrayItem {
    MinimalTrayItem {
        app_id: app_id.to_owned(),
        title: None,
        status: "Active".to_owned(),
        icon_handle: None,
        category: None,
        item_is_menu: false,
        tooltip_title: None,
        tooltip_description: None,
    }
}

fn make_menu_item(id: u32, label: &str, is_submenu: bool) -> MenuItem {
    MenuItem {
        item_id: id,
        label: label.to_owned(),
        is_submenu,
    }
}

fn items_app(items: Vec<MinimalTrayItem>) -> App {
    App::new("".into(), items)
}

// ---------------------------------------------------------------------------
// Items view navigation
// ---------------------------------------------------------------------------

#[test]
fn move_down_advances_cursor() {
    let mut app = items_app(vec![make_tray_item("a"), make_tray_item("b")]);
    app.move_down();
    assert_eq!(app.items_cursor, 1);
}

#[test]
fn move_down_clamps_at_last() {
    let mut app = items_app(vec![make_tray_item("a"), make_tray_item("b")]);
    app.items_cursor = 1;
    app.move_down();
    assert_eq!(app.items_cursor, 1);
}

#[test]
fn move_up_decrements_cursor() {
    let mut app = items_app(vec![make_tray_item("a"), make_tray_item("b")]);
    app.items_cursor = 1;
    app.move_up();
    assert_eq!(app.items_cursor, 0);
}

#[test]
fn move_up_clamps_at_zero() {
    let mut app = items_app(vec![make_tray_item("a")]);
    app.move_up();
    assert_eq!(app.items_cursor, 0);
}

#[test]
fn move_down_on_empty_list_does_not_panic() {
    let mut app = items_app(vec![]);
    app.move_down();
    assert_eq!(app.items_cursor, 0);
}

// ---------------------------------------------------------------------------
// go_back
// ---------------------------------------------------------------------------

#[test]
fn go_back_from_items_is_noop() {
    let mut app = items_app(vec![]);
    app.go_back();
    assert!(matches!(app.view, View::Items));
}

#[test]
fn go_back_from_single_menu_level_returns_to_items() {
    let mut app = items_app(vec![]);
    app.view = View::Menu {
        app_id: "org.nm".to_owned(),
        stack: vec![MenuLevel {
            submenu_id: None,
            items: vec![],
            cursor: 0,
        }],
    };
    app.go_back();
    assert!(matches!(app.view, View::Items));
}

#[test]
fn go_back_from_two_menu_levels_pops_one() {
    let mut app = items_app(vec![]);
    app.view = View::Menu {
        app_id: "org.nm".to_owned(),
        stack: vec![
            MenuLevel {
                submenu_id: None,
                items: vec![],
                cursor: 0,
            },
            MenuLevel {
                submenu_id: Some(1),
                items: vec![],
                cursor: 0,
            },
        ],
    };
    app.go_back();
    match &app.view {
        View::Menu { stack, .. } => assert_eq!(stack.len(), 1),
        View::Items => panic!("expected menu view after go_back from depth 2"),
    }
}

// ---------------------------------------------------------------------------
// Menu view navigation
// ---------------------------------------------------------------------------

#[test]
fn move_down_in_menu_advances_cursor() {
    let mut app = items_app(vec![]);
    app.view = View::Menu {
        app_id: "org.nm".to_owned(),
        stack: vec![MenuLevel {
            submenu_id: None,
            items: vec![make_menu_item(1, "A", false), make_menu_item(2, "B", false)],
            cursor: 0,
        }],
    };
    app.move_down();
    match &app.view {
        View::Menu { stack, .. } => assert_eq!(stack[0].cursor, 1),
        View::Items => panic!("expected menu view"),
    }
}

#[test]
fn move_down_in_menu_clamps_at_last() {
    let mut app = items_app(vec![]);
    app.view = View::Menu {
        app_id: "org.nm".to_owned(),
        stack: vec![MenuLevel {
            submenu_id: None,
            items: vec![make_menu_item(1, "A", false)],
            cursor: 0,
        }],
    };
    app.move_down();
    match &app.view {
        View::Menu { stack, .. } => assert_eq!(stack[0].cursor, 0),
        View::Items => panic!("expected menu view"),
    }
}

#[test]
fn move_up_in_menu_decrements_cursor() {
    let mut app = items_app(vec![]);
    app.view = View::Menu {
        app_id: "org.nm".to_owned(),
        stack: vec![MenuLevel {
            submenu_id: None,
            items: vec![make_menu_item(1, "A", false), make_menu_item(2, "B", false)],
            cursor: 1,
        }],
    };
    app.move_up();
    match &app.view {
        View::Menu { stack, .. } => assert_eq!(stack[0].cursor, 0),
        View::Items => panic!("expected menu view"),
    }
}

// ---------------------------------------------------------------------------
// compute_action
// ---------------------------------------------------------------------------

#[test]
fn compute_action_nothing_on_empty_items() {
    let app = items_app(vec![]);
    assert!(matches!(app.compute_action(), super::Action::Nothing));
}

#[test]
fn compute_action_primary_activate_for_item() {
    // A regular (non-menu-only) item should fire PrimaryActivate on Enter.
    let app = items_app(vec![make_tray_item("org.nm")]);
    match app.compute_action() {
        super::Action::PrimaryActivate(id) => assert_eq!(id, "org.nm"),
        other => panic!("expected PrimaryActivate, got {other:?}"),
    }
}

#[test]
fn compute_action_open_menu_for_menu_only_item() {
    // A menu-only item should open the menu on Enter.
    let mut item = make_tray_item("org.menuapp");
    item.item_is_menu = true;
    let app = items_app(vec![item]);
    match app.compute_action() {
        super::Action::OpenMenu(id) => assert_eq!(id, "org.menuapp"),
        other => panic!("expected OpenMenu, got {other:?}"),
    }
}

#[test]
fn compute_action_activate_for_leaf_menu_item() {
    let mut app = items_app(vec![]);
    app.view = View::Menu {
        app_id: "org.nm".to_owned(),
        stack: vec![MenuLevel {
            submenu_id: None,
            items: vec![make_menu_item(5, "Enable", false)],
            cursor: 0,
        }],
    };
    match app.compute_action() {
        super::Action::Activate { app_id, item_id } => {
            assert_eq!(app_id, "org.nm");
            assert_eq!(item_id, 5);
        }
        other => panic!("expected Activate, got {other:?}"),
    }
}

#[test]
fn compute_action_open_submenu_for_submenu_item() {
    let mut app = items_app(vec![]);
    app.view = View::Menu {
        app_id: "org.nm".to_owned(),
        stack: vec![MenuLevel {
            submenu_id: None,
            items: vec![make_menu_item(3, "Wi-Fi", true)],
            cursor: 0,
        }],
    };
    match app.compute_action() {
        super::Action::OpenSubmenu { app_id, item_id } => {
            assert_eq!(app_id, "org.nm");
            assert_eq!(item_id, 3);
        }
        other => panic!("expected OpenSubmenu, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// items cursor clamping on tray update
// ---------------------------------------------------------------------------

#[test]
fn items_cursor_clamped_when_list_shrinks() {
    let mut app = items_app(vec![
        make_tray_item("a"),
        make_tray_item("b"),
        make_tray_item("c"),
    ]);
    app.items_cursor = 2;
    // Simulate update_rx receiving fewer items (same logic as event_loop)
    let new_items = vec![make_tray_item("a")];
    app.tray_items = new_items;
    if app.items_cursor >= app.tray_items.len() {
        app.items_cursor = app.tray_items.len().saturating_sub(1);
    }
    assert_eq!(app.items_cursor, 0);
}
