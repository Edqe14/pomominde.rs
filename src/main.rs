use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;

struct Pomominders;

impl Render for Pomominders {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .child("Pomominders")
            .child(
                Button::new("start")
                    .primary()
                    .label("Start")
                    .on_click(|_, _, _| println!("Start")),
            )
    }
}

fn main() {
    application().with_assets(assets::Assets).run(|cx| {
        init(cx);

        open_window(WindowOptions::default(), cx, |_, cx| {
            cx.new(|_| Pomominders)
        })
        .expect("failed to open window");
    });
}
