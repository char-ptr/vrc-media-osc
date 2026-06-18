use std::{
    error::Error,
    net::Ipv4Addr,
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering::Relaxed},
    },
    time::Duration,
};

use async_channel::bounded;
use async_io::Timer;
use async_net::UdpSocket;
use eyre::{Context as _, Result, eyre};
use futures::{FutureExt, StreamExt};
use futures_lite::FutureExt as _;
use gpui::{AppContext, Context, Entity, ParentElement, Render, Styled, div};
use gpui_component::{
    button::Button,
    input::{Input, InputState},
    *,
};
use human_repr::HumanDuration;
use rosc::{OscMessage, OscPacket, OscType};

use crate::media::{MediaPlayer, PlayerProgress};
#[derive(Debug)]
enum OscConnState {
    Connecting,
    Connected,
    Closed(Option<Box<dyn Error + Send>>),
}
struct OscConnection {
    shutdown: Arc<AtomicBool>,
    state: OscConnState,
}
impl Drop for OscConnection {
    fn drop(&mut self) {
        self.shutdown.store(true, Relaxed);
    }
}
impl OscConnection {
    pub fn new(cx: &mut Context<Self>, addr: String, mp: Entity<MediaPlayer>) -> Self {
        let shutdown = Arc::new(AtomicBool::new(false));
        Self::background(cx, shutdown.clone(), mp, addr);
        Self {
            shutdown,
            state: OscConnState::Connecting,
        }
    }

