// SPDX-FileCopyrightText: 2026 Raphaël Doursenaud <raphael@doursenaud.fr>
// SPDX-FileCopyrightText: 2023 System76 <info@system76.com>
// SPDX-License-Identifier: GPL-3.0-only

use crate::thunderbolt::{
    BoltCommand, BoltDevice, BoltDeviceType, BoltError, BoltEvent, BoltGeneration, BoltState,
    BoltStatus, bolt_subscription,
};
use crate::ui_types::{BoltDeviceUiExt, format_security_level, security_level_is_problematic};
use crate::{config, fl};
use cosmic::applet::cosmic_panel_config::PanelAnchor;
use cosmic::surface::action::LiveSettings;
use cosmic::{
    Element, Task, app,
    applet::{
        menu_button, padded_control,
        token::subscription::{TokenRequest, TokenUpdate, activation_token_subscription},
    },
    cctk::sctk::reexports::calloop,
    cosmic_theme::Spacing,
    iced::{
        self, Alignment, Background, Border, Color, Length, Shadow, Subscription,
        core::window,
        platform_specific::shell::wayland::commands::popup::destroy_popup,
        widget::{column, container, row, stack},
    },
    theme,
    widget::{divider, icon, indeterminate_circular, scrollable, space, text},
};
use tokio::sync::mpsc::Sender;

// UI Constraints for scrollable lists
// FIXME: Evaluate if these should apply to the main active device list as well in future versions.
const MAX_DEVICES_VISIBLE: usize = 10;
const SCROLLABLE_LIST_HEIGHT: f32 = 300.0;

#[inline]
pub fn run() -> iced::Result {
    cosmic::applet::run::<CosmicExtThunderboltApplet>(())
}

/// Thunderbolt™ applet for the COSMIC™ desktop environment
///
/// Manages subscription to `boltd` D-Bus events,
/// maintains local state
/// and renders the user interface in a contextual popup.
#[derive(Default)]
struct CosmicExtThunderboltApplet {
    core: app::Core,
    icon_name: String,
    popup: Option<window::Id>,
    bolt_state: BoltState,
    bolt_sender: Option<Sender<BoltCommand>>,
    config: config::ThunderboltAppletConfig,
    error_state: Option<BoltError>,

    // Cached device lists categorized by status for efficient UI rendering
    // TODO: Refactor into a tree structure when implementing hierarchical view.
    host_devices: Vec<BoltDevice>,
    available_devices: Vec<BoltDevice>,
    authorized_devices: Vec<BoltDevice>,
    disconnected_devices: Vec<BoltDevice>,

    // UI toggle state
    show_disconnected_devices: bool,

    token_tx: Option<calloop::channel::Sender<TokenRequest>>,
}

impl CosmicExtThunderboltApplet {
    /// Formats a user-friendly error title and message based on the `BoltError` type.
    fn format_error(err: &BoltError) -> (String, String) {
        let title = match err {
            BoltError::ServiceNotFound => fl!("error-title-service-not-found"),
            _ => fl!("error-title-thunderbolt"),
        };

        let message = match err {
            BoltError::DbusConnectionFailed => fl!("error-dbus-connection"),
            BoltError::ServiceNotFound => fl!("error-service-not-found"),
            BoltError::ProxyConnectionFailed => fl!("error-proxy-connection"),
            BoltError::PropertyReadFailed => fl!("error-property-read"),
            BoltError::DaemonTerminated => fl!("error-daemon-terminated"),
            BoltError::GenericError { detail } => fl!("error-generic", detail = detail.as_str()),
            &BoltError::InvalidDestination
            | &BoltError::InvalidObjectPath
            | &BoltError::SignalSubscriptionFailed => todo!(),
        };

        (title, message)
    }

    /// Updates the tray icon based on the current error state.
    #[inline]
    fn update_icon(&mut self) {
        self.icon_name = if self.error_state.is_some() {
            "dialog-error-symbolic"
        } else {
            "thunderbolt-symbolic"
        }
        .to_string();
    }

