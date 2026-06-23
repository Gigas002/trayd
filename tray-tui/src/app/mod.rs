//! ratatui terminal UI for trayd.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
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
    pub items: Vec<MenuItem>,
    pub cursor: usize,
}

pub enum View {
    Items,
    Menu {
        app_id: String,
        stack: Vec<MenuLevel>,
    },
}

#[derive(Debug)]
enum Action {
    OpenMenu(String),
    OpenSubmenu { app_id: String, item_id: u32 },
    PrimaryActivate(String),
    Activate { app_id: String, item_id: u32 },
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
}

impl App {
    fn new(socket_path: PathBuf, initial_items: Vec<MinimalTrayItem>) -> Self {
        Self {
            socket_path,
            tray_items: initial_items,
            items_cursor: 0,
            view: View::Items,
            status_msg: None,
        }
    }

    /// Acquire the terminal, run the event loop, then restore terminal on exit.
    pub async fn run(socket_path: PathBuf) -> Result<(), TuiError> {
        let mut cmd = IpcClient::connect(&socket_path).await?;
        let initial_items = cmd.get_items().await?;
        drop(cmd);

        let (update_tx, update_rx) = mpsc::channel::<Vec<MinimalTrayItem>>(32);
        tokio::spawn(subscribe_task(socket_path.clone(), update_tx));

        let (input_tx, input_rx) = mpsc::channel::<Event>(32);
        tokio::task::spawn_blocking(move || input_task(input_tx));

        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let mut app = App::new(socket_path, initial_items);
        let result = app.event_loop(&mut terminal, update_rx, input_rx).await;

        let _ = disable_raw_mode();
        let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
        let _ = terminal.show_cursor();

        result
    }

    async fn event_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
        mut update_rx: mpsc::Receiver<Vec<MinimalTrayItem>>,
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
                        Some(items) => {
                            self.tray_items = items;
                            if self.items_cursor >= self.tray_items.len() {
                                self.items_cursor =
                                    self.tray_items.len().saturating_sub(1);
                            }
                        }
                        None => break,
                    }
                }
            }
        }
        Ok(())
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
                if item.is_submenu {
                    Action::OpenSubmenu {
                        app_id: app_id.clone(),
                        item_id: item.item_id,
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
            Action::PrimaryActivate(app_id) => {
                let mut client = IpcClient::connect(&self.socket_path).await?;
                client.activate(&app_id, 0).await?;
            }
            Action::OpenMenu(app_id) => {
                let mut client = IpcClient::connect(&self.socket_path).await?;
                let items = client.get_menu(&app_id, None).await?;
                self.view = View::Menu {
                    app_id,
                    stack: vec![MenuLevel {
                        submenu_id: None,
                        items,
                        cursor: 0,
                    }],
                };
            }
            Action::OpenSubmenu { app_id, item_id } => {
                let mut client = IpcClient::connect(&self.socket_path).await?;
                let items = client.get_menu(&app_id, Some(item_id)).await?;
                if let View::Menu { stack, .. } = &mut self.view {
                    stack.push(MenuLevel {
                        submenu_id: Some(item_id),
                        items,
                        cursor: 0,
                    });
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
                    level.cursor = level.cursor.saturating_sub(1);
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
                if let Some(level) = stack.last_mut()
                    && level.cursor + 1 < level.items.len()
                {
                    level.cursor += 1;
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

        let title = if stack.len() == 1 {
            format!(" {app_id} ")
        } else {
            format!(" {app_id} > (submenu) ")
        };

        let list_items: Vec<ListItem> = level
            .items
            .iter()
            .map(|item| {
                let label = if item.label.is_empty() {
                    "—"
                } else {
                    item.label.as_str()
                };
                let suffix = if item.is_submenu { " ▶" } else { "" };
                ListItem::new(format!("{label}{suffix}"))
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

async fn subscribe_task(socket_path: PathBuf, tx: mpsc::Sender<Vec<MinimalTrayItem>>) {
    loop {
        match run_subscribe(&socket_path, &tx).await {
            Ok(()) => break,
            Err(e) => {
                tracing::warn!(%e, "subscribe disconnected, retrying in 1s");
                tokio::time::sleep(Duration::from_secs(1)).await;
                if tx.is_closed() {
                    break;
                }
            }
        }
    }
}

async fn run_subscribe(
    socket_path: &Path,
    tx: &mpsc::Sender<Vec<MinimalTrayItem>>,
) -> Result<(), TuiError> {
    let mut client = IpcClient::connect(socket_path).await?;
    client.send_subscribe().await?;
    loop {
        match client.recv_event().await? {
            TrayEvent::Update(items) => {
                if tx.send(items).await.is_err() {
                    return Ok(());
                }
            }
        }
    }
}

fn input_task(tx: mpsc::Sender<Event>) {
    loop {
        match crossterm::event::poll(Duration::from_millis(100)) {
            Ok(true) => match crossterm::event::read() {
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
