//! ratatui terminal UI for trayd.

use std::path::{Path, PathBuf};
use std::time::Duration;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use tokio::sync::mpsc;

use crate::error::TuiError;
use crate::ipc::{IpcClient, MenuItem, MinimalTrayItem, TrayEvent};

// ---------------------------------------------------------------------------
// View state
// ---------------------------------------------------------------------------

pub struct MenuLevel {
    #[allow(dead_code)]
    pub submenu_id: Option<u32>,
    /// Label of the submenu row that opened this level (`None` at the top).
    pub label: Option<String>,
    pub items: Vec<MenuItem>,
    pub cursor: usize,
}

impl MenuLevel {
    fn new(submenu_id: Option<u32>, label: Option<String>, items: Vec<MenuItem>) -> Self {
        let cursor = items.iter().position(|i| !i.separator).unwrap_or(0);
        Self {
            submenu_id,
            label,
            items,
            cursor,
        }
    }

    /// Move to the next non-separator row in direction `step` (±1), if any.
    fn step(&mut self, step: isize) {
        let mut at = self.cursor;
        loop {
            match at.checked_add_signed(step) {
                Some(next) if next < self.items.len() => at = next,
                _ => return,
            }
            if !self.items[at].separator {
                self.cursor = at;
                return;
            }
        }
    }
}

pub enum View {
    Items,
    Menu {
        app_id: String,
        stack: Vec<MenuLevel>,
    },
}

/// What the subscription task reports to the UI.
#[derive(Debug)]
pub enum Update {
    /// A fresh snapshot of the tray.
    Items(Vec<MinimalTrayItem>),
    /// The daemon went away; the task keeps retrying.
    Disconnected(String),
}

#[derive(Debug)]
enum Action {
    OpenMenu(String),
    OpenSubmenu {
        app_id: String,
        item_id: u32,
        label: String,
    },
    PrimaryActivate(String),
    Activate {
        app_id: String,
        item_id: u32,
    },
    /// A greyed-out row: say so instead of activating it.
    Disabled(String),
    Nothing,
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

pub struct App {
    socket_path: PathBuf,
    tray_items: Vec<MinimalTrayItem>,
    items_cursor: usize,
    view: View,
    status_msg: Option<String>,
    /// Why the daemon is unreachable, while it is.
    disconnected: Option<String>,
}

impl App {
    fn new(socket_path: PathBuf, initial_items: Vec<MinimalTrayItem>) -> Self {
        Self {
            socket_path,
            tray_items: initial_items,
            items_cursor: 0,
            view: View::Items,
            status_msg: None,
            disconnected: None,
        }
    }

    /// Acquire the terminal, run the event loop, then restore terminal on exit.
    pub async fn run(socket_path: PathBuf) -> Result<(), TuiError> {
        let mut cmd = IpcClient::connect(&socket_path).await?;
        let initial_items = cmd.get_items().await?;
        drop(cmd);

        let (update_tx, update_rx) = mpsc::channel::<Update>(32);
        tokio::spawn(subscribe_task(socket_path.clone(), update_tx));

        let (input_tx, input_rx) = mpsc::channel::<Event>(32);
        tokio::task::spawn_blocking(move || input_task(input_tx));

        // Log lines written to the terminal would draw over the UI.
        crate::logger::pause();
        // Also installs a panic hook that restores the terminal.
        let mut terminal = ratatui::init();

        let mut app = App::new(socket_path, initial_items);
        let result = app.event_loop(&mut terminal, update_rx, input_rx).await;

        ratatui::restore();
        crate::logger::resume();
        result
    }