    /// Rebuilds the categorized device lists from the raw `bolt_state`.
    ///
    /// This ensures the UI reflects the current D-Bus state accurately.
    /// Host devices are filtered out by default unless explicitly configured.
    fn refresh_devices_lists(&mut self) {
        self.host_devices.clear();
        self.available_devices.clear();
        self.authorized_devices.clear();
        self.disconnected_devices.clear();

        for dev in &self.bolt_state.devices {
            if dev.device_type == crate::thunderbolt::BoltDeviceType::Host {
                if self.config.show_host_device {
                    self.host_devices.push(dev.clone());
                }
                continue;
            }

            match dev.status {
                BoltStatus::Unknown | BoltStatus::AuthError => {
                    // TODO: Display a specific error icon for AuthError states
                    self.available_devices.push(dev.clone());
                }
                BoltStatus::Connecting | BoltStatus::Connected | BoltStatus::Authorizing => {
                    self.available_devices.push(dev.clone());
                }
                BoltStatus::Authorized => {
                    self.authorized_devices.push(dev.clone());
                }
                BoltStatus::Disconnected => {
                    self.disconnected_devices.push(dev.clone());
                }
            }
        }
    }

    /// Constructs the UI row for a single Thunderbolt™ device.
    fn build_device_row(dev: &BoltDevice, action_msg: Message) -> Element<'_, Message> {
        let Spacing { .. } = theme::active().cosmic().spacing;

        let version_number = if dev.generation == BoltGeneration::Unknown {
            None
        } else {
            Some(dev.ui_generation_text())
        };

        let icon_column = if let Some(ver) = version_number {
            column![
                icon::from_name(dev.ui_icon_name()).size(24).symbolic(true),
                text::caption(ver).size(10).align_x(Alignment::Center)
            ]
            .align_x(Alignment::Center)
            .spacing(2)
        } else {
            column![icon::from_name(dev.ui_icon_name()).size(24).symbolic(true)]
                .align_x(Alignment::Center)
        };

        let vendor_widget = if let Some(ref v) = dev.vendor {
            text::caption(v.as_str()).align_x(Alignment::Start)
        } else {
            text::caption("").align_x(Alignment::Start)
        };

        let info_column = column![
            text::body(dev.ui_display_label()).align_x(Alignment::Start),
            vendor_widget
        ]
        .align_x(Alignment::Start)
        .width(Length::Fill)
        .spacing(2);

        let status_widget: Element<Message> = match &dev.status {
            BoltStatus::Unknown | BoltStatus::AuthError => {
                icon::from_name("emblem-important-symbolic")
                    .size(24)
                    .symbolic(true)
                    .into()
            }
            BoltStatus::Connecting | BoltStatus::Authorizing => {
                indeterminate_circular().size(24.0).into()
            }
            BoltStatus::Connected => {
                let dot = container(
                    space::vertical()
                        .width(Length::Fixed(0.0))
                        .height(Length::Fixed(0.0)),
                )
                .padding(4)
                .class(cosmic::style::Container::Custom(Box::new(|theme| {
                    container::Style {
                        text_color: Some(Color::TRANSPARENT),
                        background: Some(Background::Color(theme.cosmic().accent_color().into())),
                        border: Border {
                            radius: 4.0.into(),
                            width: 0.0,
                            color: Color::TRANSPARENT,
                        },
                        shadow: Shadow::default(),
                        icon_color: Some(Color::TRANSPARENT),
                        snap: true,
                    }
                })));

                dot.align_y(Alignment::Center).into()
            }
            BoltStatus::Authorized | BoltStatus::Disconnected => {
                if dev.device_type == BoltDeviceType::Host {
                    icon::from_name("computer-symbolic")
                        .size(24)
                        .symbolic(true)
                        .into()
                } else {
                    icon::from_name("emblem-ok-symbolic")
                        .size(24)
                        .symbolic(true)
                        .into()
                }
            }
        };

        let row = row![icon_column, info_column, status_widget]
            .align_y(Alignment::Center)
            .spacing(12);

