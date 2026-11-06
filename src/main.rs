use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::dialog::{DialogClose, DialogFooter};
use gpui_kit::component::input::{InputEvent, InputState, NumberInput};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, Icon, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde::{Deserialize, Serialize};

actions!(pomominders, [Quit, OpenSettings]);

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

const INTRO: Duration = Duration::from_millis(2400);
const SPRING: SpringConfig = SpringConfig::new(500., 30., 1.);
/// Critically damped: for things that appear and disappear, where overshoot would wobble.
/// How long the mouse rests before the pause button fades out.
const IDLE: Duration = Duration::from_secs(3);
const SETTLE: SpringConfig = SpringConfig::new(260., 32., 1.);

/// Startup grain; resvg rasterizes it once and the overlay only fades.
const GRAIN: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="720" height="450"><filter id="n"><feTurbulence type="fractalNoise" baseFrequency=".85" numOctaves="2" stitchTiles="stitch"/><feColorMatrix type="saturate" values="0"/></filter><rect width="100%" height="100%" filter="url(#n)"/></svg>"##;

/// Eased 0..1 progress of the intro for a section starting `delay` (fraction of INTRO) in.
fn intro_progress(t: f32, delay: f32) -> f32 {
    let x = ((t - delay) / (1. - delay)).clamp(0., 1.);
    1. - (1. - x).powi(3)
}

/// Hand-off from the blurred stand-in to the real element over the last quarter.
fn focus(p: f32) -> f32 {
    let x = ((p - 0.75) / 0.25).clamp(0., 1.);
    x * x * (3. - 2. * x)
}

/// Startup blur-in. `blurred(sigma)` builds a stand-in laid out like `content` but drawn
/// with a gaussian blur; it sharpens, then cross-fades to the real (interactive) element.
/// `scrub` pins the intro timeline (debug panel).
fn reveal<E: IntoElement>(
    id: &'static str,
    delay: f32,
    radius: f32,
    scrub: Option<f32>,
    content: impl IntoElement,
    blurred: impl Fn(f32) -> E + 'static,
) -> Div {
    div()
        .relative()
        .child(div().child(content).with_animation(
            (id, 0usize),
            Animation::new(INTRO),
            move |el, t| el.opacity(focus(intro_progress(scrub.unwrap_or(t), delay))),
        ))
        .child(div().absolute().inset_0().with_animation(
            (id, 1usize),
            Animation::new(INTRO),
            move |el, t| {
                let p = intro_progress(scrub.unwrap_or(t), delay);
                let f = focus(p);
                el.opacity((p * 3.).min(1.) * (1. - f))
                    .when(f < 1., |el| el.child(blurred(radius * (1. - p))))
            },
        ))
}

/// Gaussian-blurred SVG, rasterized by resvg since GPUI has no element filters.
/// `body` is centered on the origin in units of `1/scale` px; the result is a `w`×`h`
/// box centered on its (relative) parent, tinted by `text_color` like any `svg()`.
fn gaussian(body: &str, w: f32, h: f32, scale: f32, sigma: f32) -> Svg {
    // resvg rasterizes on the main thread mid-animation, at 2x the box size (~200ms for
    // the timer). Blur hides detail, so rasterize a box 1/d the size (d ~ sigma) and let
    // the GPU scale it back up: cost falls with d². At least 4, since even d = 2 drops
    // frames; the coarse low-sigma rasters only show while fading out under sharp text.
    let d = 2f32.powi(sigma.max(4.).log2().floor() as i32);
    // Quantized per raster px: each distinct SVG is rasterized once and cached.
    let sigma = (sigma / d * 2.).round() / 2. * d;
    let doc = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{} {} {} {}"><filter id="b" x="-1" y="-1" width="3" height="3"><feGaussianBlur stdDeviation="{}"/></filter><g filter="url(#b)">{}</g></svg>"##,
        -w / 2. / scale,
        -h / 2. / scale,
        w / scale,
        h / scale,
        sigma / scale,
        body
    );
    svg()
        .data(doc.as_bytes())
        .absolute()
        .left(relative(0.5))
        .top(relative(0.5))
        .ml(px(-w / 2. / d))
        .mt(px(-h / 2. / d))
        .w(px(w / d))
        .h(px(h / d))
        .with_transformation(Transformation::scale(size(d, d)))
}