    async fn event_loop(
        &mut self,
        terminal: &mut DefaultTerminal,
        mut update_rx: mpsc::Receiver<Update>,
        mut input_rx: mpsc::Receiver<Event>,
    ) -> Result<(), TuiError> {
        loop {
            terminal.draw(|f| self.render(f))?;

            tokio::select! {
                event = input_rx.recv() => {
                    let quit = match event {
                        Some(Event::Key(key)) => !self.handle_key(key).await?,
                        None => true,
                        _ => false,
                    };
                    if quit {
                        break;
                    }
                }
                update = update_rx.recv() => {
                    match update {
                        Some(update) => self.apply_update(update),
                        None => break,
                    }
                }
            }
        }
        Ok(())
    }

    /// Take in news from the daemon.  The cursor stays on the same tray item
    /// even when items come or go before it, and an open menu closes when its
    /// item leaves the tray.
    fn apply_update(&mut self, update: Update) {
        let items = match update {
            Update::Disconnected(reason) => {
                self.disconnected = Some(reason);
                return;
            }
            Update::Items(items) => items,
        };
        self.disconnected = None;
        let selected = self
            .tray_items
            .get(self.items_cursor)
            .map(|i| i.app_id.clone());
        self.tray_items = items;
        self.items_cursor = selected
            .and_then(|id| self.tray_items.iter().position(|i| i.app_id == id))
            .unwrap_or_else(|| {
                self.items_cursor
                    .min(self.tray_items.len().saturating_sub(1))
            });

        if let View::Menu { app_id, .. } = &self.view
            && !self.tray_items.iter().any(|i| &i.app_id == app_id)
        {
            self.status_msg = Some(format!("{app_id} left the tray"));
            self.view = View::Items;
        }
    }

    async fn handle_key(&mut self, key: KeyEvent) -> Result<bool, TuiError> {
        self.status_msg = None;

        match key.code {
            KeyCode::Char('q') => return Ok(false),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(false);
            }
            KeyCode::Esc => self.go_back(),
            KeyCode::Up | KeyCode::Char('k') => self.move_up(),
            KeyCode::Down | KeyCode::Char('j') => self.move_down(),
            KeyCode::Enter | KeyCode::Char(' ') => {
                if let Err(e) = self.select().await {
                    self.status_msg = Some(format!("Error: {e}"));
                }
            }
            // `m` always opens the menu for the selected tray item, regardless of
            // whether `item_is_menu` is set.  Useful for items whose primary action
            // is activation but that also expose a context menu.
            KeyCode::Char('m') => {
                if matches!(self.view, View::Items)
                    && let Some(item) = self.tray_items.get(self.items_cursor)
                {
                    let action = Action::OpenMenu(item.app_id.clone());
                    if let Err(e) = self.execute_action(action).await {
                        self.status_msg = Some(format!("Error: {e}"));
                    }
                }
            }
            _ => {}
        }
        Ok(true)
    }

    fn compute_action(&self) -> Action {
        match &self.view {
            View::Items => match self.tray_items.get(self.items_cursor) {
                // Menu-only items go straight to the menu; regular items activate.
                Some(item) if item.item_is_menu => Action::OpenMenu(item.app_id.clone()),
                Some(item) => Action::PrimaryActivate(item.app_id.clone()),
                None => Action::Nothing,
            },
            View::Menu { app_id, stack } => {
                let level = match stack.last() {
                    Some(l) => l,
                    None => return Action::Nothing,
                };
                let item = match level.items.get(level.cursor) {
                    Some(i) => i,
                    None => return Action::Nothing,
                };
                if item.separator {
                    Action::Nothing
                } else if !item.enabled {
                    Action::Disabled(item.label.clone())
                } else if item.is_submenu {
                    Action::OpenSubmenu {
                        app_id: app_id.clone(),
                        item_id: item.item_id,
                        label: item.label.clone(),
                    }
                } else {
                    Action::Activate {
                        app_id: app_id.clone(),
                        item_id: item.item_id,
                    }
                }
            }
        }
    }

    async fn select(&mut self) -> Result<(), TuiError> {
        let action = self.compute_action();
        self.execute_action(action).await
    }

