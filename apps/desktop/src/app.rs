use std::collections::HashMap;
use std::sync::Arc;

use iced::widget::{column, container, row, scrollable, space, text, text_editor};
use iced::{clipboard, window, Alignment, Element, Font, Length, Size, Subscription, Task};
use mcp_core::{IssuedToken, ServerState, ServerStatus, ServerType, TokenRecord};
use mcp_platform::{AppPaths, NativeAppPaths, NativeBrowserOpener, SecretStore};
use mcp_runtime::McpConnector;

use crate::session::{parse_env, AddServerRequest, ImportOutcome, ServerToolView, Session};
use crate::shell::{on_close_requested, ShellAction};
use crate::ui;
#[cfg(target_os = "macos")]
use crate::shell::{on_tray, parse_tray_menu_id};

#[cfg(target_os = "macos")]
use mcp_platform::KeychainSecretStore;
#[cfg(not(target_os = "macos"))]
use mcp_platform::MemorySecretStore;

pub fn run() -> iced::Result {
    #[cfg(target_os = "macos")]
    crate::tray::install();

    iced::daemon(App::boot, App::update, App::view)
        .subscription(App::subscription)
        .title(App::title)
        .run()
}

fn open_main_window() -> Task<Message> {
    let (_id, open) = window::open(window_settings());
    open.map(Message::WindowOpened)
}

fn window_settings() -> window::Settings {
    window::Settings {
        size: Size::new(960.0, 680.0),
        exit_on_close_request: false,
        ..window::Settings::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FormServerType {
    Local,
    Remote,
    RemoteStreamable,
}

/// Content section shown in the main pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Section {
    Servers,
    Tokens,
}

pub(crate) const FORM_SERVER_TYPES: [FormServerType; 3] = [
    FormServerType::Local,
    FormServerType::Remote,
    FormServerType::RemoteStreamable,
];

impl std::fmt::Display for FormServerType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Local => "local stdio",
            Self::Remote => "remote SSE",
            Self::RemoteStreamable => "remote Streamable HTTP",
        })
    }
}

impl From<FormServerType> for ServerType {
    fn from(value: FormServerType) -> Self {
        match value {
            FormServerType::Local => Self::Local,
            FormServerType::Remote => Self::Remote,
            FormServerType::RemoteStreamable => Self::RemoteStreamable,
        }
    }
}

impl From<ServerType> for FormServerType {
    fn from(value: ServerType) -> Self {
        match value {
            ServerType::Local => Self::Local,
            ServerType::Remote => Self::Remote,
            ServerType::RemoteStreamable => Self::RemoteStreamable,
        }
    }
}

/// Raw add/edit form fields, borrowed from `App` (env text is owned because
/// the editor hands out an owned `String`).
#[derive(Debug, Clone)]
pub(crate) struct FormInput<'a> {
    pub name: &'a str,
    pub server_type: FormServerType,
    pub command: &'a str,
    pub args: &'a str,
    pub remote_url: &'a str,
    pub env_text: String,
    pub bearer: &'a str,
    pub auto_start: bool,
}

/// Build an `AddServerRequest` from the raw form fields.
pub(crate) fn add_request_from_form(input: FormInput<'_>) -> AddServerRequest {
    let local = input.server_type == FormServerType::Local;
    AddServerRequest {
        name: input.name.to_string(),
        server_type: input.server_type.into(),
        command: if local {
            Some(input.command.to_string())
        } else {
            None
        },
        args: if local {
            input.args.split_whitespace().map(str::to_string).collect()
        } else {
            Vec::new()
        },
        env: parse_env(&input.env_text),
        remote_url: if local {
            None
        } else {
            Some(input.remote_url.to_string())
        },
        auto_start: input.auto_start,
        bearer: if input.bearer.is_empty() {
            None
        } else {
            Some(input.bearer.to_string())
        },
    }
}

