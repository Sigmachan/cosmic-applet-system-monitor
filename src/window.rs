use crate::monitor::{SystemMonitor, SystemStats};
use cosmic::{
    Element, app,
    applet::padded_control,
    cosmic_theme::Spacing,
    iced::{
        Alignment, Length, Subscription, window,
        widget::{column, container, row},
    },
    theme,
    widget::{button, divider, icon, text},
};

use std::time::Duration;

pub struct Window {
    core: cosmic::app::Core,
    stats: SystemStats,
    popup: Option<window::Id>,
}

#[derive(Debug, Clone)]
pub enum Message {
    TogglePopup,
    CloseRequested(window::Id),
    Tick,
    StatsUpdate(Result<SystemStats, String>),
}

fn progress_bar(value: f32) -> Element<'static, Message> {
    let filled = (value.clamp(0.0, 100.0) / 100.0).clamp(0.0, 1.0);

    let fill = container(row![])
        .width(Length::FillPortion((filled * 1000.0) as u16))
        .height(Length::Fixed(6.0))
        .class(cosmic::style::Container::Custom(Box::new(|t| {
            container::Style {
                background: Some(cosmic::iced::core::Background::Color(
                    t.cosmic().accent_color().into(),
                )),
                border: cosmic::iced::core::Border {
                    radius: 3.0.into(),
                    width: 0.0,
                    color: cosmic::iced::core::Color::TRANSPARENT,
                },
                ..Default::default()
            }
        })));

    let bg = container(row![])
        .width(Length::FillPortion(((1.0 - filled) * 1000.0) as u16))
        .height(Length::Fixed(6.0))
        .class(cosmic::style::Container::Custom(Box::new(|_t| {
            container::Style {
                background: Some(cosmic::iced::core::Background::Color(
                    cosmic::iced::core::Color::from_rgba(1.0, 1.0, 1.0, 0.12),
                )),
                border: cosmic::iced::core::Border {
                    radius: 3.0.into(),
                    width: 0.0,
                    color: cosmic::iced::core::Color::TRANSPARENT,
                },
                ..Default::default()
            }
        })));

    container(row![fill, bg])
        .height(Length::Fixed(6.0))
        .into()
}

impl Window {
    fn panel_text(&self) -> String {
        let mut parts = Vec::new();

        parts.push(format!("CPU:{:3.0}%", self.stats.cpu_percent));
        parts.push(format!("RAM:{:3.0}%", self.stats.ram_percent));

        for gpu in &self.stats.gpus {
            parts.push(format!("GPU:{:3.0}%", gpu.usage_percent));
        }

        if self.stats.gpus.is_empty() {
            parts.push("GPU: --".to_string());
        }

        parts.join("  ")
    }
}

impl cosmic::Application for Window {
    type Message = Message;
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    const APP_ID: &str = "com.system76.CosmicAppletSystemMonitor";

    fn init(core: cosmic::app::Core, _flags: Self::Flags) -> (Self, app::Task<Self::Message>) {
        (
            Self {
                core,
                stats: SystemStats::default(),
                popup: None,
            },
            cosmic::iced::Task::perform(
                tokio::task::spawn_blocking(|| {
                    let mut monitor = SystemMonitor::new();
                    monitor.stats()
                }),
                |result| match result {
                    Ok(stats) => cosmic::Action::App(Message::StatsUpdate(Ok(stats))),
                    Err(e) => cosmic::Action::App(Message::StatsUpdate(Err(format!(
                        "spawn_blocking: {e}"
                    )))),
                },
            ),
        )
    }

    fn core(&self) -> &cosmic::app::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::app::Core {
        &mut self.core
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }

    fn subscription(&self) -> Subscription<Message> {
        cosmic::iced::time::every(Duration::from_secs(2)).map(|_| Message::Tick)
    }

