use std::process::Command;
use std::time::{Duration, Instant};

use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants};
use gpui_kit::component::dialog::{DialogClose, DialogFooter};
use gpui_kit::component::input::{InputEvent, InputState, NumberInput};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

actions!(pomominders, [Quit]);

const PLAY: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1" stroke-linejoin="round"><circle cx="12" cy="12" r="11.5"/><path d="M9.5 7.5v9l7-4.5z"/></svg>"##;
const PAUSE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1" stroke-linecap="round"><circle cx="12" cy="12" r="11.5"/><path d="M10.5 8.5v7M13.5 8.5v7"/></svg>"##;
const RESET: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="11.5"/><path d="M17 12H7M11 8l-4 4 4 4"/></svg>"##;
const MORE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1"><circle cx="12" cy="12" r="11.5"/><g fill="#000" stroke="none"><circle cx="8" cy="12" r=".75"/><circle cx="12" cy="12" r=".75"/><circle cx="16" cy="12" r=".75"/></g></svg>"##;

/// (label, default); the first three are durations in minutes, indexed by `Stage`.
const SETTINGS: [(&str, u64); 4] = [
    ("Work duration", 25),
    ("Short break duration", 5),
    ("Long break duration", 15),
    ("Long break interval", 4),
];
const INTERVAL: usize = 3;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Stage {
    Work,
    Short,
    Long,
}

impl Stage {
    const ALL: [Stage; 3] = [Stage::Work, Stage::Short, Stage::Long];

    fn label(self) -> &'static str {
        match self {
            Stage::Work => "Work",
            Stage::Short => "Short Break",
            Stage::Long => "Long Break",
        }
    }

    fn color(self) -> Hsla {
        match self {
            Stage::Work => rgb(0x923737),
            Stage::Short => rgb(0x2C6A6E),
            Stage::Long => rgb(0x2E5A79),
        }
        .into()
    }
}

/// Stage that follows `stage`, given work sessions completed so far (including this one).
fn next_stage(stage: Stage, works_done: u64, interval: u64) -> Stage {
    match stage {
        Stage::Work if works_done % interval == 0 => Stage::Long,
        Stage::Work => Stage::Short,
        _ => Stage::Work,
    }
}

fn faded(alpha: f32) -> Hsla {
    hsla(0., 0., 1., alpha)
}

// Button draws its icon at 75% of the button size.
fn icon_button(id: &'static str, svg: &[u8], cx: &App) -> Button {
    Button::new(id)
        .custom(
            ButtonCustomVariant::new(cx)
                .foreground(faded(0.5))
                .hover(faded(0.08))
                .active(faded(0.15)),
        )
        .rounded(px(999.))
        .icon(Icon::default().data(svg))
        .with_size(px(48.))
}

// Lucide, inlined: the default `assets::Assets` bundle doesn't include these.
const CLOCK: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><path d="M12 6v6l4 2"/></svg>"##;
const VOLUME: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M11 4.702a.705.705 0 0 0-1.203-.498L6.413 7.587A1.4 1.4 0 0 1 5.416 8H3a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2.416a1.4 1.4 0 0 1 .997.413l3.383 3.384A.705.705 0 0 0 11 19.298z"/><path d="M16 9a5 5 0 0 1 0 6"/><path d="M19.364 18.364a9 9 0 0 0 0-12.728"/></svg>"##;

/// Width of the settings control column, so every control shares one right edge.
const CONTROL_W: f32 = 160.;

fn group_header(icon: &[u8], title: &'static str, cx: &App) -> Div {
    h_flex()
        .gap_2()
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(cx.theme().muted_foreground)
        .child(Icon::default().data(icon).size_4())
        .child(title)
}

fn setting_row(
    label: &'static str,
    hint: Option<&'static str>,
    control: impl IntoElement,
    cx: &App,
) -> Div {
    h_flex()
        .justify_between()
        .gap_6()
        .child(
            v_flex()
                .gap_0p5()
                .child(div().text_sm().child(label))
                .when_some(hint, |this, hint| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(hint),
                    )
                }),
        )
        .child(
            h_flex()
                .w(px(CONTROL_W))
                .flex_shrink_0()
                .justify_end()
                .child(control),
        )
}

