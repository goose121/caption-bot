use serenity::{
    async_trait,
    model::{
        id::{UserId, ChannelId},
        webhook::Webhook
    },
    cache::Cache,
    http::client::Http,
};
use bimap::hash::BiHashMap;
use std::collections::HashMap;
use std::time::Instant;
use songbird::{
    events::{
        context_data::{SpeakingUpdateData, VoiceData},
        EventHandler,
        EventContext,
        Event,
    },
    model::payload::{ClientDisconnect, Speaking},
};
use std::sync::{Arc, Mutex};
use std::num::Wrapping;
use discortp::rtp::Rtp;

use serde_json::json;

use crate::vosk;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SSRC(pub u32);

pub struct VoiceReceive {
    // Map from audio SSRC to UserId
    // LOCK ORDER: recognizers, ssrc_map
    ssrc_map: Mutex<BiHashMap<SSRC, UserId>>,
    recognizers: Mutex<HashMap<SSRC, vosk::Recognizer>>,
    model: &'static vosk::Model,
    ctx: (Arc<Cache>, Arc<Http>),
    chan: ChannelId,
    webhook: Webhook,
}

impl VoiceReceive {
    pub fn new(model: &'static vosk::Model, cache: Arc<Cache>, http: Arc<Http>, chan: ChannelId, webhook: Webhook) -> VoiceReceive {
        VoiceReceive {
            ssrc_map: Default::default(),
            recognizers: Default::default(),
            model,
            ctx: (cache, http),
            chan,
            webhook
        }
    }

    async fn begin_speaking(&self, ssrc: SSRC) {
        eprintln!("Begin speaking");
        let mut recognizers = self.recognizers.lock().unwrap();

        recognizers.get_mut(&ssrc).map(|r| r.reset());
    }

    async fn finish_speaking(&self, ssrc: SSRC) {
        eprintln!("Finish speaking");
        let r = {
            self.recognizers.lock().unwrap().remove(&ssrc)
        };


        if let Some(mut rec) = r {
            // rec.accept_waveform_i16(&*vec![0i16; 1]);
            let partial: std::ffi::CString = rec.partial_result_json().to_owned();
            let json: std::ffi::CString = rec.final_result_json().to_owned();
            let text = serde_json::from_slice::<vosk::SimpleResult>(json.to_bytes())
                .unwrap()
                .text;

            let u = {
                self.ssrc_map.lock().unwrap().get_by_left(&ssrc).copied()
            };

            let (name, avatar) = match u {
                Some(u) => {
                    let m = self
                        .chan
                        .to_channel((&self.ctx.0, &*self.ctx.1))
                        .await
                        .unwrap()
                        .guild()
                        .unwrap()
                        .guild_id
                        .member((&self.ctx.0, &*self.ctx.1), u)
                        .await
                        .unwrap();
                    (m.display_name().into_owned(), m.face())
                },
                None => ("Unknown user".to_string(), String::new())
            };

            if !text.is_empty() {
                let map = json!({"name": "CaptionBot"});
                // let webhook = self.ctx.1.create_webhook(self.chan.0, &map).await.unwrap();

                self.webhook.execute(
                    &self.ctx.1,
                    false,
                    |w| {
                        w.content(text);
                        w.avatar_url(avatar);
                        w.username(name)
                    })
                    .await
                    .unwrap();

                // webhook.delete(&self.ctx.1).await.unwrap();
            }
        }
    }

    async fn process_audio(&self, data: VoiceData<'_>) {
        use std::fs::OpenOptions;
        use byteorder::WriteBytesExt;
        
        let ssrc = SSRC(u32::from_be(data.packet.ssrc));
        let mut recognizers = self.recognizers.lock().unwrap();
        let rec = recognizers.entry(ssrc).or_insert_with(|| vosk::Recognizer::new(&self.model, 48_000.0));

        let mono_data: Vec<i16> = data.audio.as_ref().unwrap().chunks_exact(2).map(|c| c[0]/2 + c[1]/2).collect();

        let mut f = OpenOptions::new()
            .append(true)
            .create(true)
            .write(true)
            .open("capture.raw")
            .unwrap();

        for &s in mono_data.iter() {
            f.write_i16::<byteorder::NativeEndian>(s).unwrap();
        }

        if rec.accept_waveform_i16(&*mono_data) {
            eprintln!("{:?}", rec.partial_result_json());
            // TODO: edit message with partial results
        } else {
            eprintln!("{:?}", rec.partial_result_json());
            // TODO
        }
    }
}

#[async_trait]
impl EventHandler for VoiceReceive {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        match *ctx {
            EventContext::SpeakingStateUpdate(Speaking { speaking, ssrc, user_id, .. }) => {
                let ssrc = SSRC(u32::from_be(ssrc));
                let user_id = UserId(user_id.unwrap().0);

                {
                    self.ssrc_map.lock().unwrap().insert(ssrc, user_id);
                }

                if speaking.is_empty() {
                    self.finish_speaking(ssrc).await;
                } else {
                    self.begin_speaking(ssrc).await;
                }
            },
            EventContext::SpeakingUpdate(SpeakingUpdateData { speaking, ssrc, .. }) => {
                let ssrc = SSRC(u32::from_be(ssrc));

                if speaking {
                    self.begin_speaking(ssrc).await;
                } else {
                    self.finish_speaking(ssrc).await;
                }
            },
            EventContext::ClientDisconnect(ClientDisconnect { user_id }) => {
                let user_id = UserId(user_id.0);

                let res = {
                    self.ssrc_map.lock().unwrap().get_by_right(&user_id).copied()
                };
                if let Some(ssrc) = res
                {
                    self.finish_speaking(ssrc).await;

                    self.ssrc_map.lock().unwrap().remove_by_right(&user_id);
                }
            },
            EventContext::VoicePacket(ref p) => {
                self.process_audio(p.clone()).await;
            },
            _ => {}
        }

        None
    }
}

#[derive(Clone)]
pub struct ArcVoiceReceive(pub Arc<VoiceReceive>);

#[async_trait]
impl EventHandler for ArcVoiceReceive {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        self.0.act(ctx).await
    }
}
