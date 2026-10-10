//! `--gui`: a small status window for the background watcher. It never ingests anything
//! itself; if no watcher is running it installs and starts one (see [`service`]), then
//! shows what that watcher has submitted.

mod data;

use core::time::Duration;
use std::sync::{Arc, Mutex};

use iced::border;
use iced::font::Weight;
use iced::theme::Palette;
use iced::widget::{Space, button, column, container, row, scrollable, space, text, tooltip};
use iced::{Alignment, Color, Element, Font, Length, Size, Subscription, Task, Theme, color};
use tracing::{error, warn};

use crate::service;
use crate::update::{self, Update};
use data::{DAY_SECS, MatchRow, Snapshot, Stats, Tracker};

const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const UPDATE_CHECK_INTERVAL: Duration = Duration::from_hours(6);
// Bundled, so the window looks the same everywhere and never depends on system fonts.
const FONT: Font = Font::with_name("Fira Sans");
const BOLD: Font = Font {
    weight: Weight::Bold,
    ..FONT
};

pub(crate) fn run() -> i32 {
    let result = iced::application(App::boot, App::update, App::view)
        .title("Deadlock API Ingest")
        .font(include_bytes!("../../assets/fonts/FiraSans-Regular.ttf"))
        .font(include_bytes!("../../assets/fonts/FiraSans-Bold.ttf"))
        .default_font(FONT)
        .subscription(App::subscription)
        .theme(App::theme)
        .window(iced::window::Settings {
            size: Size::new(720.0, 640.0),
            min_size: Some(Size::new(520.0, 480.0)),
            icon: window_icon(),
            // Matches StartupWMClass in install-linux.sh's desktop entry, so the desktop
            // shows the open window under that launcher and icon.
            #[cfg(target_os = "linux")]
            platform_specific: iced::window::settings::PlatformSpecific {
                application_id: crate::paths::APP_NAME.to_owned(),
                ..Default::default()
            },
            ..iced::window::Settings::default()
        })
        .run();
    match result {
        Ok(()) => 0,
        Err(e) => {
            error!("GUI failed: {e}");
            1
        }
    }
}

/// `icon.ico`'s 64x64 image as RGBA, decoded by `build.rs`.
fn window_icon() -> Option<iced::window::Icon> {
    const RGBA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/window-icon.rgba"));
    iced::window::icon::from_rgba(RGBA.to_vec(), 64, 64)
        .inspect_err(|e| warn!("Invalid window icon: {e}"))
        .ok()
}

#[derive(Debug, Clone)]
enum Message {
    Tick,
    Refreshed(Snapshot),
    StartService,
    Started(Result<(), String>),
    CheckUpdate,
    UpdateChecked(Result<Option<Update>, String>),
    ApplyUpdate,
    UpdateApplied(Result<(), String>),
}

enum UpdateState {
    Checking,
    UpToDate,
    /// The check failed (offline, rate-limited); stay quiet, it's retried later.
    Unknown,
    Available(Update),
    Applying(Update),
    Failed(Update, String),
}

enum StartState {
    Idle,
    Starting,
    Failed(String),
}

struct App {
    theme: Theme,
    tracker: Arc<Mutex<Tracker>>,
    snapshot: Option<Snapshot>,
    start: StartState,
    update: UpdateState,
    /// A refresh is in flight; ticks are skipped so a slow first read can't pile them up.
    refreshing: bool,
}