/// Prefilled form values for one server, produced by the EditServer task.
#[derive(Debug, Clone)]
pub(crate) struct EditFormState {
    pub id: String,
    pub name: String,
    pub server_type: FormServerType,
    pub command: String,
    pub args: String,
    pub remote_url: String,
    pub env_text: String,
    pub bearer: String,
    pub auto_start: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(server_type: FormServerType) -> FormInput<'static> {
        FormInput {
            name: "everything",
            server_type,
            command: "npx",
            args: "-y server-everything",
            remote_url: "https://example.com/mcp",
            env_text: "API_TOKEN=sk-1".to_string(),
            bearer: "",
            auto_start: true,
        }
    }

    #[test]
    fn add_request_from_form_local_omits_remote_fields() {
        let request = add_request_from_form(input(FormServerType::Local));
        assert_eq!(request.name, "everything");
        assert_eq!(request.server_type, ServerType::Local);
        assert_eq!(request.command.as_deref(), Some("npx"));
        assert_eq!(
            request.args,
            vec!["-y".to_string(), "server-everything".to_string()]
        );
        assert_eq!(request.remote_url, None);
        assert_eq!(
            request.env.get("API_TOKEN").map(String::as_str),
            Some("sk-1")
        );
        assert_eq!(request.bearer, None);
    }

    #[test]
    fn add_request_from_form_remote_omits_local_fields_and_blanks_bearer() {
        let request = add_request_from_form(input(FormServerType::RemoteStreamable));
        assert_eq!(request.server_type, ServerType::RemoteStreamable);
        assert_eq!(request.command, None);
        assert!(request.args.is_empty());
        assert_eq!(
            request.remote_url.as_deref(),
            Some("https://example.com/mcp")
        );
        assert_eq!(request.bearer, None);
    }
}

pub(crate) struct App {
    session: Session,
    window_id: Option<window::Id>,
    pub(crate) endpoint: String,
    pub(crate) section: Section,
    pub(crate) show_add_form: bool,
    pub(crate) editing_id: Option<String>,
    pub(crate) show_import_form: bool,
    pub(crate) import_json: text_editor::Content,
    pub(crate) notice: Option<String>,
    pub(crate) client_name: String,
    pub(crate) tokens: Vec<TokenRecord>,
    pub(crate) plaintext: Option<String>,
    pub(crate) servers: Vec<ServerState>,
    pub(crate) tools_by_server: HashMap<String, Vec<ServerToolView>>,
    pub(crate) oauth_by_server: HashMap<String, bool>,
    pub(crate) server_name: String,
    pub(crate) server_type: FormServerType,
    pub(crate) command: String,
    pub(crate) args: String,
    pub(crate) remote_url: String,
    pub(crate) env: text_editor::Content,
    pub(crate) bearer: String,
    pub(crate) auto_start: bool,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    WindowOpened(window::Id),
    CloseRequested,
    WindowClosed(window::Id),
    TrayTick,
    CopyEndpoint,
    CopyPlaintext,
    Navigate(Section),
    ToggleAddForm,
    CancelAddForm,
    EditServer(String),
    EditLoaded(Result<EditFormState, String>),
    UpdateServer,
    ImportText(text_editor::Action),
    ToggleImportForm,
    CancelImportForm,
    ImportJson,
    ImportDone(Result<ImportOutcome, String>),
    ClientName(String),
    IssueToken,
    TokenIssued(Result<IssuedToken, String>),
    Revoke(String),
    ServerName(String),
    ServerType(FormServerType),
    Command(String),
    Args(String),
    RemoteUrl(String),
    Env(text_editor::Action),
    Bearer(String),
    AutoStart(bool),
    AddServer,
    Start(String),
    Stop(String),
    Delete(String),
    ToggleTool {
        id: String,
        name: String,
        public: bool,
    },
    Oauth(String),
    Snapshot(Result<Snapshot, String>),
    OpDone(Result<(), String>),
    SnapshotReadyClearForm,
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    tokens: Vec<TokenRecord>,
    servers: Vec<ServerState>,
    tools: HashMap<String, Vec<ServerToolView>>,
    oauth: HashMap<String, bool>,
}

fn secret_store() -> Arc<dyn SecretStore> {
    #[cfg(target_os = "macos")]
    {
        Arc::new(KeychainSecretStore::new())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Arc::new(MemorySecretStore::new())
    }
}

async fn load_snapshot(session: Session) -> Result<Snapshot, String> {
    let tokens = session.list_tokens()?;
    let servers = session.list_servers().await?;
    let mut tools = HashMap::new();
    let mut oauth = HashMap::new();
    for server in &servers {
        if server.status == ServerStatus::Running {
            tools.insert(
                server.config.id.clone(),
                session.list_server_tools(&server.config.id).await?,
            );
        }
        if server.config.server_type != ServerType::Local {
            oauth.insert(
                server.config.id.clone(),
                session.oauth_connected(&server.config.id)?,
            );
        }
    }
    Ok(Snapshot {
        tokens,
        servers,
        tools,
        oauth,
    })
}