    async fn execute_action(&mut self, action: Action) -> Result<(), TuiError> {
        match action {
            Action::Nothing => {}
            Action::Disabled(label) => {
                self.status_msg = Some(format!("\"{label}\" is disabled"));
            }
            Action::PrimaryActivate(app_id) => {
                let mut client = IpcClient::connect(&self.socket_path).await?;
                client.activate(&app_id, 0).await?;
            }
            Action::OpenMenu(app_id) => {
                let mut client = IpcClient::connect(&self.socket_path).await?;
                let items = client.get_menu(&app_id, None).await?;
                self.view = View::Menu {
                    app_id,
                    stack: vec![MenuLevel::new(None, None, items)],
                };
            }
            Action::OpenSubmenu {
                app_id,
                item_id,
                label,
            } => {
                let mut client = IpcClient::connect(&self.socket_path).await?;
                let items = client.get_menu(&app_id, Some(item_id)).await?;
                if let View::Menu { stack, .. } = &mut self.view {
                    stack.push(MenuLevel::new(Some(item_id), Some(label), items));
                }
            }
            Action::Activate { app_id, item_id } => {
                let mut client = IpcClient::connect(&self.socket_path).await?;
                client.activate(&app_id, item_id).await?;
                self.view = View::Items;
            }
        }
        Ok(())
    }

    pub fn move_up(&mut self) {
        match &mut self.view {
            View::Items => {
                self.items_cursor = self.items_cursor.saturating_sub(1);
            }
            View::Menu { stack, .. } => {
                if let Some(level) = stack.last_mut() {
                    level.step(-1);
                }
            }
        }
    }

    pub fn move_down(&mut self) {
        match &mut self.view {
            View::Items => {
                if self.items_cursor + 1 < self.tray_items.len() {
                    self.items_cursor += 1;
                }
            }
            View::Menu { stack, .. } => {
                if let Some(level) = stack.last_mut() {
                    level.step(1);
                }
            }
        }
    }

    pub fn go_back(&mut self) {
        let should_switch = match &mut self.view {
            View::Items => return,
            View::Menu { stack, .. } => {
                stack.pop();
                stack.is_empty()
            }
        };
        if should_switch {
            self.view = View::Items;
        }
    }

    fn render(&self, f: &mut ratatui::Frame) {
        let area = f.area();
        let [content, status_bar] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(area);

        match &self.view {
            View::Items => self.render_items(f, content),
            View::Menu { .. } => self.render_menu(f, content),
        }

        self.render_status(f, status_bar);
    }

    fn render_items(&self, f: &mut ratatui::Frame, area: Rect) {
        let list_items: Vec<ListItem> = self
            .tray_items
            .iter()
            .map(|item| {
                let name = item.title.as_deref().unwrap_or(item.app_id.as_str());
                // [⊞] badge signals that Enter opens the menu rather than activating.
                let menu_tag = if item.item_is_menu { "  [⊞]" } else { "" };
                let mut lines = vec![Line::from(format!("{name}{menu_tag}  [{}]", item.status))];
                // Show tooltip as a dim second line when available.
                let tip = item
                    .tooltip_title
                    .as_deref()
                    .or(item.tooltip_description.as_deref());
                if let Some(tip) = tip {
                    lines.push(Line::from(Span::styled(
                        format!("  {tip}"),
                        Style::default().fg(Color::DarkGray),
                    )));
                }
                ListItem::new(Text::from(lines))
            })
            .collect();

        let list = List::new(list_items)
            .block(
                Block::default()
                    .title(" System Tray ")
                    .borders(Borders::ALL),
            )
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol("> ");

        let selected = if self.tray_items.is_empty() {
            None
        } else {
            Some(self.items_cursor)
        };
        let mut state = ListState::default().with_selected(selected);
        f.render_stateful_widget(list, area, &mut state);
    }