        if dev.device_type == BoltDeviceType::Host {
            menu_button(row).into()
        } else {
            menu_button(row).on_press(action_msg).into()
        }
    }

    /// Builds a scrollable section containing a list of devices.
    fn build_device_list_section<'a>(
        devices: &'a [BoltDevice],
        _header_key: &'a str,
        action_fn: impl Fn(&BoltDevice) -> Message,
    ) -> Element<'a, Message> {
        let mut column = column![];

        for dev in devices {
            let row_element = CosmicExtThunderboltApplet::build_device_row(dev, action_fn(dev));
            column = column.push(row_element);
        }

        column.into()
    }

    /// Checks if any device requires user attention (e.g., authorization or error).
    fn needs_attention(&self) -> bool {
        self.available_devices.iter().any(|dev| {
            matches!(
                dev.status,
                BoltStatus::Unknown | BoltStatus::Connected | BoltStatus::AuthError
            )
        })
    }

    // --- Update Helpers ---

    fn handle_toggle_popup(&mut self) -> app::Task<Message> {
        if let Some(p) = self.popup.take() {
            Task::batch([destroy_popup(p)])
        } else {
            let get_popup_task = cosmic::surface::surface_task(cosmic::surface::action::app_popup(
                |_| LiveSettings::default(),
                move |app: &mut Self| {
                    let new_id = window::Id::unique();
                    app.popup.replace(new_id);
                    app.core.applet.get_popup_settings(
                        app.core.main_window_id().unwrap(),
                        new_id,
                        None,
                        None,
                        None,
                    )
                },
                None,
            ));

            Task::batch([get_popup_task])
        }
    }

    fn handle_thunderbolt_event(&mut self, event: BoltEvent) {
        match event {
            BoltEvent::Error(err) => {
                self.error_state = Some(err);
                self.bolt_sender = None;
                self.host_devices.clear();
                self.available_devices.clear();
                self.authorized_devices.clear();
                self.disconnected_devices.clear();
                self.update_icon();
            }
            BoltEvent::Init { sender, state } => {
                self.error_state = None;
                self.bolt_sender.replace(sender);
                self.bolt_state = state;
                self.refresh_devices_lists();
                self.update_icon();
            }
            BoltEvent::DevicesChanged { state } => {
                self.error_state = None;
                self.bolt_state = state;

                // Debug logging to trace state synchronization between D-Bus and UI
                for dev in &self.bolt_state.devices {
                    tracing::debug!(
                        "UI SYNC: Device {} -> Status: {:?}",
                        dev.uid.as_str(),
                        dev.status
                    );
                }

                self.refresh_devices_lists();
                self.update_icon();
            }
            BoltEvent::Finished => {
                // The D-Bus subscription stream has ended. Since the applet relies entirely
                // on `boltd`, a clean restart is not possible without the daemon.
                // Exiting allows the shell to potentially restart the applet later.
                // TODO: Evaluate if a graceful restart mechanism should be implemented instead of exit.
                eprintln!("thunderbolt subscription finished. exiting...");
                std::process::exit(0);
            }
        }
    }

    fn handle_request(&mut self, r: BoltCommand) {
        // Optimistically update the UI state before the daemon confirms the action.
        // This provides immediate visual feedback to the user.
        match &r {
            BoltCommand::AuthorizeDevice(uid) => {
                if let Some(d) = self.bolt_state.devices.iter_mut().find(|d| d.uid == *uid) {
                    d.status = BoltStatus::Authorizing;
                }
            }
            BoltCommand::EnrollDevice(uid) => {
                if let Some(d) = self.bolt_state.devices.iter_mut().find(|d| d.uid == *uid) {
                    d.status = BoltStatus::Authorizing;
                }
            }
            BoltCommand::ForgetDevice(uid) => {
                if let Some(d) = self.bolt_state.devices.iter_mut().find(|d| d.uid == *uid) {
                    d.stored = false;
                }
            }
            BoltCommand::SetDeviceLabel { uid: _, label: _ } => {
                // TODO: Implement device labeling support
                todo!("SetDeviceLabel is not implemented yet");
            }
        }

        // Update optimistic UI state
        self.refresh_devices_lists();

        // Send the command to the background worker via the channel.
        // We clone the sender to avoid borrowing issues in the async block.
        if let Some(tx) = self.bolt_sender.clone() {
            tokio::spawn(async move {
                let _ = tx.send(r).await;
            });
        }
    }

    fn handle_token_update(&mut self, u: TokenUpdate) {
        match u {
            TokenUpdate::Init(tx) => {
                self.token_tx = Some(tx);
            }
            TokenUpdate::Finished => {
                self.token_tx = None;
            }
            TokenUpdate::ActivationToken { token, .. } => {
                // Launch the settings application with the activation token
                // to ensure proper window focusing on Wayland.
                let mut cmd = std::process::Command::new("cosmic-settings");
                cmd.arg("thunderbolt");
                if let Some(token) = token {
                    cmd.env("XDG_ACTIVATION_TOKEN", &token);
                    cmd.env("DESKTOP_STARTUP_ID", &token);
                }
                tokio::spawn(cosmic::process::spawn(cmd));
            }
        }
    }

    // --- View Helpers ---

    fn view_error_state(&self) -> Element<'_, Message> {
        let err = self.error_state.as_ref().unwrap();
        let (title, message) = CosmicExtThunderboltApplet::format_error(err);

        let content = column![
            padded_control(
                row![
                    icon::from_name("dialog-error-symbolic")
                        .size(32)
                        .symbolic(true),
                    column![text::title3(title), text::body(message)].spacing(8)
                ]
                .spacing(12)
                .align_y(Alignment::Center)
            ),
            padded_control(text::caption(fl!("error-check-boltd-installed")))
        ]
        .align_x(Alignment::Center)
        .padding(20);

        self.core.applet.popup_container(content).into()
    }

    fn view_active_devices(&self) -> Element<'_, Message> {
        let mut active_column = column![];

        // List already authorized devices (no action needed).
        for dev in &self.authorized_devices {
            let row_element: Element<Message>;
            if dev.stored {
                row_element = CosmicExtThunderboltApplet::build_device_row(
                    dev,
                    Message::Request(BoltCommand::ForgetDevice(dev.uid.clone())),
                );
            } else {
                row_element = CosmicExtThunderboltApplet::build_device_row(dev, Message::Ignore);
            }
            active_column = active_column.push(row_element);
        }

        // List devices awaiting authorization/enrollment.
        for dev in &self.available_devices {
            let row_element = CosmicExtThunderboltApplet::build_device_row(
                dev,
                Message::Request(BoltCommand::EnrollDevice(dev.uid.clone())),
            );
            active_column = active_column.push(row_element);
        }

        column![active_column].into()
    }

    fn view_disconnected_devices(&self) -> Element<'_, Message> {
        let dropdown_icon = if self.show_disconnected_devices {
            "go-up-symbolic"
        } else {
            "go-down-symbolic"
        };

        let disconnected_devices_btn = menu_button(row![
            text::body(fl!("disconnected-devices"))
                .width(Length::Fill)
                .height(Length::Fixed(24.0))
                .align_y(Alignment::Center),
            container(icon::from_name(dropdown_icon).size(16).symbolic(true))
                .center(Length::Fixed(24.0))
        ])
        .on_press(Message::ToggleDisconnectedDevices(
            !self.show_disconnected_devices,
        ));

        let mut content = column![disconnected_devices_btn];

        if self.show_disconnected_devices && !self.disconnected_devices.is_empty() {
            let list_content = CosmicExtThunderboltApplet::build_device_list_section(
                &self.disconnected_devices,
                "disconnected-header",
                |dev| Message::Request(BoltCommand::ForgetDevice(dev.uid.clone())),
            );

            // Limit the height of the list if there are many devices to avoid
            // overflowing the screen.
            // FIXME: add length to the config?
            // FIXME: compute the height dynamically according to monitor height?
            if self.disconnected_devices.len() > MAX_DEVICES_VISIBLE {
                content = content
                    .push(scrollable(list_content).height(Length::Fixed(SCROLLABLE_LIST_HEIGHT)));
            } else {
                content = content.push(list_content);
            }
        }

        column![content].into()
    }
}