/// SVG text centered on the origin. GPUI centers the ascent+descent box in the line,
/// which puts the baseline ~0.379em below center for both fonts used here.
fn svg_text(text: &str, family: &str, size: f32) -> String {
    format!(
        r#"<text y="{}" text-anchor="middle" font-family="{family}" font-weight="500" font-size="{size}">{text}</text>"#,
        0.379 * size
    )
}

/// Clickable element whose hover/press feed a spring target: 0 idle, 1 hovered, 2 pressed.
/// GPUI's `.hover()`/`.active()` styles snap, so the state is tracked by hand.
fn pressable(id: &'static str, window: &mut Window, cx: &mut App) -> (Stateful<Div>, f32) {
    let state = window.use_keyed_state((id, 0usize), cx, |_, _| (false, false));
    let (hovered, pressed) = *state.read(cx);
    let set = |f: fn(&mut (bool, bool))| {
        let state = state.clone();
        move |cx: &mut App| {
            state.update(cx, |s, cx| {
                f(s);
                cx.notify()
            })
        }
    };
    let (down, up, up_out) = (
        set(|s| s.1 = true),
        set(|s| s.1 = false),
        set(|s| s.1 = false),
    );
    let hover = state.clone();
    let el = div()
        .id(id)
        .cursor_pointer()
        .on_hover(move |&h, _, cx| {
            hover.update(cx, |s, cx| {
                s.0 = h;
                cx.notify()
            })
        })
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            cx.stop_propagation();
            down(cx)
        })
        .on_mouse_up(MouseButton::Left, move |_, _, cx| up(cx))
        .on_mouse_up_out(MouseButton::Left, move |_, _, cx| up_out(cx));
    (
        el,
        if pressed {
            2.
        } else if hovered {
            1.
        } else {
            0.
        },
    )
}

/// Splits a `pressable` spring value into (hover, press) amounts.
fn hover_press(v: f32) -> (f32, f32) {
    (v.clamp(0., 1.), (v - 1.).max(0.))
}

const BUTTON: f32 = 48.;
/// Width of the reset and more buttons beside the toggle, gaps included.
const EXTRAS: f32 = 2. * (12. + BUTTON);

fn icon(svg: &'static [u8]) -> Icon {
    Icon::default().data(svg).size(px(BUTTON * 0.75))
}

/// Blurred, non-interactive stand-in for an `icon_button`.
fn blurred_icon(svg: &'static [u8], sigma: f32) -> Div {
    // Nest the icon document as a 24-unit box centered on the origin.
    let doc = std::str::from_utf8(svg).unwrap();
    let body = doc.replacen(
        "<svg ",
        r#"<svg x="-12" y="-12" width="24" height="24" "#,
        1,
    );
    div().relative().size(px(BUTTON)).child(
        gaussian(&body, BUTTON * 2., BUTTON * 2., BUTTON * 0.75 / 24., sigma)
            .text_color(faded(0.5)),
    )
}

fn icon_button(
    id: &'static str,
    svg: &'static [u8],
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let (el, target) = pressable(id, window, cx);
    el.size(px(BUTTON))
        .flex()
        .items_center()
        .justify_center()
        .on_click(on_click)
        .with_spring(id, SpringAnimation::new(SPRING).to(target), move |el, v| {
            let (hover, press) = hover_press(v);
            let scale = 1. - 0.08 * press;
            el.child(
                div()
                    .absolute()
                    .top(px(BUTTON / 2. * (1. - scale)))
                    .left(px(BUTTON / 2. * (1. - scale)))
                    .size(px(BUTTON * scale))
                    .rounded_full()
                    .bg(faded(0.08 * hover + 0.07 * press)),
            )
            .child(
                icon(svg)
                    .text_color(faded(0.5 + 0.35 * hover))
                    .transform(Transformation::scale(size(scale, scale))),
            )
        })
        .into_any_element()
}

/// (background, text) alpha of a tab pill.
fn pill_alpha(active: bool, hover: f32, press: f32) -> (f32, f32) {
    if active {
        (0.15, 0.9)
    } else {
        (0.04 + 0.06 * hover + 0.05 * press, 0.5 + 0.25 * hover)
    }
}

fn pill(stage: Stage, active: bool, hover: f32, press: f32) -> Div {
    let (bg, fg) = pill_alpha(active, hover, press);
    div()
        .px_4()
        .py_1p5()
        .rounded_full()
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .bg(faded(bg))
        .text_color(faded(fg))
        .child(stage.label())
}