    fn render_menu(&self, f: &mut ratatui::Frame, area: Rect) {
        let (app_id, stack) = match &self.view {
            View::Menu { app_id, stack } => (app_id, stack),
            View::Items => return,
        };
        let level = match stack.last() {
            Some(l) => l,
            None => return,
        };

        // "Title › Submenu › Inner", named like the items list names the item.
        let name = self
            .tray_items
            .iter()
            .find(|i| &i.app_id == app_id)
            .and_then(|i| i.title.as_deref())
            .unwrap_or(app_id);
        let mut title = format!(" {name}");
        for label in stack.iter().filter_map(|l| l.label.as_deref()) {
            title.push_str(" › ");
            title.push_str(label);
        }
        title.push(' ');

        let width = usize::from(area.width.saturating_sub(4));
        let list_items: Vec<ListItem> = level
            .items
            .iter()
            .map(|item| {
                if item.separator {
                    return ListItem::new("─".repeat(width))
                        .style(Style::default().fg(Color::DarkGray));
                }
                let check = match item.checked {
                    Some(true) => "[x] ",
                    Some(false) => "[ ] ",
                    None => "",
                };
                let suffix = if item.is_submenu { " ▶" } else { "" };
                let mut spans = vec![Span::raw(format!("{check}{}{suffix}", item.label))];
                if let Some(shortcut) = &item.shortcut {
                    spans.push(Span::styled(
                        format!("  {shortcut}"),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                let row = ListItem::new(Line::from(spans));
                if item.enabled {
                    row
                } else {
                    row.style(Style::default().fg(Color::DarkGray))
                }
            })
            .collect();

        let list = List::new(list_items)
            .block(Block::default().title(title).borders(Borders::ALL))
            .highlight_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ");

        let selected = if level.items.is_empty() {
            None
        } else {
            Some(level.cursor)
        };
        let mut state = ListState::default().with_selected(selected);
        f.render_stateful_widget(list, area, &mut state);
    }

    fn render_status(&self, f: &mut ratatui::Frame, area: Rect) {
        let (text, style) = if let Some(msg) = &self.status_msg {
            (format!(" {msg}"), Style::default().fg(Color::Red))
        } else if let Some(reason) = &self.disconnected {
            (
                format!(" trayd unreachable, retrying… ({reason})"),
                Style::default().fg(Color::Yellow),
            )
        } else {
            let hint = match &self.view {
                View::Items => " j/k: navigate  Enter: activate/menu  m: open menu  q: quit",
                View::Menu { .. } => " j/k: navigate  Enter: select  Esc: back  q: quit",
            };
            (hint.to_owned(), Style::default().fg(Color::DarkGray))
        };

        f.render_widget(Paragraph::new(text).style(style), area);
    }
}

// ---------------------------------------------------------------------------
// Background tasks
// ---------------------------------------------------------------------------

async fn subscribe_task(socket_path: PathBuf, tx: mpsc::Sender<Update>) {
    loop {
        match run_subscribe(&socket_path, &tx).await {
            Ok(()) => break,
            Err(e) => {
                tracing::debug!(%e, "subscribe disconnected, retrying in 1s");
                if tx.send(Update::Disconnected(e.to_string())).await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
                if tx.is_closed() {
                    break;
                }
            }
        }
    }
}

async fn run_subscribe(socket_path: &Path, tx: &mpsc::Sender<Update>) -> Result<(), TuiError> {
    let mut client = IpcClient::connect(socket_path).await?;
    client.send_subscribe().await?;
    loop {
        match client.recv_event().await? {
            TrayEvent::Update(items) => {
                if tx.send(Update::Items(items)).await.is_err() {
                    return Ok(());
                }
            }
        }
    }
}

fn input_task(tx: mpsc::Sender<Event>) {
    loop {
        match event::poll(Duration::from_millis(100)) {
            Ok(true) => match event::read() {
                Ok(event) => {
                    if tx.blocking_send(event).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    tracing::error!(%e, "crossterm read failed");
                    break;
                }
            },
            Ok(false) => {
                if tx.is_closed() {
                    break;
                }
            }
            Err(e) => {
                tracing::error!(%e, "crossterm poll failed");
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests;
