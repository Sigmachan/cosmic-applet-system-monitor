use crate::monitor::{GpuStats, SystemMonitor, SystemStats};
use cosmic::{
    Element, app,
    applet::padded_control,
    cosmic_theme::Spacing,
    iced::{
        Alignment, Length, Subscription,
        widget::{column, container, row},
        window,
    },
    theme,
    widget::{button, divider, icon, text},
};

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

/// How often the panel and popup metrics refresh.
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

pub struct Window {
    core: cosmic::app::Core,
    /// Persistent collector — created once, reused every tick so CPU deltas and
    /// GPU discovery stay correct/cheap. Shared with blocking refresh tasks.
    monitor: Arc<Mutex<SystemMonitor>>,
    stats: SystemStats,
    popup: Option<window::Id>,
}

#[derive(Debug, Clone)]
pub enum Message {
    TogglePopup,
    CloseRequested(window::Id),
    Tick,
    StatsUpdate(SystemStats),
}

/// Builds a refresh task that samples the monitor off the UI thread.
fn refresh(monitor: Arc<Mutex<SystemMonitor>>) -> app::Task<Message> {
    cosmic::iced::Task::perform(
        tokio::task::spawn_blocking(move || {
            // A poisoned lock means a previous sample panicked; recover the
            // guard rather than cascading the panic into the UI loop.
            let mut guard = monitor.lock().unwrap_or_else(|e| e.into_inner());
            guard.stats()
        }),
        |result| match result {
            Ok(stats) => cosmic::Action::App(Message::StatsUpdate(stats)),
            Err(err) => {
                tracing::error!(%err, "stats refresh task failed");
                cosmic::Action::None
            }
        },
    )
}

/// A thin accent-coloured progress bar normalised to `[0, 100]`.
fn progress_bar(value: f32) -> Element<'static, Message> {
    const HEIGHT: f32 = 6.0;
    const SCALE: f32 = 1000.0;

    let filled = (value.clamp(0.0, 100.0) / 100.0).clamp(0.0, 1.0);
    // FillPortion(0) collapses a segment entirely; keep a 1-unit floor so the
    // track and fill never vanish at the extremes.
    let fill_portion = ((filled * SCALE) as u16).max(1);
    let rest_portion = (((1.0 - filled) * SCALE) as u16).max(1);

    let fill = container(row![])
        .width(Length::FillPortion(fill_portion))
        .height(Length::Fixed(HEIGHT))
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
        .width(Length::FillPortion(rest_portion))
        .height(Length::Fixed(HEIGHT))
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
        .height(Length::Fixed(HEIGHT))
        .into()
}

/// Formats an optional usage percentage as `"42%"` or `"--"`.
fn fmt_usage(usage: Option<f32>) -> String {
    usage.map_or_else(|| "--".to_string(), |u| format!("{u:.0}%"))
}

impl Window {
    fn panel_text(&self) -> String {
        let mut parts = vec![
            format!("CPU:{:3.0}%", self.stats.cpu_percent),
            format!("RAM:{:3.0}%", self.stats.ram_percent),
        ];

        if self.stats.gpus.is_empty() {
            parts.push("GPU: --".to_string());
        } else {
            for gpu in &self.stats.gpus {
                parts.push(format!("GPU:{:>4}", fmt_usage(gpu.usage_percent)));
            }
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
        let monitor = Arc::new(Mutex::new(SystemMonitor::new()));
        let task = refresh(monitor.clone());
        (
            Self {
                core,
                monitor,
                stats: SystemStats::default(),
                popup: None,
            },
            task,
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
        cosmic::iced::time::every(REFRESH_INTERVAL).map(|_| Message::Tick)
    }

    fn update(&mut self, message: Self::Message) -> app::Task<Self::Message> {
        match message {
            Message::Tick => refresh(self.monitor.clone()),
            Message::StatsUpdate(stats) => {
                self.stats = stats;
                app::Task::none()
            }
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

        let cpu_text = match self.stats.cpu_temp {
            Some(temp) => format!("{:.0}%  {:.0}°C", self.stats.cpu_percent, temp),
            None => format!("{:.0}%", self.stats.cpu_percent),
        };

        let ram_text = format!(
            "{:.1} / {:.1} GB  ({:.0}%)",
            self.stats.ram_used_gb, self.stats.ram_total_gb, self.stats.ram_percent
        );

        let mut content = column![
            metric_row("cpu-symbolic", "CPU", cpu_text),
            container(progress_bar(self.stats.cpu_percent)).padding([0, 12, 0, 44]),
            padded_control(divider::horizontal::default()).padding([space_xxs, space_s]),
            metric_row("memory-symbolic", "RAM", ram_text),
            container(progress_bar(self.stats.ram_percent)).padding([0, 12, 0, 44]),
        ];

        if !self.stats.gpus.is_empty() {
            content = content
                .push(padded_control(divider::horizontal::default()).padding([space_xxs, space_s]));
            content = content.push(padded_control(
                row![
                    icon::from_name("gpu-symbolic").size(24).symbolic(true),
                    text::body("GPU").width(Length::Fill),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            ));

            for gpu in &self.stats.gpus {
                content = content.push(
                    container(column![
                        text::caption(gpu_text(gpu)),
                        progress_bar(gpu.usage_percent.unwrap_or(0.0))
                    ])
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

/// A labelled metric header: `[icon]  Title / subtitle`.
fn metric_row<'a>(icon_name: &'a str, title: &'a str, subtitle: String) -> Element<'a, Message> {
    padded_control(
        row![
            icon::from_name(icon_name).size(24).symbolic(true),
            column![text::body(title), text::caption(subtitle)].width(Length::Fill),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    )
    .into()
}

/// One line of GPU detail, e.g. `"NVIDIA GeForce RTX 5090  37%  52°C"`.
fn gpu_text(gpu: &GpuStats) -> String {
    let usage = fmt_usage(gpu.usage_percent);
    match gpu.temp {
        Some(temp) => format!("{}  {}  {:.0}°C", gpu.name, usage, temp),
        None => format!("{}  {}", gpu.name, usage),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_usage_handles_known_and_unknown() {
        assert_eq!(fmt_usage(Some(0.0)), "0%");
        assert_eq!(fmt_usage(Some(42.4)), "42%");
        assert_eq!(fmt_usage(None), "--");
    }

    #[test]
    fn gpu_text_includes_temp_when_present() {
        let with_temp = GpuStats {
            name: "AMD (card1)".into(),
            usage_percent: Some(30.0),
            temp: Some(48.0),
        };
        assert_eq!(gpu_text(&with_temp), "AMD (card1)  30%  48°C");

        let no_temp = GpuStats {
            name: "NVIDIA RTX 5090".into(),
            usage_percent: None,
            temp: None,
        };
        assert_eq!(gpu_text(&no_temp), "NVIDIA RTX 5090  --");
    }
}