/// Blurred, non-interactive stand-in for a `pill`: the background is a native
/// (gaussian) shadow, the label a blurred SVG over an invisible layout copy.
fn blurred_pill(stage: Stage, active: bool, sigma: f32) -> Div {
    let (bg, fg) = pill_alpha(active, 0., 0.);
    pill(stage, active, 0., 0.)
        .relative()
        .bg(transparent_black())
        .text_color(transparent_black())
        .shadow(vec![BoxShadow {
            color: faded(bg),
            offset: point(px(0.), px(0.)),
            blur_radius: px(sigma * 2.),
            spread_radius: px(0.),
            inset: false,
        }])
        .child(
            gaussian(
                &svg_text(stage.label(), "SF Pro Text, Helvetica Neue", 14.),
                240.,
                96.,
                1.,
                sigma,
            )
            .text_color(faded(fg)),
        )
}

const TIMER_FONT: &str = "Helvetica Neue";
const TIMER_SIZE: f32 = 112.;

fn blurred_timer(text: String, sigma: f32) -> Div {
    timer(text.clone())
        .relative()
        .text_color(transparent_black())
        .child(
            gaussian(
                &svg_text(&text, TIMER_FONT, TIMER_SIZE),
                560.,
                260.,
                1.,
                sigma,
            )
            .text_color(rgb(0xDEDDE2)),
        )
}

fn timer(text: String) -> Div {
    // Not the system font: SF swaps optical-size outlines at large sizes, but GPUI
    // measures glyph bounds from the small-size outlines, so edges get clipped.
    div()
        .font_family(TIMER_FONT)
        .font_weight(FontWeight::MEDIUM)
        .text_size(px(TIMER_SIZE))
        .line_height(px(120.))
        .child(text)
}

/// How far a digit travels when it rolls in or out.
const ROLL: f32 = 36.;

/// The timer with each character in its own slot, so a changed digit rolls: the old
/// one slides out and fades while the new one slides in. `down` sets the direction.
/// Shift that centers a digit's ink in its slot. Helvetica Neue's figures are tabular,
/// but its narrow "1" sits 0.076em left of center, leaving a gap before the colon.
fn ink_shift(c: char) -> Pixels {
    px(if c == '1' { 0.076 * TIMER_SIZE } else { 0. })
}

fn rolling_timer(slots: &[Slot], down: bool) -> Div {
    let dir = if down { 1. } else { -1. };
    timer(String::new())
        .flex()
        .children(slots.iter().enumerate().map(|(i, slot)| {
            let ch = slot.shown.to_string();
            let shift = ink_shift(slot.shown);
            if slot.rolls == 0 {
                return div().relative().left(shift).child(ch).into_any_element();
            }
            let id = |part| SharedString::from(format!("digit-{part}-{i}-{}", slot.rolls));
            let roll = || Animation::new(Duration::from_millis(450)).with_easing(ease_out_quint());
            div()
                .relative()
                .child(div().relative().left(shift).child(ch).with_animation(
                    id("in"),
                    roll(),
                    move |el, t| el.top(px(-dir * ROLL * (1. - t))).opacity(t),
                ))
                .child(
                    div()
                        .absolute()
                        .left(ink_shift(slot.previous))
                        .child(slot.previous.to_string())
                        .with_animation(id("out"), roll(), move |el, t| {
                            el.top(px(dir * ROLL * t)).opacity((1. - t * 1.6).max(0.))
                        }),
                )
                .into_any_element()
        }))
}

/// Diffs `time` against the shown slots, starting a roll for each changed character.
/// Returns whether it rolls down, or `None` if nothing rolls.
fn roll(slots: &mut Vec<Slot>, time: &str) -> Option<bool> {
    if slots.len() != time.len() {
        // First render (or minutes gained a digit): show as-is, nothing to roll from.
        *slots = time
            .chars()
            .map(|c| Slot {
                shown: c,
                previous: c,
                rolls: 0,
            })
            .collect();
        return None;
    }
    let shown: String = slots.iter().map(|s| s.shown).collect();
    if shown == time {
        return None;
    }
    for (slot, c) in slots.iter_mut().zip(time.chars()) {
        if slot.shown != c {
            *slot = Slot {
                shown: c,
                previous: slot.shown,
                rolls: slot.rolls + 1,
            };
        }
    }
    // Same "MM:SS" shape, so string order is time order: counting down rolls down.
    Some(time < shown.as_str())
}

