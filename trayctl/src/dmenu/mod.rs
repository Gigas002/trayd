//! Submenu loop: IPC `get_menu` → stdin lines → dmenu stdout → `activate`.

use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::error::CtlError;
use crate::ipc::{IpcClient, MenuItem};

/// Drive the dmenu submenu loop for `app_id`.
///
/// At each level the visible menu items are fed to `dmenu_cmd` via stdin; the
/// selected line is read back from stdout.  If the user selects a submenu
/// folder the loop descends; otherwise `activate` is called and the loop ends.
pub async fn run_submenu_loop(
    client: &mut IpcClient,
    app_id: &str,
    dmenu_cmd: &str,
) -> Result<(), CtlError> {
    let mut submenu_id: Option<u32> = None;

    loop {
        let items = client.get_menu(app_id, submenu_id).await?;

        // Separator / invisible items have empty labels — skip them.
        let visible: Vec<&MenuItem> = items.iter().filter(|i| !i.label.is_empty()).collect();
        if visible.is_empty() {
            return Ok(());
        }

        let labels: Vec<String> = visible.iter().map(|i| i.label.clone()).collect();

        let selection = match spawn_dmenu(dmenu_cmd, &labels).await? {
            None => return Ok(()), // user canceled (Esc / empty output)
            Some(s) => s,
        };

        let selected = match find_selected(&items, &selection) {
            None => return Ok(()), // selection didn't match any known item
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

/// Spawn `cmd_str` as a dmenu-compatible process, write `labels` to its stdin
/// (one per line), and return the trimmed selected line.
///
/// Returns `Ok(None)` when the user cancels (empty or no output).
pub(crate) async fn spawn_dmenu(
    cmd_str: &str,
    labels: &[String],
) -> Result<Option<String>, CtlError> {
    let mut parts = cmd_str.split_whitespace();
    let program = parts
        .next()
        .ok_or_else(|| CtlError::InvalidDmenuCmd(cmd_str.to_owned()))?;
    let args: Vec<&str> = parts.collect();

    let mut child = Command::new(program)
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| CtlError::DmenuSpawn(format!("{program}: {e}")))?;

    // Write all labels then drop stdin to signal EOF.
    let mut stdin = child.stdin.take().expect("stdin was piped");
    let input = labels.join("\n") + "\n";
    stdin.write_all(input.as_bytes()).await?;
    drop(stdin);

    // Read back exactly one line (the selection).
    let stdout = child.stdout.take().expect("stdout was piped");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).await?;

    child.wait().await?;

    let trimmed = line.trim().to_owned();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed))
    }
}

/// Return the first item in `items` whose label equals `label`.
pub(crate) fn find_selected<'a>(items: &'a [MenuItem], label: &str) -> Option<&'a MenuItem> {
    items.iter().find(|i| i.label == label)
}

#[cfg(test)]
mod tests;