impl App {
    fn boot() -> (Self, Task<Message>) {
        let data_dir = NativeAppPaths.data_dir();
        let secrets = secret_store();
        let connector = Arc::new(McpConnector::new(secrets.clone()));
        let session = Session::open(&data_dir, connector, secrets, Arc::new(NativeBrowserOpener))
            .expect("open desktop session");

        let tokens = session.tokens();
        let aggregator = session.aggregator();
        let http: Task<Message> = Task::future(async move {
            if let Err(error) = mcp_http::serve_with_aggregator(tokens, aggregator).await {
                eprintln!("mcp http server failed: {error}");
            }
        })
        .discard();

        let app = Self {
            endpoint: Session::aggregator_endpoint(),
            session: session.clone(),
            window_id: None,
            section: Section::Servers,
            show_add_form: false,
            editing_id: None,
            show_import_form: false,
            import_json: text_editor::Content::new(),
            notice: None,
            client_name: String::from("cursor"),
            tokens: Vec::new(),
            plaintext: None,
            servers: Vec::new(),
            tools_by_server: HashMap::new(),
            oauth_by_server: HashMap::new(),
            server_name: String::new(),
            server_type: FormServerType::Local,
            command: String::from("npx"),
            args: String::from("-y @modelcontextprotocol/server-everything"),
            remote_url: String::new(),
            env: text_editor::Content::new(),
            bearer: String::new(),
            auto_start: true,
            error: None,
        };

        let startup = Task::perform(
            async move {
                session.auto_start().await?;
                load_snapshot(session).await
            },
            Message::Snapshot,
        );

        (app, Task::batch([http, open_main_window(), startup]))
    }

    fn refresh(&self) -> Task<Message> {
        let session = self.session.clone();
        Task::perform(load_snapshot(session), Message::Snapshot)
    }

    fn apply_shell(&self, action: ShellAction) -> Task<Message> {
        match action {
            ShellAction::OpenWindow => open_main_window(),
            ShellAction::FocusWindow => self
                .window_id
                .map(window::gain_focus)
                .unwrap_or_else(open_main_window),
            ShellAction::CloseWindow => {
                self.window_id.map(window::close).unwrap_or_else(Task::none)
            }
            ShellAction::Exit => iced::exit(),
            ShellAction::CopyEndpoint => clipboard::write(self.endpoint.clone()),
        }
    }

    fn drain_tray(&self) -> Task<Message> {
        #[cfg(target_os = "macos")]
        {
            let mut tasks = Vec::new();
            while let Ok(event) = tray_icon::menu::MenuEvent::receiver().try_recv() {
                if let Some(command) = parse_tray_menu_id(event.id.as_ref()) {
                    tasks.push(self.apply_shell(on_tray(self.window_id.is_some(), command)));
                }
            }
            Task::batch(tasks)
        }
        #[cfg(not(target_os = "macos"))]
        {
            Task::none()
        }
    }

    fn set_error(&mut self, result: Result<(), String>) {
        match result {
            Ok(()) => self.error = None,
            Err(error) => self.error = Some(error),
        }
    }

    fn clear_form(&mut self) {
        self.editing_id = None;
        self.server_name.clear();
        self.server_type = FormServerType::Local;
        self.command = String::from("npx");
        self.args = String::from("-y @modelcontextprotocol/server-everything");
        self.remote_url.clear();
        self.env = text_editor::Content::new();
        self.bearer.clear();
        self.auto_start = true;
    }