/// One character of the timer, plus what it showed before its last change.
#[derive(Clone, Copy)]
struct Slot {
    shown: char,
    previous: char,
    /// Bumped on every change; restarts the roll animation.
    rolls: usize,
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

/// Settings on disk: `<config dir>/pomominders/settings.toml`, where the config dir is
/// ~/Library/Application Support (macOS), $XDG_CONFIG_HOME or ~/.config (Linux), %APPDATA% (Windows).
/// Missing keys fall back to defaults; an unreadable file is ignored.
#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    work_minutes: u64,
    short_break_minutes: u64,
    long_break_minutes: u64,
    long_break_interval: u64,
    auto_start: bool,
    /// 0..=100
    alarm_volume: f32,
}

impl Default for Saved {
    fn default() -> Self {
        Self::new(SETTINGS.map(|(_, default)| default), false, 100.)
    }
}

impl Saved {
    /// `values` are indexed like `SETTINGS`.
    fn new([work, short, long, interval]: [u64; 4], auto_start: bool, alarm_volume: f32) -> Self {
        Self {
            work_minutes: work,
            short_break_minutes: short,
            long_break_minutes: long,
            long_break_interval: interval,
            auto_start,
            alarm_volume,
        }
    }

    fn values(&self) -> [u64; 4] {
        [
            self.work_minutes,
            self.short_break_minutes,
            self.long_break_minutes,
            self.long_break_interval,
        ]
    }

    fn path() -> Option<PathBuf> {
        Some(
            dirs::config_dir()?
                .join("pomominders")
                .join("settings.toml"),
        )
    }

    fn load() -> Self {
        Self::path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    // ponytail: plain overwrite, not atomic; a crash mid-write just resets to defaults.
    fn store(&self) {
        let Some(path) = Self::path() else { return };
        let result = std::fs::create_dir_all(path.parent().unwrap())
            .and_then(|_| std::fs::write(&path, toml::to_string(self).unwrap()));
        if let Err(err) = result {
            eprintln!("failed to save settings to {}: {err}", path.display());
        }
    }
}

/// Rendered inside the settings dialog; its own view so controls re-render while it's open.
struct Settings {
    inputs: [Entity<InputState>; 4],
    auto_start: bool,
    volume: Entity<SliderState>,
}

impl Settings {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let saved = Saved::load();
        let inputs = saved.values().map(|value| {
            let state = cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(value.to_string())
                    .step(1.)
                    .min(1.)
                    .max(999.)
            });
            cx.subscribe(&state, |this, _, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    this.save(cx);
                }
                cx.notify();
            })
            .detach();
            state
        });
        let volume = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(100.)
                .step(1.)
                .default_value(saved.alarm_volume.clamp(0., 100.))
        });
        cx.subscribe(&volume, |this, _, _: &SliderEvent, cx| {
            this.save(cx);
            cx.notify();
        })
        .detach();
        Self {
            inputs,
            auto_start: saved.auto_start,
            volume,
        }
    }

    fn save(&self, cx: &App) {
        Saved::new(
            std::array::from_fn(|i| self.get(i, cx)),
            self.auto_start,
            self.volume.read(cx).value().end(),
        )
        .store();
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
                                this.save(cx);
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
    /// Stage being cross-faded away from, and a counter that restarts the fade.
    from: Stage,
    switches: usize,
    grain: Arc<Image>,
    /// Timer characters as last rendered, and whether the latest change rolled down.
    slots: Vec<Slot>,
    rolls_down: bool,
    /// Last mouse move over the window; the pause button hides after `IDLE` without one.
    moved_at: Instant,
    mouse: Option<Point<Pixels>>,
    /// Paused time left; `None` means the stage hasn't started yet.
    remaining: Option<Duration>,
    ends_at: Option<Instant>,
    works_done: u64,
    settings: Entity<Settings>,
    ticker: Option<Task<()>>,
    /// Debug panel (debug builds only): scrubs the intro; `None` until the slider moves.
    timeline: Entity<SliderState>,
    scrub: Option<f32>,
}