/// Rendered inside the settings dialog; its own view so controls re-render while it's open.
struct Settings {
    inputs: [Entity<InputState>; 4],
    auto_start: bool,
    volume: Entity<SliderState>,
}

impl Settings {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let inputs = SETTINGS.map(|(_, default)| {
            let state = cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(default.to_string())
                    .step(1.)
                    .min(1.)
                    .max(999.)
            });
            cx.subscribe(&state, |_, _, _: &InputEvent, cx| cx.notify())
                .detach();
            state
        });
        let volume = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(100.)
                .step(1.)
                .default_value(100.)
        });
        cx.subscribe(&volume, |_, _, _: &SliderEvent, cx| cx.notify())
            .detach();
        Self {
            inputs,
            auto_start: false,
            volume,
        }
    }

    fn get(&self, i: usize, cx: &App) -> u64 {
        let value = self.inputs[i].read(cx).value();
        value
            .trim()
            .parse()
            .ok()
            .filter(|&n| n > 0)
            .unwrap_or(SETTINGS[i].1)
    }

    /// 0.0..=1.0
    fn volume(&self, cx: &App) -> f32 {
        self.volume.read(cx).value().end() / 100.
    }
}

impl Render for Settings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let number = |i: usize, hint| {
            setting_row(
                SETTINGS[i].0,
                Some(hint),
                NumberInput::new(&self.inputs[i]).w_full(),
                cx,
            )
        };

        v_flex()
            .gap_5()
            .child(
                v_flex()
                    .gap_3()
                    .child(group_header(CLOCK, "Timer", cx))
                    .children([0, 1, 2].map(|i| number(i, "In minutes")))
                    .child(number(INTERVAL, "Work sessions before a long break"))
                    .child(setting_row(
                        "Auto start sessions",
                        None,
                        Switch::new("auto-start")
                            .checked(self.auto_start)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.auto_start = *checked;
                                cx.notify();
                            })),
                        cx,
                    )),
            )
            .child(div().h_px().bg(cx.theme().border))
            .child(
                v_flex()
                    .gap_3()
                    .child(group_header(VOLUME, "Sound", cx))
                    .child(setting_row(
                        "Alarm volume",
                        None,
                        // Inset so the thumb's overhang stays inside the control column.
                        div().w_full().px_2().child(Slider::new(&self.volume)),
                        cx,
                    )),
            )
    }
}

struct Pomodoro {
    stage: Stage,
    /// Paused time left; `None` means the stage hasn't started yet.
    remaining: Option<Duration>,
    ends_at: Option<Instant>,
    works_done: u64,
    settings: Entity<Settings>,
    ticker: Option<Task<()>>,
}

impl Pomodoro {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = cx.new(|cx| Settings::new(window, cx));
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();
        Self {
            stage: Stage::Work,
            remaining: None,
            ends_at: None,
            works_done: 0,
            settings,
            ticker: None,
        }
    }

    fn setting(&self, i: usize, cx: &App) -> u64 {
        self.settings.read(cx).get(i, cx)
    }

    fn open_settings(&self, window: &mut Window, cx: &mut App) {
        let settings = self.settings.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .w(px(440.))
                .title("Settings")
                .child(settings.clone())
                .footer(
                    DialogFooter::new()
                        .w_full()
                        .child(DialogClose::new().trigger(|b| b.primary().w_full().label("Ok"))),
                )
        });
    }

    fn full(&self, cx: &App) -> Duration {
        Duration::from_secs(60 * self.setting(self.stage as usize, cx))
    }

    fn left(&self, cx: &App) -> Duration {
        match self.ends_at {
            Some(end) => end.saturating_duration_since(Instant::now()),
            None => self.remaining.unwrap_or_else(|| self.full(cx)),
        }
    }

    fn running(&self) -> bool {
        self.ends_at.is_some()
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        self.ends_at = Some(Instant::now() + self.left(cx));
        self.ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                if this.update(cx, |this, cx| this.tick(cx)).is_err() {
                    break;
                }
            }
        }));
        cx.notify();
    }

    fn pause(&mut self, cx: &mut Context<Self>) {
        self.remaining = Some(self.left(cx));
        self.ends_at = None;
        self.ticker = None;
        cx.notify();
    }

    fn toggle(&mut self, cx: &mut Context<Self>) {
        if self.running() {
            self.pause(cx)
        } else {
            self.start(cx)
        }
    }

    fn set_stage(&mut self, stage: Stage, cx: &mut Context<Self>) {
        self.stage = stage;
        self.remaining = None;
        self.ends_at = None;
        self.ticker = None;
        cx.notify();
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        if !self.left(cx).is_zero() {
            return cx.notify();
        }
        // ponytail: macOS system sound via afplay; swap for rodio if other platforms matter
        let settings = self.settings.read(cx);
        let (volume, auto_start) = (settings.volume(cx), settings.auto_start);
        let _ = Command::new("afplay")
            .args([
                "-v",
                &volume.to_string(),
                "/System/Library/Sounds/Glass.aiff",
            ])
            .spawn();
        if self.stage == Stage::Work {
            self.works_done += 1;
        }
        let next = next_stage(self.stage, self.works_done, self.setting(INTERVAL, cx));
        self.set_stage(next, cx);
        if auto_start {
            self.start(cx);
        }
    }

    fn time_text(&self, cx: &App) -> String {
        let secs = self.left(cx).as_secs_f64().ceil() as u64;
        format!("{:02}:{:02}", secs / 60, secs % 60)
    }
}