impl App {
    fn boot() -> (Self, Task<Message>) {
        let mut app = Self {
            theme: Theme::custom(
                "Deadlock",
                Palette {
                    background: color!(0x15171c),
                    text: color!(0xe9e6df),
                    primary: color!(0xd6a24e),
                    success: color!(0x5cb87a),
                    warning: color!(0xe3a33b),
                    danger: color!(0xe0605a),
                },
            ),
            tracker: Arc::default(),
            snapshot: None,
            // `ensure_started` is a no-op when the watcher already runs; checking that here
            // would put file I/O in front of the first frame.
            start: StartState::Starting,
            update: UpdateState::Checking,
            refreshing: false,
        };
        let tasks = [app.refresh(), check_update(), start_service()];
        (app, Task::batch(tasks))
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick if self.refreshing => Task::none(),
            Message::Tick => self.refresh(),
            Message::Refreshed(snapshot) => {
                self.snapshot = Some(snapshot);
                self.refreshing = false;
                Task::none()
            }
            Message::StartService => {
                self.start = StartState::Starting;
                start_service()
            }
            Message::Started(result) => {
                self.start = match result {
                    Ok(()) => StartState::Idle,
                    Err(e) => StartState::Failed(e),
                };
                self.refresh()
            }
            Message::CheckUpdate => match self.update {
                // Don't interrupt an update in progress or hide its error.
                UpdateState::Applying(_) | UpdateState::Failed(..) => Task::none(),
                _ => check_update(),
            },
            Message::UpdateChecked(result) => {
                if !matches!(self.update, UpdateState::Applying(_)) {
                    self.update = match result {
                        Ok(Some(update)) => UpdateState::Available(update),
                        Ok(None) => UpdateState::UpToDate,
                        Err(e) => {
                            warn!("{e}");
                            UpdateState::Unknown
                        }
                    };
                }
                Task::none()
            }
            Message::ApplyUpdate => {
                let (UpdateState::Available(update) | UpdateState::Failed(update, _)) =
                    &self.update
                else {
                    return Task::none();
                };
                let update = update.clone();
                self.update = UpdateState::Applying(update.clone());
                Task::perform(
                    blocking(move || update::apply(&update)),
                    Message::UpdateApplied,
                )
            }
            Message::UpdateApplied(Ok(())) => iced::exit(),
            Message::UpdateApplied(Err(e)) => {
                if let UpdateState::Applying(update) = &self.update {
                    self.update = UpdateState::Failed(update.clone(), e);
                }
                Task::none()
            }
        }
    }

    fn refresh(&mut self) -> Task<Message> {
        self.refreshing = true;
        let tracker = Arc::clone(&self.tracker);
        Task::perform(
            blocking(move || {
                tracker
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .snapshot()
            }),
            Message::Refreshed,
        )
    }

    #[allow(clippy::unused_self)]
    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            iced::time::every(REFRESH_INTERVAL).map(|_| Message::Tick),
            iced::time::every(UPDATE_CHECK_INTERVAL).map(|_| Message::CheckUpdate),
        ])
    }

    fn theme(&self) -> Theme {
        self.theme.clone()
    }

    fn view(&self) -> Element<'_, Message> {
        let (stats, recent, now) = self.snapshot.as_ref().map_or_else(
            || (Stats::default(), &[][..], data::now_secs()),
            |s| (s.stats.clone(), &s.recent[..], s.now),
        );

        let update_status = match &self.update {
            UpdateState::Checking => "Checking for updates…",
            UpdateState::UpToDate => "Up to date",
            _ => "",
        };
        let footer = row![
            text(format!(
                "Deadlock API Ingest v{}",
                env!("CARGO_PKG_VERSION")
            ))
            .size(11)
            .style(text::secondary),
            space::horizontal(),
            text(update_status).size(11).style(text::secondary),
        ];

        let mut content = column![].spacing(16).padding(20);
        if let Some(bar) = self.update_bar() {
            content = content.push(bar);
        }
        content
            .push(self.status_banner())
            .push(stat_cards(&stats, now))
            .push(recent_matches(recent, now))
            .push(footer)
            .into()
    }

    /// Shown while a newer version is available, being installed, or failed to install.
    fn update_bar(&self) -> Option<Element<'_, Message>> {
        let (update, detail, button_label, can_press) = match &self.update {
            UpdateState::Available(u) => (
                u,
                text("Updates install in a few seconds; your matches keep being sent.")
                    .style(text::secondary),
                "Update now",
                true,
            ),
            UpdateState::Applying(u) => (
                u,
                text("Downloading and installing…").style(text::secondary),
                "Updating…",
                false,
            ),
            UpdateState::Failed(u, e) => {
                (u, text(e.as_str()).style(text::danger), "Try again", true)
            }
            UpdateState::Checking | UpdateState::UpToDate | UpdateState::Unknown => return None,
        };
        let content = row![
            column![
                text(format!("Version {} is available", update.version))
                    .size(15)
                    .font(BOLD),
                detail.size(12),
            ]
            .spacing(2)
            .width(Length::Fill),
            button(text(button_label).font(BOLD))
                .padding([8, 18])
                .on_press_maybe(can_press.then_some(Message::ApplyUpdate)),
        ]
        .spacing(14)
        .align_y(Alignment::Center);
        Some(
            container(content)
                .padding([12, 18])
                .width(Length::Fill)
                .style(update_card)
                .into(),
        )
    }

    fn status_banner(&self) -> Element<'_, Message> {
        let running = self.snapshot.as_ref().map(|s| s.running);
        let palette = self.theme.palette();

        let (dot, title, detail): (Color, &str, String) = match (running, &self.start) {
            (Some(true), _) => (
                palette.success,
                "Uploading your matches automatically",
                "Everything is set up. This keeps running in the background and starts with \
                 your computer, so you can close this window."
                    .to_owned(),
            ),
            (None, _) => (palette.warning, "Checking…", String::new()),
            (Some(false), StartState::Starting) => (
                palette.warning,
                "Setting things up…",
                "Installing and starting the background uploader.".to_owned(),
            ),
            (Some(false), StartState::Failed(e)) => {
                (palette.danger, "Couldn't start the uploader", e.clone())
            }
            (Some(false), StartState::Idle) => (
                palette.danger,
                "The uploader isn't running",
                "Start it to keep submitting the matches you play and watch.".to_owned(),
            ),
        };

        let mut content = row![
            status_dot(dot),
            column![
                text(title).size(18).font(BOLD),
                text(detail).size(13).style(text::secondary),
            ]
            .spacing(4)
            .width(Length::Fill),
        ]
        .spacing(14)
        .align_y(Alignment::Center);

        if running == Some(false) {
            let starting = matches!(self.start, StartState::Starting);
            content = content.push(
                button(text(if starting { "Starting…" } else { "Start" }).font(BOLD))
                    .padding([8, 20])
                    .on_press_maybe((!starting).then_some(Message::StartService)),
            );
        }

        container(content)
            .padding(18)
            .width(Length::Fill)
            .style(card)
            .into()
    }
}

