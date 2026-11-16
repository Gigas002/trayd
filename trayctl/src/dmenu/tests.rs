use super::{find_selected, menu_lines, spawn_dmenu};
use crate::ipc::MenuItem;

fn make_item(item_id: u32, label: &str, is_submenu: bool) -> MenuItem {
    MenuItem {
        item_id,
        label: label.to_owned(),
        is_submenu,
        enabled: true,
        separator: false,
        checked: None,
    }
}

fn lines(items: &[MenuItem]) -> Vec<String> {
    menu_lines(items).into_iter().map(|(l, _)| l).collect()
}

#[test]
fn separators_disabled_and_blank_rows_are_not_offered() {
    let mut disabled = make_item(2, "Greyed", false);
    disabled.enabled = false;
    let mut separator = make_item(3, "", false);
    separator.separator = true;
    let items = [
        make_item(1, "Open", false),
        disabled,
        separator,
        make_item(4, "   ", false),
    ];
    assert_eq!(lines(&items), ["Open"]);
}

#[test]
fn submenus_and_toggles_are_marked() {
    let mut on = make_item(2, "Mute", false);
    on.checked = Some(true);
    let mut off = make_item(3, "Shuffle", false);
    off.checked = Some(false);
    let items = [make_item(1, "More", true), on, off];
    assert_eq!(lines(&items), ["More ›", "[x] Mute", "[ ] Shuffle"]);
}

#[test]
fn control_characters_never_reach_the_menu() {
    let items = [make_item(1, "Evil\x1b[31m\nline", false)];
    assert_eq!(lines(&items), ["Evil [31m line"]);
}

#[test]
fn duplicate_labels_select_the_row_that_was_picked() {
    let items = [
        make_item(10, "Settings", false),
        make_item(20, "Settings", false),
    ];
    let entries = menu_lines(&items);
    assert_eq!(entries[1].0, "Settings (2)");
    assert_eq!(find_selected(&entries, "Settings").unwrap().item_id, 10);
    assert_eq!(find_selected(&entries, "Settings (2)").unwrap().item_id, 20);
}

#[test]
fn unknown_selection_matches_nothing() {
    let items = [make_item(1, "Copy", false)];
    assert!(find_selected(&menu_lines(&items), "Delete").is_none());
}

#[tokio::test]
async fn dmenu_command_runs_through_the_shell() {
    // Quoting works, and the picked line comes back trimmed.
    let picked = spawn_dmenu("sed -n '2p'", &["a".into(), "b c".into()])
        .await
        .unwrap();
    assert_eq!(picked.as_deref(), Some("b c"));
    // Empty output means the user cancelled.
    assert_eq!(
        spawn_dmenu("cat >/dev/null", &["a".into()]).await.unwrap(),
        None
    );
}

#[tokio::test]
async fn missing_dmenu_command_is_an_error() {
    assert!(
        spawn_dmenu("definitely-not-a-dmenu-xyz", &["a".into()])
            .await
            .is_err()
    );
    assert!(spawn_dmenu("   ", &["a".into()]).await.is_err());
}