impl Render for Pomodoro {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let running = self.running();

        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_6()
            .bg(self.stage.color())
            .text_color(rgb(0xDEDDE2))
            .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
            // Tabs stay in layout (`invisible`) so the timer does not shift when it starts.
            .child(
                h_flex()
                    .gap_3()
                    .when(running, |this| this.invisible())
                    .children(Stage::ALL.map(|stage| {
                        let active = stage == self.stage;
                        div()
                            .id(stage.label())
                            .px_4()
                            .py_1p5()
                            .rounded_full()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .cursor_pointer()
                            .bg(faded(if active { 0.15 } else { 0.04 }))
                            .text_color(if active { faded(0.9) } else { faded(0.5) })
                            .hover(|s| s.bg(faded(0.1)))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |this, _, _, cx| this.set_stage(stage, cx)))
                            .child(stage.label())
                    })),
            )
            .child(
                // Not the system font: SF swaps optical-size outlines at large sizes, but GPUI
                // measures glyph bounds from the small-size outlines, so edges get clipped.
                div()
                    .font_family("Helvetica Neue")
                    .font_weight(FontWeight::MEDIUM)
                    .text_size(px(112.))
                    .line_height(px(120.))
                    .child(self.time_text(cx)),
            )
            .child(
                h_flex()
                    .gap_3()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        icon_button("toggle", if running { PAUSE } else { PLAY }, cx)
                            .on_click(cx.listener(|this, _, _, cx| this.toggle(cx))),
                    )
                    // Removed (not invisible) so the pause button recenters; row height is unchanged.
                    .when(!running, |this| {
                        this.child(
                            h_flex()
                                .gap_3()
                                .child(icon_button("reset", RESET, cx).on_click(cx.listener(
                                    |this, _, _, cx| {
                                        let stage = this.stage;
                                        this.set_stage(stage, cx)
                                    },
                                )))
                                .child(icon_button("more", MORE, cx).on_click(cx.listener(
                                    |this, _, window, cx| this.open_settings(window, cx),
                                ))),
                        )
                    }),
            )
    }
}

fn main() {
    application().with_assets(assets::Assets).run(|cx| {
        init(cx);
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(720.), px(450.)),
                cx,
            ))),
            titlebar: Some(TitlebarOptions {
                title: Some("Pomominders".into()),
                appears_transparent: true,
                traffic_light_position: None,
            }),
            ..Default::default()
        };
        open_window(options, cx, |window, cx| {
            cx.new(|cx| Pomodoro::new(window, cx))
        })
        .expect("failed to open window");
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    use super::{Stage, next_stage};

    #[test]
    fn cycles_to_long_break_every_interval() {
        let mut stage = Stage::Work;
        let mut works = 0;
        let mut seen = vec![];
        for _ in 0..8 {
            if stage == Stage::Work {
                works += 1;
            }
            stage = next_stage(stage, works, 4);
            seen.push(stage);
        }
        use Stage::*;
        assert_eq!(seen, [Short, Work, Short, Work, Short, Work, Long, Work]);
    }
}
