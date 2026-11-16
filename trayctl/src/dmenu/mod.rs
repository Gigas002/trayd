//! Submenu loop: IPC `get_menu` → stdin lines → dmenu stdout → `activate`.

use std::collections::HashMap;
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::error::CtlError;
use crate::ipc::{IpcClient, MenuItem};

/// Drive the dmenu submenu loop for `app_id`.
///
/// At each level the selectable menu rows are fed to `dmenu_cmd` via stdin;
/// the selected line is read back from stdout.  If the user selects a
/// submenu the loop descends; otherwise `activate` is called and the loop
/// ends.
pub async fn run_submenu_loop(
    client: &mut IpcClient,
    app_id: &str,
    dmenu_cmd: &str,
) -> Result<(), CtlError> {
    let mut submenu_id: Option<u32> = None;

    loop {
        let items = client.get_menu(app_id, submenu_id).await?;
        let entries = menu_lines(&items);
        if entries.is_empty() {
            return Ok(());
        }
        let lines: Vec<String> = entries.iter().map(|(line, _)| line.clone()).collect();

        let selection = match spawn_dmenu(dmenu_cmd, &lines).await? {
            None => return Ok(()), // user canceled (Esc / empty output)
            Some(s) => s,
        };

        let selected = match find_selected(&entries, &selection) {
            None => return Ok(()), // typed text that matches no row
            Some(item) => item,
        };

        if selected.is_submenu {
            submenu_id = Some(selected.item_id);
        } else {
            client.activate(app_id, selected.item_id).await?;
            return Ok(());
        }
    }
}

/// The lines to show for `items`, each with the row it stands for.
///
/// Separators and greyed-out rows are left out.  Labels are cleaned of
/// control characters (they come from other programs and would otherwise
/// reach terminal-based menus), toggles get a `[x]`/`[ ]` prefix and
/// submenus a `›` suffix, and repeated labels are numbered so every line
/// maps back to exactly one row.
pub(crate) fn menu_lines(items: &[MenuItem]) -> Vec<(String, &MenuItem)> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    items
        .iter()
        .filter(|i| i.enabled && !i.separator && !i.label.trim().is_empty())
        .map(|item| {
            let label: String = item
                .label
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect();
            let mut line = match item.checked {
                Some(true) => format!("[x] {}", label.trim()),
                Some(false) => format!("[ ] {}", label.trim()),
                None => label.trim().to_owned(),
            };
            if item.is_submenu {
                line.push_str(" ›");
            }
            let count = seen.entry(line.clone()).or_insert(0);
            *count += 1;
            if *count > 1 {
                line = format!("{line} ({count})");
            }
            (line, item)
        })
        .collect()
}

/// Spawn `cmd_str` through `sh -c` as a dmenu-compatible process, write
/// `lines` to its stdin (one per line), and return the trimmed selected line.
///
/// Going through the shell lets the command use quoting, e.g.
/// `rofi -dmenu -p 'Tray menu'`; the menu text only ever travels over stdin.
///
/// Returns `Ok(None)` when the user cancels (empty or no output).
pub(crate) async fn spawn_dmenu(
    cmd_str: &str,
    lines: &[String],
) -> Result<Option<String>, CtlError> {
    if cmd_str.trim().is_empty() {
        return Err(CtlError::InvalidDmenuCmd(cmd_str.to_owned()));
    }

    let mut child = Command::new("sh")
        .arg("-c")
        .arg(cmd_str)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| CtlError::DmenuSpawn(format!("sh: {e}")))?;

    // Write all lines then drop stdin to signal EOF.  A menu that exits
    // without reading everything closes the pipe; that is not an error.
    let mut stdin = child.stdin.take().expect("stdin was piped");
    let input = lines.join("\n") + "\n";
    if let Err(e) = stdin.write_all(input.as_bytes()).await
        && e.kind() != std::io::ErrorKind::BrokenPipe
    {
        return Err(e.into());
    }
    drop(stdin);

    // Read back exactly one line (the selection).
    let stdout = child.stdout.take().expect("stdout was piped");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).await?;

    let status = child.wait().await?;
    // 126/127: the shell could not run the command at all.
    if matches!(status.code(), Some(126 | 127)) {
        return Err(CtlError::DmenuSpawn(format!(
            "{cmd_str}: command not found or not executable"
        )));
    }

    let trimmed = line.trim_end_matches(['\n', '\r']).trim().to_owned();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed))
    }
}

/// The row whose line is exactly `selection`.
pub(crate) fn find_selected<'a>(
    entries: &[(String, &'a MenuItem)],
    selection: &str,
) -> Option<&'a MenuItem> {
    entries
        .iter()
        .find(|(line, _)| line == selection)
        .map(|(_, item)| *item)
}

#[cfg(test)]
mod tests;