impl Pomodoro {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = cx.new(|cx| Settings::new(window, cx));
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();
        let timeline = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(1.)
                .step(0.001)
                .default_value(1.)
        });
        cx.subscribe(&timeline, |this, slider, _: &SliderEvent, cx| {
            this.scrub = Some(slider.read(cx).value().end());
            cx.notify();
        })
        .detach();
        Self {
            stage: Stage::Work,
            from: Stage::Work,
            switches: 0,
            grain: Arc::new(Image::from_bytes(ImageFormat::Svg, GRAIN.to_vec())),
            slots: Vec::new(),
            rolls_down: true,
            moved_at: Instant::now(),
            mouse: None,
            remaining: None,
            ends_at: None,
            works_done: 0,
            settings,
            ticker: None,
            timeline,
            scrub: None,
        }
    }

    fn setting(&self, i: usize, cx: &App) -> u64 {
        self.settings.read(cx).get(i, cx)
    }

    fn open_settings(&self, window: &mut Window, cx: &mut App) {
        if window.has_active_dialog(cx) {
            return;
        }
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

    /// Running with no recent mouse move: the ticker's re-renders pick this up.
    fn idle(&self) -> bool {
        self.running() && self.moved_at.elapsed() > IDLE
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        self.ends_at = Some(Instant::now() + self.left(cx));
        // Idle counts from starting, so the pause button shows briefly before fading.
        self.moved_at = Instant::now();
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
        self.from = self.stage;
        self.switches += 1;
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (running, stage) = (self.running(), self.stage);
        let toggle_svg = if running { PAUSE } else { PLAY };

        let tabs = h_flex().gap_3().children(Stage::ALL.map(|s| {
            let (el, target) = pressable(s.label(), window, cx);
            // Guarded: the tabs stay clickable while they fade out.
            el.on_click(cx.listener(move |this, _, _, cx| {
                if !this.running() {
                    this.set_stage(s, cx)
                }
            }))
            .with_spring(
                s.label(),
                SpringAnimation::new(SPRING).to(target),
                move |el, v| {
                    let (hover, press) = hover_press(v);
                    el.child(pill(s, s == stage, hover, press))
                },
            )
        }));

        let toggle = icon_button(
            "toggle",
            toggle_svg,
            cx.listener(|this, _, _, cx| this.toggle(cx)),
            window,
            cx,
        );
        // Guarded like the tabs.
        let reset = icon_button(
            "reset",
            RESET,
            cx.listener(|this, _, _, cx| {
                if !this.running() {
                    this.set_stage(this.stage, cx)
                }
            }),
            window,
            cx,
        );
        let more = icon_button(
            "more",
            MORE,
            cx.listener(|this, _, window, cx| {
                if !this.running() {
                    this.open_settings(window, cx)
                }
            }),
            window,
            cx,
        );
        // 1 while paused, 0 while running: drives hiding the tabs and extras.
        let shown = if running { 0. } else { 1. };
        // While running, the extras collapse to zero width so the toggle glides to center.
        let extras = div()
            .child(
                h_flex()
                    .flex_shrink_0()
                    .pl_3()
                    .gap_3()
                    .child(reset)
                    .child(more),
            )
            .with_spring("extras", SpringAnimation::new(SETTLE).to(shown), |el, v| {
                let v = v.clamp(0., 1.);
                el.w(px(EXTRAS * v))
                    .opacity(v)
                    .when(v < 0.01, |el| el.invisible())
            });

        let time = self.time_text(cx);
        if let Some(down) = roll(&mut self.slots, &time) {
            self.rolls_down = down;
        }
        let scrub = self.scrub;
        v_flex()
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                // Only real motion counts: skip repeats at the same spot, and the first
                // event, which just reports where the cursor already was.
                if this
                    .mouse
                    .replace(event.position)
                    .is_none_or(|p| p == event.position)
                {
                    return;
                }
                let was_idle = this.idle();
                this.moved_at = Instant::now();
                if was_idle {
                    cx.notify();
                }
            }))
            .relative()
            .size_full()
            .items_center()
            .justify_center()
            .gap_6()
            .bg(self.from.color())
            .text_color(rgb(0xDEDDE2))
            .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
            .child(div().absolute().inset_0().bg(stage.color()).with_animation(
                ("fade", self.switches),
                Animation::new(Duration::from_millis(450)).with_easing(ease_in_out),
                |el, t| el.opacity(t),
            ))
            // Tabs fade but stay in layout so the timer does not shift when it starts.
            .child(
                reveal("intro-tabs", 0., 8., scrub, tabs, move |sigma| {
                    h_flex()
                        .gap_3()
                        .children(Stage::ALL.map(|s| blurred_pill(s, s == stage, sigma)))
                })
                .with_spring(
                    "tabs",
                    SpringAnimation::new(SETTLE).to(shown),
                    |el, v| {
                        let v = v.clamp(0., 1.);
                        el.opacity(v).when(v < 0.01, |el| el.invisible())
                    },
                ),
            )
            .child(reveal(
                "intro-timer",
                0.1,
                16.,
                scrub,
                rolling_timer(&self.slots, self.rolls_down),
                move |sigma| blurred_timer(time.clone(), sigma),
            ))
            .child(reveal(
                "intro-buttons",
                0.2,
                8.,
                scrub,
                h_flex()
                    .child(div().child(toggle).with_spring(
                        "toggle-idle",
                        SpringAnimation::new(SETTLE).to(if self.idle() { 0. } else { 1. }),
                        |el, v| el.opacity(v.clamp(0., 1.)),
                    ))
                    .child(extras),
                move |sigma| {
                    h_flex()
                        .gap_3()
                        .child(blurred_icon(toggle_svg, sigma))
                        .when(!running, |el| {
                            el.child(blurred_icon(RESET, sigma))
                                .child(blurred_icon(MORE, sigma))
                        })
                },
            ))
            .child(
                img(self.grain.clone())
                    .absolute()
                    .inset_0()
                    .size_full()
                    .object_fit(ObjectFit::Fill)
                    .with_animation("intro-grain", Animation::new(INTRO), move |el, t| {
                        let o = 0.3 * (1. - scrub.unwrap_or(t)).powi(2);
                        el.opacity(o).when(o < 0.001, |el| el.invisible())
                    }),
            )
            .when(cfg!(debug_assertions), |el| {
                el.child(gpui_fps::fps_monitor(window, cx)).child(
                    v_flex()
                        .absolute()
                        .bottom_3()
                        .left_3()
                        .w(px(200.))
                        .gap_2()
                        .p_3()
                        .rounded_lg()
                        .bg(hsla(0., 0., 0., 0.35))
                        .text_xs()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            h_flex()
                                .justify_between()
                                .child("Intro timeline")
                                .child(format!(
                                    "{:.0} ms",
                                    scrub.unwrap_or(1.) * INTRO.as_millis() as f32
                                )),
                        )
                        .child(Slider::new(&self.timeline)),
                )
            })
    }
}