    fn background(
        cx: &mut Context<Self>,
        shutdown: Arc<AtomicBool>,
        mp: Entity<MediaPlayer>,
        addr: String,
    ) {
        let (tx, rx) = bounded::<Option<PlayerProgress>>(5);
        let (tx_state, rx_state) = bounded::<OscConnState>(5);
        let shutdown_clone = shutdown.clone();
        cx.background_spawn(async move {
            let sock = match UdpSocket::bind("0.0.0.0:0").await {
                Ok(sock) => sock,
                Err(bind_err) => {
                    tx_state
                        .send(OscConnState::Closed(Some(Box::new(bind_err))))
                        .await;
                    return;
                }
            };

            if let Err(conn_err) = sock.connect(addr).await {
                tx_state
                    .send(OscConnState::Closed(Some(Box::new(conn_err))))
                    .await;
                return;
            }
            println!("connected");
            let mut con = false;
            while shutdown.load(std::sync::atomic::Ordering::Relaxed) == false {
                let event = rx
                    .recv()
                    .map(|fut| fut.context(eyre!("recv err")))
                    .or(async {
                        Timer::after(Duration::from_millis(200)).await;
                        Err(eyre!("timeout"))
                    })
                    .await;
                match event {
                    Ok(None) => {
                        println!("got event but no data");
                    }
                    Ok(Some(data)) => {
                        println!("got event data : {data:#?}");

                        let count = data.position.as_millis() as f32
                            / data.length.map(|x| x.as_millis()).unwrap_or(1000) as f32;

                        println!("{count} count");
                        let data_string = format!(
                            "♪ {}{} [{}] by {:?} \n{:▒<10}\n❤️ MPRIS",
                            match data.status {
                                mpris::PlaybackStatus::Playing => "▶︎",
                                mpris::PlaybackStatus::Paused => "⏸",
                                mpris::PlaybackStatus::Stopped => "⏸",
                            },
                            data.title.unwrap_or("nothing".to_string()),
                            data.album_name.unwrap_or("".to_string()),
                            data.artists.unwrap_or(vec![]),
                            "█".repeat((count * 10f32).ceil() as usize)
                        );

                        let packet = OscPacket::Message(OscMessage {
                            addr: "/chatbox/input".to_string(),
                            args: vec![OscType::String(data_string), OscType::Bool(true)],
                        });
                        let Ok(encoded) = rosc::encoder::encode(&packet) else {
                            eprintln!("failed to encode packet");
                            continue;
                        };
                        println!("encoded okay going to send!!");
                        match sock.send(&encoded).await {
                            Ok(bytes) => {
                                println!("okay sent {} bytes!!", bytes);
                                if !con {
                                    tx_state.send(OscConnState::Connected).await;
                                }
                                con = true;
                            }
                            Err(err) => {
                                println!("socket disconnected");
                                tx_state
                                    .send(OscConnState::Closed(Some(Box::new(err))))
                                    .await;

                                break;
                            }
                        }
                    }
                    Err(e) => {}
                }
            }

            tx_state.send(OscConnState::Closed(None)).await;
            shutdown.store(true, Relaxed);
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let meow = mp.clone();
            loop {
                let tx_cloned = tx.clone();
                if tx_cloned.is_closed() {
                    println!("tx closed");
                    break;
                }
                let new_data = cx.read_entity(&meow, move |this, cx| {
                    let state = this.last_state.clone();
                    cx.background_spawn(async move {
                        tx_cloned.send(state).await;
                    })
                });

                let mut timer = cx
                    .background_executor()
                    .timer(Duration::from_secs(2))
                    .fuse();
                futures::select! {
                    _ = timer => {},
                    state = rx_state.recv().fuse() => {
                        println!("got state {state:#?}");
                        let Ok(state) = state else {
                            // closed so we might as well kill ourselds
                            this.update(cx, move |this, cx| {
                                this.state = OscConnState::Closed(None);
                                cx.notify()
                            });
                            break;
                        };

                        println!("got osc state = {state:?}");
                        let stop = match state { OscConnState::Closed(_) => true,_ => false,  };
                        this.update(cx, move |this, cx| {
                            this.state = state;

                            cx.notify()
                        });
                        if stop {break}
                    }
                }
                // if shutdown_clone.load(Relaxed) {
                //     println!("shutdown");
                //     this.update(cx, move |this, cx| {
                //         this.state = OscConnState::Closed(None);
                //         cx.notify()
                //     });
                //     break;
                // }
            }
        })
        .detach();
    }
}
pub struct OscManagement {
    addr_inp: Entity<InputState>,
    mp: Entity<MediaPlayer>,
    conn: Option<Entity<OscConnection>>,
}
impl OscManagement {
    pub fn new(
        cx: &mut Context<Self>,
        window: &mut gpui::Window,
        mp: &Entity<MediaPlayer>,
    ) -> Self {
        let addr_inp = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("127.0.0.1:9000")
                .default_value("127.0.0.1:9000")
        });
        Self {
            addr_inp,
            mp: mp.clone(),
            conn: None,
        }
    }
    fn connect(&mut self, cx: &mut Context<Self>) -> Result<()> {
        let conn = cx.new(|cx| {
            OscConnection::new(
                cx,
                self.addr_inp.read(cx).value().to_string(),
                self.mp.clone(),
            )
        });
        self.conn = Some(conn);
        cx.notify();
        Ok(())
    }
}

impl Render for OscManagement {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::prelude::Context<Self>,
    ) -> impl gpui::prelude::IntoElement {
        let conn_state = self.conn.as_ref().map(|x| &x.read(cx).state);

        match conn_state {
            Some(x @ OscConnState::Connected) => div().v_flex().gap_2().child(
                Button::new("osc-disconnect")
                    .label("disconnect")
                    .on_click(cx.listener(|this, e, win, cx| {
                        let Some(ref conn) = this.conn else {
                            return;
                        };
                        conn.update(cx, |this, cx| {
                            this.shutdown.store(true, Relaxed);
                        })
                    })),
            ),
            Some(x @ OscConnState::Connecting) => div().v_flex().gap_2().child("connecting.."),
            _ => div()
                .v_flex()
                .gap_2()
                .child(Input::new(&self.addr_inp))
                .child(
                    Button::new("osc-connect")
                        .label("connect")
                        .on_click(cx.listener(|this, e, win, cx| {
                            let outcome = this.connect(cx);
                            println!("connection = {outcome:?}")
                        })),
                ),
        }
    }
}