#[derive(Debug, Clone)]
enum Message {
    ConfigChanged(config::ThunderboltAppletConfig),
    TogglePopup,
    CloseRequested(window::Id),
    ToggleDisconnectedDevices(bool),
    Ignore,
    ThunderboltEvent(BoltEvent),
    Request(BoltCommand),
    Token(TokenUpdate),
    OpenSettings,
}

impl cosmic::Application for CosmicExtThunderboltApplet {
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    type Message = Message;
    const APP_ID: &'static str = config::APP_ID;

    fn core(&self) -> &app::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut app::Core {
        &mut self.core
    }

    fn init(core: app::Core, _flags: Self::Flags) -> (Self, app::Task<Self::Message>) {
        (
            Self {
                core,
                icon_name: "thunderbolt-symbolic".to_string(),
                token_tx: None,
                ..Default::default()
            },
            Task::none(),
        )
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        Some(Message::CloseRequested(id))
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            self.core.watch_config(Self::APP_ID).map(|u| {
                for err in u.errors {
                    tracing::error!(?err, "Error watching config");
                }
                Message::ConfigChanged(u.config)
            }),
            activation_token_subscription(0).map(Message::Token),
            bolt_subscription(0).map(Message::ThunderboltEvent),
        ])
    }

    fn update(&mut self, message: Self::Message) -> app::Task<Self::Message> {
        match message {
            Message::ConfigChanged(c) => {
                self.config = c;
                self.refresh_devices_lists();
            }
            Message::TogglePopup => {
                return self.handle_toggle_popup();
            }
            Message::Ignore => {}
            Message::ToggleDisconnectedDevices(enabled) => {
                self.show_disconnected_devices = enabled;
            }
            Message::ThunderboltEvent(e) => {
                self.handle_thunderbolt_event(e);
            }
            Message::Request(r) => {
                self.handle_request(r);
            }
            Message::CloseRequested(id) => {
                if Some(id) == self.popup {
                    self.popup = None;
                }
            }
            Message::OpenSettings => {
                let exec = "cosmic-settings thunderbolt".to_string();
                if let Some(tx) = self.token_tx.as_ref() {
                    let _ = tx.send(TokenRequest {
                        app_id: Self::APP_ID.to_string(),
                        exec,
                    });
                }
            }
            Message::Token(u) => {
                self.handle_token_update(u);
            }
        }

        // Ensure the icon reflects the latest state after any message processing.
        self.update_icon();
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let btn = self
            .core
            .applet
            .icon_button(&self.icon_name)
            .on_press_down(Message::TogglePopup);

        if self.needs_attention() {
            // Create a small colored dot to overlay on the icon.
            // The dot is technically transparent text with a colored background.
            let dot = container(space::vertical().height(Length::Fixed(0.0)))
                .padding(2.0)
                .class(cosmic::style::Container::Custom(Box::new(|theme| {
                    container::Style {
                        text_color: Some(Color::TRANSPARENT),
                        background: Some(Background::Color(theme.cosmic().accent_color().into())),
                        border: Border {
                            radius: 2.0.into(),
                            width: 0.0,
                            color: Color::TRANSPARENT,
                        },
                        shadow: Shadow::default(),
                        icon_color: Some(Color::TRANSPARENT),
                        snap: true,
                    }
                })));

            // Calculate dot alignment based on the panel's position to ensure
            // the dot remains visible outside the icon bounds.
            let (dot_align_x, dot_align_y) = match self.core.applet.anchor {
                PanelAnchor::Left => (Alignment::Start, Alignment::Center),
                PanelAnchor::Right => (Alignment::End, Alignment::Center),
                PanelAnchor::Top => (Alignment::Center, Alignment::Start),
                PanelAnchor::Bottom => (Alignment::Center, Alignment::End),
            };

            let dot_container = container(dot)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(dot_align_x)
                .align_y(dot_align_y)
                .padding(2.0);

            stack![btn, dot_container].into()
        } else {
            btn.into()
        }
    }

    fn view_window(&self, _id: window::Id) -> Element<'_, Message> {
        let Spacing {
            space_xxs, space_s, ..
        } = theme::active().cosmic().spacing;

        // If an error state exists, display the error view immediately.
        if let Some(_err) = &self.error_state {
            return self.view_error_state();
        }

        let mut content = column![];

        let security_label = format_security_level(self.bolt_state.security_level);

        if security_level_is_problematic(self.bolt_state.security_level) {
            content = content.push(padded_control(
                row![
                    icon::from_name("dialog-warning-symbolic")
                        .size(24)
                        .symbolic(true),
                    column![
                        text::body(fl!("security-level-warning")),
                        //text::caption(security_label),
                    ]
                    .spacing(4)
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            ));

            //return self.core.applet.popup_container(content).into();
        }

        // Display host controllers if configured to do so.
        if !self.host_devices.is_empty() {
            for dev in &self.host_devices {
                let row_element =
                    CosmicExtThunderboltApplet::build_device_row(dev, Message::Ignore);
                content = content.push(row_element);
            }
        }

        if self.config.show_security_level {
            content = content
                .push(padded_control(
                    row![
                        text::body(fl!("security-level")).width(Length::Fill),
                        text::caption(security_label),
                    ]
                    .align_y(Alignment::Center)
                    .spacing(12),
                ))
                .align_x(Alignment::Center)
                .padding([8, 0]);
        }

        if self.config.show_host_device || self.config.show_security_level {
            content = content
                .push(padded_control(divider::horizontal::default()).padding([space_xxs, space_s]));
        }

        let has_active_devices =
            !self.authorized_devices.is_empty() || !self.available_devices.is_empty();

        if has_active_devices {
            content = content.push(self.view_active_devices());
        }

        // Handle disconnected devices section with a collapsible dropdown.
        if !self.disconnected_devices.is_empty() {
            content = content.push(self.view_disconnected_devices());
        }

        // Add the settings button at the bottom of the popup.
        content = content
            .push(padded_control(divider::horizontal::default()).padding([space_xxs, space_s]))
            .push(menu_button(text::body(fl!("settings"))).on_press(Message::OpenSettings));

        self.core.applet.popup_container(content).into()
    }

    fn style(&self) -> Option<iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}
