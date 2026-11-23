# Pomominders

A minimal pomodoro timer for macOS, built with Rust and [GPUI](https://www.gpui.rs).

- Work, short break and long break stages, each with its own color
- Start/pause, reset, and a settings dialog (durations, long break interval, auto start, alarm volume, notifications)
- Plays a chime when a stage ends, with a toast in the top-right corner that shows even under Focus modes; with auto start on, its Start button counts down before starting the next stage

Settings are saved to `~/Library/Application Support/pomominders/settings.toml`.

## Build

Requires Rust (stable) and Xcode.

```sh
cargo run            # run in dev mode
cargo test
```

To build `Pomominders.app`:

```sh
cargo install cargo-bundle
cargo bundle --release
# -> target/release/bundle/osx/Pomominders.app
```

The app is ad hoc signed only, so on another Mac you'll need to right-click → Open the first time.

## Shortcuts

- `⌘,` settings
- `⌘Q` quit