    fn form_input(&self) -> FormInput<'_> {
        FormInput {
            name: &self.server_name,
            server_type: self.server_type,
            command: &self.command,
            args: &self.args,
            remote_url: &self.remote_url,
            env_text: self.env.text(),
            bearer: &self.bearer,
            auto_start: self.auto_start,
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::WindowOpened(id) => {
                if let Some(old) = self.window_id.replace(id) {
                    if old != id {
                        return window::close(old);
                    }
                }
                Task::none()
            }
            Message::CloseRequested => self.apply_shell(on_close_requested()),
            Message::WindowClosed(id) => {
                if self.window_id == Some(id) {
                    self.window_id = None;
                }
                Task::none()
            }
            Message::TrayTick => self.drain_tray(),
            Message::CopyEndpoint => clipboard::write(self.endpoint.clone()),
            Message::CopyPlaintext => self
                .plaintext
                .clone()
                .map(clipboard::write)
                .unwrap_or_else(Task::none),
            Message::Navigate(section) => {
                self.section = section;
                Task::none()
            }
            Message::ToggleAddForm => {
                // Leaving edit mode must not leave stale prefills behind.
                if self.editing_id.is_some() {
                    self.clear_form();
                }
                self.show_import_form = false;
                self.notice = None;
                self.show_add_form = !self.show_add_form;
                Task::none()
            }
            Message::CancelAddForm => {
                self.clear_form();
                self.show_add_form = false;
                self.notice = None;
                Task::none()
            }
            Message::ToggleImportForm => {
                if self.show_add_form {
                    self.clear_form();
                    self.show_add_form = false;
                }
                self.notice = None;
                self.show_import_form = !self.show_import_form;
                Task::none()
            }
            Message::CancelImportForm => {
                self.import_json = text_editor::Content::new();
                self.show_import_form = false;
                self.notice = None;
                Task::none()
            }
            Message::ImportText(action) => {
                self.import_json.perform(action);
                Task::none()
            }
            Message::ImportJson => {
                let session = self.session.clone();
                let raw = self.import_json.text();
                self.notice = None;
                Task::perform(
                    async move { session.import_json(&raw).await },
                    Message::ImportDone,
                )
            }
            Message::ImportDone(result) => match result {
                Ok(outcome) => {
                    self.notice = Some(outcome.summary());
                    self.import_json = text_editor::Content::new();
                    self.show_import_form = false;
                    self.error = None;
                    self.refresh()
                }
                Err(error) => {
                    self.error = Some(error);
                    Task::none()
                }
            },
            Message::EditServer(id) => {
                self.show_import_form = false;
                self.notice = None;
                let session = self.session.clone();
                Task::perform(
                    async move {
                        let config = session
                            .list_servers()
                            .await?
                            .into_iter()
                            .find(|state| state.config.id == id)
                            .ok_or_else(|| format!("unknown server: {id}"))?
                            .config;
                        let (env, bearer) = session.server_secret_values(&id).await?;
                        let mut lines: Vec<String> =
                            env.iter().map(|(key, value)| format!("{key}={value}")).collect();
                        lines.sort();
                        Ok(EditFormState {
                            id: id.clone(),
                            name: config.name,
                            server_type: config.server_type.into(),
                            command: config.command.unwrap_or_default(),
                            args: config.args.join(" "),
                            remote_url: config.remote_url.unwrap_or_default(),
                            env_text: lines.join("\n"),
                            bearer: bearer.unwrap_or_default(),
                            auto_start: config.auto_start,
                        })
                    },
                    Message::EditLoaded,
                )
            }
            Message::EditLoaded(result) => match result {
                Ok(form) => {
                    self.server_name = form.name;
                    self.server_type = form.server_type;
                    self.command = form.command;
                    self.args = form.args;
                    self.remote_url = form.remote_url;
                    self.env = text_editor::Content::with_text(&form.env_text);
                    self.bearer = form.bearer;
                    self.auto_start = form.auto_start;
                    self.editing_id = Some(form.id);
                    self.show_add_form = true;
                    self.error = None;
                    Task::none()
                }
                Err(error) => {
                    self.error = Some(error);
                    Task::none()
                }
            },
            Message::ClientName(value) => {
                self.client_name = value;
                Task::none()
            }
            Message::IssueToken => {
                let session = self.session.clone();
                let name = self.client_name.clone();
                Task::perform(
                    async move { session.issue_token(&name) },
                    Message::TokenIssued,
                )
            }
            Message::TokenIssued(result) => match result {
                Ok(issued) => {
                    self.plaintext = Some(issued.plaintext);
                    self.error = None;
                    self.refresh()
                }
                Err(error) => {
                    self.error = Some(error);
                    Task::none()
                }
            },
            Message::Revoke(id) => {
                let session = self.session.clone();
                Task::perform(async move { session.revoke_token(&id) }, Message::OpDone)
            }
            Message::ServerName(value) => {
                self.server_name = value;
                Task::none()
            }
            Message::ServerType(value) => {
                self.server_type = value;
                Task::none()
            }
            Message::Command(value) => {
                self.command = value;
                Task::none()
            }
            Message::Args(value) => {
                self.args = value;
                Task::none()
            }
            Message::RemoteUrl(value) => {
                self.remote_url = value;
                Task::none()
            }
            Message::Env(action) => {
                self.env.perform(action);
                Task::none()
            }
            Message::Bearer(value) => {
                self.bearer = value;
                Task::none()
            }
            Message::AutoStart(value) => {
                self.auto_start = value;
                Task::none()
            }
            Message::AddServer => {
                let session = self.session.clone();
                self.notice = None;
                let request = add_request_from_form(self.form_input());
                Task::perform(async move { session.add_server(request).await }, |result| {
                    match result {
                        Ok(_) => Message::SnapshotReadyClearForm,
                        Err(error) => Message::OpDone(Err(error)),
                    }
                })
            }
            Message::UpdateServer => {
                let Some(id) = self.editing_id.clone() else {
                    return Task::none();
                };
                let session = self.session.clone();
                self.notice = None;
                let request = add_request_from_form(self.form_input());
                Task::perform(
                    async move { session.update_server(&id, request).await },
                    |result| match result {
                        Ok(()) => Message::SnapshotReadyClearForm,
                        Err(error) => Message::OpDone(Err(error)),
                    },
                )
            }
            Message::Start(id) => {
                let session = self.session.clone();
                Task::perform(
                    async move { session.start_server(&id).await },
                    Message::OpDone,
                )
            }
            Message::Stop(id) => {
                let session = self.session.clone();
                Task::perform(
                    async move { session.stop_server(&id).await },
                    Message::OpDone,
                )
            }
            Message::Delete(id) => {
                let session = self.session.clone();
                self.notice = None;
                Task::perform(
                    async move { session.delete_server(&id).await.map(|_| ()) },
                    Message::OpDone,
                )
            }
            Message::ToggleTool { id, name, public } => {
                let session = self.session.clone();
                Task::perform(
                    async move { session.set_tool_permission(&id, &name, public).await },
                    Message::OpDone,
                )
            }
            Message::Oauth(id) => {
                let session = self.session.clone();
                Task::perform(
                    async move { session.oauth_connect(&id).await },
                    Message::OpDone,
                )
            }
            Message::Snapshot(result) => {
                match result {
                    Ok(snapshot) => {
                        self.tokens = snapshot.tokens;
                        self.servers = snapshot.servers;
                        self.tools_by_server = snapshot.tools;
                        self.oauth_by_server = snapshot.oauth;
                        self.error = None;
                    }
                    Err(error) => self.error = Some(error),
                }
                Task::none()
            }
            Message::OpDone(result) => {
                self.set_error(result);
                if self.error.is_none() {
                    self.refresh()
                } else {
                    Task::none()
                }
            }
            Message::SnapshotReadyClearForm => {
                self.clear_form();
                self.show_add_form = false;
                self.error = None;
                self.refresh()
            }
        }
    }

    fn title(_app: &Self, _window: window::Id) -> String {
        String::from("MCP Manager")
    }

    fn subscription(_app: &Self) -> Subscription<Message> {
        Subscription::batch([
            window::close_requests().map(|_| Message::CloseRequested),
            window::close_events().map(Message::WindowClosed),
            iced::time::every(std::time::Duration::from_millis(100)).map(|_| Message::TrayTick),
        ])
    }

    fn view(&self, _window: window::Id) -> Element<'_, Message> {
        let top_bar = container(
            row![
                text("MCP Manager").size(15).font(ui::SEMIBOLD),
                space::horizontal(),
                container(
                    text(&self.endpoint).size(12).font(Font::MONOSPACE).style(
                        |theme| text::Style {
                            color: Some(ui::theme::of(theme).text_secondary),
                        },
                    ),
                )
                .padding([6, 10])
                .style(ui::styles::chip),
                ui::secondary_button("Copy").on_press(Message::CopyEndpoint),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        )
        .padding([10, 16])
        .width(Length::Fill)
        .style(ui::styles::top_bar);

        let mut sidebar_content = column![].spacing(12);
        sidebar_content = sidebar_content.push(
            column![
                ui::nav_item(
                    self.section == Section::Servers,
                    "Servers",
                    Message::Navigate(Section::Servers),
                ),
                ui::nav_item(
                    self.section == Section::Tokens,
                    "Client tokens",
                    Message::Navigate(Section::Tokens),
                ),
            ]
            .spacing(2),
        );
        sidebar_content = sidebar_content.push(space::vertical());
        sidebar_content = sidebar_content.push(ui::secondary(
            "Closing this window hides the app to the menu bar.",
        ));

        let sidebar = container(sidebar_content.padding(12))
            .width(Length::Fixed(216.0))
            .height(Length::Fill)
            .style(ui::styles::sidebar);

        let pane = match self.section {
            Section::Servers => ui::servers::view(self),
            Section::Tokens => ui::tokens::view(self),
        };

        let mut body = column![].spacing(16);
        if let Some(error) = &self.error {
            body = body.push(ui::error_banner(error));
        }
        let body = body
            .push(scrollable(pane).width(Length::Fill).height(Length::Fill))
            .height(Length::Fill);
        let content = container(body)
            .padding(24)
            .width(Length::Fill)
            .height(Length::Fill);

        container(
            column![top_bar, row![sidebar, content].height(Length::Fill)]
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .style(ui::styles::app_background)
        .into()
    }
}
