mod db;
mod text_input;

use gpui::{
    App, Application, Bounds, Context, Entity, Focusable, KeyBinding, SharedString,
    TitlebarOptions, Window, WindowBounds, WindowOptions, actions, div, prelude::*, px, rgb, size,
};

use db::{Db, Issue};
use text_input::{Submit, TextInput};

const APP_NAME: &str = "Solilog";

actions!(solilog, [Quit]);

struct Solilog {
    db: Db,
    input: Entity<TextInput>,
    /// Newest first.
    issues: Vec<Issue>,
}

impl Solilog {
    fn new(db: Db, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextInput::new("New issue…", cx));
        cx.subscribe(&input, |this, input, _: &Submit, cx| {
            let title = input.read(cx).text().trim().to_string();
            if title.is_empty() {
                return;
            }
            match this.db.create_issue(&title) {
                Ok(issue) => {
                    this.issues.insert(0, issue);
                    input.update(cx, |input, cx| input.clear(cx));
                    cx.notify();
                }
                // Keep the text in the input so nothing is lost.
                Err(err) => eprintln!("failed to create issue: {err:#}"),
            }
        })
        .detach();

        window.focus(&input.read(cx).focus_handle(cx));

        let issues = db.list_issues().unwrap_or_else(|err| {
            eprintln!("failed to load issues: {err:#}");
            Vec::new()
        });

        Self { db, input, issues }
    }
}

impl Render for Solilog {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x1c1c1f))
            .text_color(rgb(0xe6e6e6))
            .text_size(px(14.))
            .line_height(px(20.))
            .child(
                div()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(rgb(0x2c2c31))
                    .child(self.input.clone()),
            )
            .child(
                div()
                    .id("issues")
                    .flex_1()
                    .flex()
                    .flex_col()
                    .overflow_y_scroll()
                    .py_1()
                    .children(self.issues.iter().map(|issue| {
                        div()
                            .flex()
                            .gap_3()
                            .px_4()
                            .py_1p5()
                            .child(
                                div()
                                    .w(px(48.))
                                    .text_color(rgb(0x7a7a85))
                                    .child(format!("SOL-{}", issue.id)),
                            )
                            .child(issue.title.clone())
                    })),
            )
    }
}

fn main() -> anyhow::Result<()> {
    let db = Db::open_default()?;

    Application::new().run(move |cx: &mut App| {
        text_input::bind_keys(cx);
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.on_action(|_: &Quit, cx| cx.quit());

        let bounds = Bounds::centered(None, size(px(1000.), px(700.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(SharedString::from(APP_NAME)),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Solilog::new(db, window, cx)),
        )
        .unwrap();

        // Quit the app when the last window is closed.
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        cx.activate(true);
    });

    Ok(())
}