fn stat_cards(stats: &Stats, now: i64) -> Element<'static, Message> {
    // Split as "5 min" / "ago" so the value stays on one line in a narrow card.
    let (last, last_hint) = match stats.last_ingested_at.map(|t| elapsed(now, t)) {
        None => ("—".to_owned(), ""),
        Some(None) => ("Just now".to_owned(), ""),
        Some(Some(e)) => (e, "ago"),
    };
    let via_steam = if stats.via_steam > 0 {
        format!("{} via Steam", stats.via_steam)
    } else {
        "all time".to_owned()
    };
    row![
        stat_card("Matches submitted", stats.total.to_string(), via_steam),
        stat_card("Last 24 hours", stats.last_day.to_string(), "matches"),
        stat_card("Last 7 days", stats.last_week.to_string(), "matches"),
        stat_card("Last upload", last, last_hint),
    ]
    .spacing(12)
    .into()
}

fn stat_card(
    label: &'static str,
    value: String,
    hint: impl Into<String>,
) -> Element<'static, Message> {
    container(
        column![
            text(label).size(12).style(text::secondary),
            text(value).size(28).font(BOLD),
            text(hint.into()).size(11).style(text::secondary),
        ]
        .spacing(2),
    )
    .padding(16)
    .width(Length::Fill)
    .height(Length::Fixed(104.0))
    .style(card)
    .into()
}

fn recent_matches(recent: &[MatchRow], now: i64) -> Element<'static, Message> {
    let title = text("Recent matches").size(16).font(BOLD);

    let body: Element<'static, Message> = if recent.is_empty() {
        container(
            column![
                text("No matches yet").size(15).font(BOLD),
                text(
                    "Play a match or open one from your match history in Deadlock. \
                     It shows up here a few seconds later."
                )
                .size(13)
                .style(text::secondary),
            ]
            .spacing(6)
            .align_x(Alignment::Center),
        )
        .center(Length::Fill)
        .into()
    } else {
        scrollable(
            column(recent.iter().map(|m| match_row(m, now)))
                .spacing(2)
                .padding(iced::Padding::ZERO.right(12)),
        )
        .height(Length::Fill)
        .into()
    };

    container(column![title, body].spacing(12))
        .padding(18)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(card)
        .into()
}

