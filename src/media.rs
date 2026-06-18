use std::{
    sync::{Arc, atomic::AtomicBool},
    thread,
    time::Duration,
};

use eyre::{Context as _, eyre};
use gpui::{Context, IntoElement, ParentElement, Render, Styled, Window, div};
use gpui_component::{Icon, IconName, StyledExt, label::Label};
use mpris::{PlaybackStatus, Player, PlayerFinder, Progress, ProgressTracker};

#[derive(Debug)]
pub struct MediaPlayer {
    shutdown: Arc<AtomicBool>,
    pub last_state: Option<PlayerProgress>,
}
#[derive(Debug, Clone)]
pub struct PlayerProgress {
    pub title: Option<String>,
    pub length: Option<Duration>,
    pub artists: Option<Vec<String>>,
    pub album_name: Option<String>,
    pub status: PlaybackStatus,
    pub position: Duration,
}

impl From<&Progress> for PlayerProgress {
    fn from(prog: &Progress) -> Self {
        let meta = prog.metadata();

        Self {
            title: meta.title().map(|x| x.to_string()),
            length: meta.length(),
            artists: meta
                .artists()
                .map(|x| x.iter().map(|a| a.to_string()).collect()),
            album_name: meta.album_name().map(|x| x.to_string()),
            status: prog.playback_status(),
            position: prog.position(),
        }
    }
}

impl Drop for MediaPlayer {
    fn drop(&mut self) {
        self.shutdown // should work cuz keeps atomic reference alive until all consumers drop so this should tell threads to die!!
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
impl MediaPlayer {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            shutdown: Arc::new(AtomicBool::new(false)),
            last_state: None,
        }
    }
    fn spawn_background(
        sd: Arc<AtomicBool>,
        cx: &mut Context<Self>,
    ) -> async_channel::Receiver<PlayerProgress> {
        let (tx, rx) = async_channel::bounded(5);
        let _ = thread::spawn(move || {
            let mut player: Option<Player> = None;
            let mut tracker: Option<ProgressTracker> = None;
            while sd.load(std::sync::atomic::Ordering::Relaxed) == false {
                if let Some(ref player) = player
                    && player.is_running()
                {
                    if let Ok(mut tracker) = player.track_progress(500) {
                        loop {
                            tracker.force_refresh();
                            let data = tracker.tick();
                            // println!("tracker data = {data:#?}");
                            tx.send_blocking(PlayerProgress::from(data.progress));
                        }
                    }
                } else {
                    player = PlayerFinder::new()
                        .context(eyre::eyre!("unable to get player finder"))
                        .and_then(|pf| pf.find_active().context(eyre!("unable to find active")))
                        .inspect_err(|err| println!("failed to find player : {err}"))
                        .ok();
                }
                thread::sleep(Duration::from_millis(200));
            }
        });
        return rx;
    }
    pub fn start_listening(&mut self, cx: &mut Context<Self>) {
        let mut rx = Self::spawn_background(self.shutdown.clone(), cx);
        let sd = self.shutdown.clone();
        cx.spawn(async move |this, mut cx| {
            while sd.load(std::sync::atomic::Ordering::Relaxed) == false {
                // println!(" rx_closed = {}", rx.is_closed());
                let event = rx.recv().await;
                match event {
                    Ok(data) => {
                        // println!("got event data : {data:#?}");
                        this.update(cx, |this, cx| {
                            this.last_state = Some(data);
                            cx.notify();
                        });
                    }
                    Err(e) => eprintln!("failed to get event data : {e:?}"),
                }
            }
        })
        .detach();
    }
}
impl PlayerProgress {
    fn render(&mut self) -> impl IntoElement {
        let song_prog = self.position.as_millis() as f32
            / self.length.unwrap_or(Duration::from_secs(1)).as_millis() as f32;
        div()
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(
                        Icon::new(match self.status {
                            PlaybackStatus::Paused => IconName::Pause,
                            PlaybackStatus::Playing => IconName::Play,
                            PlaybackStatus::Stopped => IconName::CircleX,
                        })
                        .mr_2(),
                    )
                    .child("listening to ")
                    .child(
                        Label::new(self.title.clone().unwrap_or("not found".to_string()))
                            .font_bold(),
                    )
                    .child(" by ")
                    .child(
                        Label::new(
                            self.artists
                                .clone()
                                .unwrap_or(vec!["no artists found?".to_string()])
                                .join(", "),
                        )
                        .font_bold(),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(self.position.as_secs_f32().floor().to_string())
                    .child(
                        gpui_component::progress::Progress::new("song-progress")
                            .value(song_prog * 100f32),
                    )
                    .child(
                        self.length
                            .map(|x| x.as_secs_f32())
                            .unwrap_or(1f32)
                            .floor()
                            .to_string(),
                    ), // .child(Label::new(song_prog.to_string()).text_color(green_300())),
            )
    }
}
impl Render for MediaPlayer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(ref pp) = self.last_state else {
            return div().child("no playing :<");
        };
        return div().child(pp.clone().render());
    }
}