    fn update(&mut self, message: Self::Message) -> app::Task<Self::Message> {
        match message {
            Message::Tick => {
                return cosmic::iced::Task::perform(
                    tokio::task::spawn_blocking(|| {
                        let mut monitor = SystemMonitor::new();
                        monitor.stats()
                    }),
                    |result| match result {
                        Ok(stats) => cosmic::Action::App(Message::StatsUpdate(Ok(stats))),
                        Err(e) => cosmic::Action::App(Message::StatsUpdate(Err(format!(
                            "spawn_blocking: {e}"
                        )))),
                    },
                );
            }
            Message::StatsUpdate(result) => match result {
                Ok(stats) => {
                    self.stats = stats;
                    app::Task::none()
                }
                Err(err) => {
                    tracing::error!("Stats update error: {err}");
                    app::Task::none()
                }
            },
            Message::TogglePopup => {
                if let Some(p) = self.popup.take() {
                    return cosmic::iced::platform_specific::shell::wayland::commands::popup::destroy_popup(p);
                }

                let new_id = window::Id::unique();
                self.popup = Some(new_id);

                let popup_settings = self.core.applet.get_popup_settings(
                    self.core.main_window_id().unwrap(),
                    new_id,
                    None,
                    None,
                    None,
                );

                cosmic::iced::platform_specific::shell::wayland::commands::popup::get_popup(
                    popup_settings,
                )
            }
            Message::CloseRequested(id) => {
                if Some(id) == self.popup {
                    self.popup = None;
                }
                app::Task::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let label = self.core.applet.text(self.panel_text());

        let button = button::custom(label)
            .padding([0, self.core.applet.suggested_padding(true).0])
            .on_press_down(Message::TogglePopup)
            .class(cosmic::theme::Button::AppletIcon);

        self.core.applet.autosize_window(button).into()
    }

    fn view_window(&self, _id: window::Id) -> Element<'_, Message> {
        let Spacing {
            space_xxs, space_s, ..
        } = theme::active().cosmic().spacing;

        let cpu_bar = progress_bar(self.stats.cpu_percent);
        let cpu_text = if let Some(temp) = self.stats.cpu_temp {
            format!("{:.0}%  {:.0}°C", self.stats.cpu_percent, temp)
        } else {
            format!("{:.0}%", self.stats.cpu_percent)
        };

        let ram_bar = progress_bar(self.stats.ram_percent);
        let ram_text = format!(
            "{:.1} / {:.1} GB  ({:.0}%)",
            self.stats.ram_used_gb, self.stats.ram_total_gb, self.stats.ram_percent
        );

        let mut content = column![
            padded_control(
                row![
                    icon::from_name("cpu-symbolic").size(24).symbolic(true),
                    column![text::body("CPU"), text::caption(cpu_text)].width(Length::Fill),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            ),
            container(cpu_bar).padding([0, 12, 0, 44]),
            padded_control(divider::horizontal::default())
                .padding([space_xxs, space_s]),
            padded_control(
                row![
                    icon::from_name("memory-symbolic").size(24).symbolic(true),
                    column![text::body("RAM"), text::caption(ram_text)].width(Length::Fill),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            ),
            container(ram_bar).padding([0, 12, 0, 44]),
        ];

        if !self.stats.gpus.is_empty() {
            content =
                content.push(
                    padded_control(divider::horizontal::default())
                        .padding([space_xxs, space_s]),
                );
            content = content.push(padded_control(
                row![
                    icon::from_name("gpu-symbolic").size(24).symbolic(true),
                    text::body("GPU").width(Length::Fill),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            ));

            for gpu in &self.stats.gpus {
                let gpu_bar = progress_bar(gpu.usage_percent);
                let gpu_text = if let Some(temp) = gpu.temp {
                    format!("{}  {:.0}%  {:.0}°C", gpu.name, gpu.usage_percent, temp)
                } else {
                    format!("{}  {:.0}%", gpu.name, gpu.usage_percent)
                };
                content = content.push(
                    container(column![text::caption(gpu_text), gpu_bar])
                        .padding([0, 12, 0, 44]),
                );
            }
        }

        self.core
            .applet
            .popup_container(container(content).padding([8, 0]))
            .into()
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        Some(Message::CloseRequested(id))
    }
}