fn match_row(m: &MatchRow, now: i64) -> Element<'static, Message> {
    let mut badges = row![].spacing(6);
    if m.metadata {
        badges = badges.push(badge(
            "Match data",
            "Players, heroes, items and the outcome",
            false,
        ));
    }
    if m.replay {
        badges = badges.push(badge("Replay", "The full replay file", false));
    }
    if m.via_steam {
        badges = badges.push(badge(
            "via Steam",
            "One of your matches, recovered through your Steam account",
            true,
        ));
    }
    let when = m.ingested_at.map_or_else(String::new, |t| time_ago(now, t));

    container(
        row![
            text(format!("Match {}", m.match_id))
                .font(BOLD)
                .width(Length::Fixed(150.0)),
            badges,
            space::horizontal(),
            text(when).size(12).style(text::secondary),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    )
    .padding([8, 10])
    .style(list_row)
    .into()
}

fn badge(
    label: &'static str,
    explanation: &'static str,
    accent: bool,
) -> Element<'static, Message> {
    let pill = container(text(label).size(11))
        .padding([2, 8])
        .style(move |theme: &Theme| {
            let palette = theme.extended_palette();
            let pair = if accent {
                palette.primary.weak
            } else {
                palette.background.strong
            };
            container::Style {
                background: Some(pair.color.into()),
                text_color: Some(pair.text),
                border: border::rounded(10),
                ..container::Style::default()
            }
        });
    tooltip(
        pill,
        container(text(explanation).size(12))
            .padding(8)
            .style(container::rounded_box),
        tooltip::Position::Top,
    )
    .into()
}

fn status_dot(color: Color) -> Element<'static, Message> {
    container(Space::new().width(12).height(12))
        .style(move |_: &Theme| container::Style {
            background: Some(color.into()),
            border: border::rounded(6),
            ..container::Style::default()
        })
        .into()
}

fn card(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.weakest.color.into()),
        border: border::rounded(12)
            .width(1)
            .color(palette.background.weak.color),
        ..container::Style::default()
    }
}

fn update_card(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        border: border::rounded(12)
            .width(1)
            .color(palette.primary.base.color),
        ..card(theme)
    }
}

fn list_row(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.extended_palette().background.base.color.into()),
        border: border::rounded(8),
        ..container::Style::default()
    }
}

/// "5 min", "3 h", "2 days"; `None` for under a minute.
fn elapsed(now: i64, then: i64) -> Option<String> {
    let secs = (now - then).max(0);
    match secs {
        0..60 => None,
        60..3600 => Some(format!("{} min", secs / 60)),
        3600..DAY_SECS => Some(format!("{} h", secs / 3600)),
        _ if secs < 2 * DAY_SECS => Some("1 day".to_owned()),
        _ => Some(format!("{} days", secs / DAY_SECS)),
    }
}

fn time_ago(now: i64, then: i64) -> String {
    elapsed(now, then).map_or_else(|| "just now".to_owned(), |e| format!("{e} ago"))
}

fn check_update() -> Task<Message> {
    Task::perform(blocking(update::check), Message::UpdateChecked)
}

fn start_service() -> Task<Message> {
    Task::perform(blocking(service::ensure_started), Message::Started)
}

/// Runs file and process work off the UI thread.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f)
        .await
        .unwrap_or_else(|e| std::panic::resume_unwind(e.into_panic()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_time_ago() {
        let now = 10 * DAY_SECS;
        assert_eq!(time_ago(now, now + 5), "just now");
        assert_eq!(time_ago(now, now - 59), "just now");
        assert_eq!(time_ago(now, now - 125), "2 min ago");
        assert_eq!(time_ago(now, now - 3 * 3600), "3 h ago");
        assert_eq!(time_ago(now, now - DAY_SECS - 10), "1 day ago");
        assert_eq!(time_ago(now, now - 5 * DAY_SECS), "5 days ago");
    }
}
