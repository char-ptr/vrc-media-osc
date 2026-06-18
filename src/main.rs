mod media;
mod osc;

use gpui::*;
use gpui_component::{button::*, *};

use crate::{media::MediaPlayer, osc::OscManagement};

#[derive(Debug)]
pub struct HelloWorld {
    mp: Entity<MediaPlayer>,
    oscm: Entity<OscManagement>,
}
impl HelloWorld {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mp = cx.new(|cx| MediaPlayer::new(cx));
        mp.update(cx, |this, cx| {
            this.start_listening(cx);
        });
        let oscm = cx.new(|cx| OscManagement::new(cx, window, &mp));
        Self { mp, oscm }
    }
}

impl Render for HelloWorld {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .v_flex()
            .gap_2()
            .size_full()
            .items_center()
            .justify_center()
            // .child("Hello, World!")
            // .child(
            //     Button::new("ok")
            //         .primary()
            //         .label("Let's Go!")
            //         .on_click(|_, _, _| println!("Clicked!")),
            // )
            .child(self.mp.clone())
            .child(self.oscm.clone())
    }
}

fn init_app(cx: &mut App) {
    let dark = ThemeRegistry::global(cx).default_dark_theme().clone();
    Theme::global_mut(cx).apply_config(&dark);
}
fn main() {
    let app = gpui_platform::application().with_assets(gpui_component_assets::Assets);

    app.run(move |cx| {
        // This must be called before using any GPUI Component features.
        gpui_component::init(cx);
        init_app(cx);

        cx.spawn(async move |cx| {
            cx.open_window(
                WindowOptions {
                    ..Default::default()
                },
                |window, cx| {
                    let view = cx.new(|cx| HelloWorld::new(window, cx));
                    // This first level on the window, should be a Root.
                    cx.new(|cx| Root::new(view, window, cx))
                },
            )
            .expect("Failed to open window");
        })
        .detach();
    });
}