/// A bundled .app gets its icon from Info.plist; this covers unbundled runs (`cargo run`).
#[cfg(target_os = "macos")]
fn set_dock_icon() {
    use objc2::{AllocAnyThread as _, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let data = NSData::with_bytes(include_bytes!("../assets/icon.png"));
    if let (Some(mtm), Some(image)) = (
        MainThreadMarker::new(),
        NSImage::initWithData(NSImage::alloc(), &data),
    ) {
        unsafe { NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image)) };
    }
}

fn main() {
    application().with_assets(assets::Assets).run(|cx| {
        init(cx);
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-,", OpenSettings, None),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.set_menus([Menu::new("Pomominders").items([
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::separator(),
            MenuItem::action("Quit Pomominders", Quit),
        ])]);
        #[cfg(target_os = "macos")]
        set_dock_icon();
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
        let (handle, view) = open_window(options, cx, |window, cx| {
            cx.new(|cx| Pomodoro::new(window, cx))
        })
        .expect("failed to open window");
        // App-level so the menu item stays enabled with nothing focused. Deferred because
        // global actions run while the window is mid-update, so it can't be borrowed here.
        cx.on_action(move |_: &OpenSettings, cx| {
            let view = view.clone();
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, cx| {
                    view.update(cx, |this, cx| this.open_settings(window, cx))
                });
            });
        });
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    use super::{Saved, Stage, next_stage, roll};

    #[test]
    fn rolls_only_changed_digits_in_time_direction() {
        let mut slots = Vec::new();
        assert_eq!(roll(&mut slots, "25:00"), None);
        assert_eq!(roll(&mut slots, "25:00"), None);

        assert_eq!(roll(&mut slots, "24:59"), Some(true));
        let rolled: Vec<_> = slots
            .iter()
            .map(|s| (s.previous, s.shown, s.rolls))
            .collect();
        assert_eq!(
            rolled,
            [
                ('2', '2', 0),
                ('5', '4', 1),
                (':', ':', 0),
                ('0', '5', 1),
                ('0', '9', 1)
            ]
        );

        assert_eq!(roll(&mut slots, "25:00"), Some(false));
        assert_eq!(slots[1].rolls, 2);
    }

    #[test]
    fn saved_settings_round_trip_and_fill_missing_keys() {
        let saved = Saved::new([50, 10, 20, 3], true, 40.);
        let back: Saved = toml::from_str(&toml::to_string(&saved).unwrap()).unwrap();
        assert_eq!(
            (back.values(), back.auto_start, back.alarm_volume),
            ([50, 10, 20, 3], true, 40.)
        );

        let partial: Saved = toml::from_str("work_minutes = 30").unwrap();
        assert_eq!(partial.values(), [30, 5, 15, 4]);
    }

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
