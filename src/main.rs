use std::io::{Read, stdin};
use byteorder::{ByteOrder, LittleEndian, ReadBytesExt};
use std::thread;
use std::sync::Mutex;
use std::sync::Arc;
use std::ffi::{CString, CStr};
use once_cell::sync::OnceCell;
use std::error::Error;
use std::env;

use tokio::sync::Mutex as TokioMutex;

use serenity::{
    async_trait,
    model::{
        gateway::Ready,
        id::{
            GuildId,
            MessageId,
        },
        webhook::Webhook,
        channel::{GuildChannel, Channel},
        application::command::CommandType,
        interactions::{
            application_command::{
                ApplicationCommand,
                ApplicationCommandInteraction,
                ApplicationCommandInteractionDataOptionValue,
                ApplicationCommandOptionType,
            },
            message_component::ActionRowComponent,
            Interaction,
            InteractionResponseType,
            InteractionApplicationCommandCallbackDataFlags,
        },
    },
    http::Http,
    builder::{
        CreateInteractionResponse,
        CreateComponents,
        CreateActionRow,
        CreateButton,
        CreateSelectMenu,
        CreateSelectMenuOption,
    },
    prelude::*,
};

use songbird::{CoreEvent, SerenityInit};

mod vosk;
mod voice_recv;
mod config;
use config::Config;
mod db;
use db::BotDb;

static MODEL: OnceCell<vosk::Model> = OnceCell::new();

static CONFIG: OnceCell<Config> = OnceCell::new();

// fn main() -> Result<(), Box<dyn std::error::Error>> {
//     // use vosk::sys::*;
//     // unsafe {
//     //     let path = CString::new("./model")?;
//     //     let model = vosk_model_new(path.as_ptr());
//     //     let rec = vosk_recognizer_new(model, 16000.0);

//     //     let mut bytes = [0u8; 3200];

//     //     while let Ok(n_bytes) = stdin().read(&mut bytes) {
//     //         if n_bytes == 0 {
//     //             break;
//     //         }
//     //         let is_final = vosk_recognizer_accept_waveform(rec, bytes.as_ptr(), n_bytes);
//     //         let res = if is_final == 1 {
//     //             vosk_recognizer_result(rec)
//     //         } else {
//     //             // vosk_recognizer_partial_result(rec)
//     //             continue
//     //         };

//     //         let cs = CStr::from_ptr(res as *mut i8);
//     //         println!("{:?}", cs);
//     //     }

//     //     let res = vosk_recognizer_final_result(rec);
//     //     let cs = CStr::from_ptr(res as *mut i8);
//     //     println!("{:?}", cs);

//     //     vosk_recognizer_free(rec);
//     //     vosk_model_free(model);
//     // }

//     // Ok(())

//     use vosk::{Model, Recognizer};

//     let model = Model::new("./model");
//     let mut rec = Recognizer::new(&model, 48000.0);

//     // rec.set_max_alternatives(5);

//     let mut bytes = [0u8; 8192];

//     while let Ok(n_bytes) = stdin().read(&mut bytes) {
//         if n_bytes == 0 {
//             break;
//         }
//         let is_final = rec.accept_waveform(&bytes[..n_bytes]);
//         let res = if is_final {
//             rec.result_json()
//         } else {
//             // rec.partial_result()
//             continue
//         };

//         println!("{}", serde_json::from_slice::<vosk::SimpleResult>(res.to_bytes()).unwrap().text);
//     }

//     let res = rec.final_result_json();
//     println!("final: {}", serde_json::from_slice::<vosk::SimpleResult>(res.to_bytes()).unwrap().text);

//     Ok(())
// }

#[derive(Debug)]
enum BotError<M> {
    UserMessage(M),
    Error(Option<Box<dyn Error + Send + Sync>>)
}

impl<M, E: Error + Send + Sync + 'static> From<E> for BotError<M> {
    fn from(e: E) -> BotError<M> {
        BotError::Error(Some(Box::new(e)))
    }
}

impl<'a> From<BotError<&'a str>> for BotError<String> {
    fn from(e: BotError<&'a str>) -> BotError<String> {
        match e {
            BotError::UserMessage(m) => BotError::UserMessage(m.into()),
            BotError::Error(e) => BotError::Error(e)
        }
    }
}

fn get_config() -> Result<&'static Config, Box<dyn Error + Send + Sync>> {
    CONFIG.get_or_try_init(|| Ok(Config::from_file("./config.yaml")?))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let config = get_config()?;

    // Load the model before connecting because it is slow; check
    // model exists because VOSK error reporting is bad
    if config.model_path.exists() {
        MODEL.set(
            vosk::Model::new(&config.model_path)
                .ok_or_else(|| format!("Could not create vosk model from {:?}", config.model_path))?
        )
            .unwrap();
    } else {
        eprintln!("Model {:?} not found", config.model_path);
        std::process::exit(-1);
    }

    let songbird_config = songbird::Config::default()
        .decode_mode(songbird::driver::DecodeMode::Decode);

    let db = BotDb::new(&config.db_path).await?;

    // Build our client.
    let mut client = Client::builder(&*config.bot_token, GatewayIntents::default())
        .event_handler(Handler::new(db))
        .application_id(config.application_id)
        .register_songbird_from_config(songbird_config)
        .await
        .expect("Error creating client");

    // Finally, start a single shard, and start listening to events.
    //
    // Shards will automatically attempt to reconnect, and will perform
    // exponential backoff until it reconnects.
    if let Err(why) = client.start().await {
        println!("Client error: {:?}", why);
    }

    Ok(())
}

struct Handler {
    // Use Mutex for now because SqliteConnection is not Sync
    db: TokioMutex<BotDb>
}

impl Handler {
    fn new(db: BotDb) -> Handler {
        Handler {
            db: TokioMutex::new(db)
        }
    }

    async fn init_webhook(http: impl AsRef<Http>, chan: &GuildChannel) -> Result<Webhook, BotError<String>> {
        let hooks = chan.webhooks(&http).await?;
        let bot_id = http.as_ref().get_current_user().await?.id;

        if let Some(hook) = hooks.into_iter().find(|h| h.user.as_ref().map(|u| u.id) == Some(bot_id)) {
            Ok(hook)
        } else {
            Ok(chan.create_webhook(&http, "caption_bot").await?)
        }
    }

    async fn handle_interaction(
        &self,
        ctx: &Context,
        interaction: &Interaction
    ) -> Result<(), BotError<String>>
    {
        match interaction {
            Interaction::ApplicationCommand(cmd) => {
                match cmd.data.name.as_str() {
                    "caption" => {
                        let opt = cmd
                            .data
                            .options
                            .get(0)
                            .ok_or(BotError::UserMessage("Expected channel option"))?
                            .resolved
                            .as_ref()
                            .ok_or(BotError::UserMessage("Expected channel object"))?;

                        if let ApplicationCommandInteractionDataOptionValue::Channel(ch) = opt {
                            let guild_id = cmd
                                .guild_id
                                .ok_or(BotError::UserMessage("This command can only be used in servers"))?;
                            let db = self.db.lock().await;
                            let cfg_fut ={
                                let db = &*db;
                                db.guild_config(guild_id)
                            };

                            let guild_config = match cfg_fut.await {
                                Ok(Some(c)) => c,
                                Ok(None) => {
                                    let fut = {
                                        let db = &*db;
                                        db.add_guild(guild_id)
                                    };
                                    fut.await?
                                },
                                Err(e) => return Err(e.into())
                            };
                            drop(db);

                            let manager = songbird::get(ctx).await
                                .ok_or(BotError::<String>::Error(None))?;

                            // Avoid duplicate join events. TODO: only
                            // add handlers to new calls (detected
                            // with get())
                            if let Some(driver_lock) = manager.get(guild_id) {
                                let mut driver = driver_lock.lock().await;

                                driver.remove_all_global_events();
                            }

                            if let (driver_lock, Ok(_)) = manager.join(guild_id, ch.id).await {
                                let mut driver = driver_lock.lock().await;
                                // let webhook = ctx
                                //     .http
                                //     .get_webhook_from_url(&*get_config().unwrap().webhook_url)
                                //     .await
                                //     .unwrap();

                                let guild_ch = match ctx.http.get_channel(ch.id.0).await? {
                                    Channel::Guild(ch) => ch,
                                    _ => return Err(BotError::UserMessage("Captioning only available for guild channels").into())
                                };

                                let recv = voice_recv::ArcVoiceReceive(
                                    Arc::new(
                                        voice_recv::VoiceReceive::new(
                                            MODEL.get().unwrap(),
                                            ctx.cache.clone(),
                                            ctx.http.clone(),
                                            // guild_config.caption_channel.ok_or(BotError::UserMessage("There is no default caption channel in this server"))?,
                                            // webhook
                                            ch.id,
                                            Self::init_webhook(ctx, &guild_ch).await?
                                        )));

                                driver.add_global_event(
                                    CoreEvent::SpeakingStateUpdate.into(),
                                    recv.clone(),
                                );

                                driver.add_global_event(
                                    CoreEvent::SpeakingUpdate.into(),
                                    recv.clone(),
                                );

                                driver.add_global_event(
                                    CoreEvent::VoicePacket.into(),
                                    recv.clone(),
                                );

                                driver.add_global_event(
                                    CoreEvent::RtcpPacket.into(),
                                    recv.clone(),
                                );

                                driver.add_global_event(
                                    CoreEvent::ClientDisconnect.into(),
                                    recv,
                                );
                            }

                            cmd
                                .create_interaction_response(ctx, |r| {
                                    r.kind(InteractionResponseType::ChannelMessageWithSource);
                                    r.interaction_response_data(|d| {
                                        d.content(format!("Captioning {}", ch.id.mention()))
                                    })
                                })
                                .await?;
                        }
                    },
                    "set" => {
                        let sub = cmd
                            .data
                            .options
                            .get(0)
                            .ok_or(BotError::UserMessage("Expected subcommand"))?;

                        match &*sub.name {
                            "channel" => {
                                let opt = sub
                                    .options
                                    .get(0)
                                    .ok_or(BotError::UserMessage("Expected channel option"))?
                                    .resolved
                                    .as_ref()
                                    .ok_or(BotError::UserMessage("Expected channel object"))?;

                                if let ApplicationCommandInteractionDataOptionValue::Channel(ch) = opt {
                                    let guild_id = cmd
                                        .guild_id
                                        .ok_or(BotError::UserMessage("This command can only be used in servers"))?;
                                    let db = self.db.lock().await;

                                    let fut = {
                                        db.set_caption_channel(guild_id, Some(ch.id))
                                    };

                                    fut.await?;

                                    cmd
                                        .create_interaction_response(ctx, |r| {
                                            r.kind(InteractionResponseType::ChannelMessageWithSource);
                                            r.interaction_response_data(|d| {
                                                d.content(format!("Caption channel set to {}", ch.id.mention()))
                                            })
                                        })
                                        .await?;
                                }
                            },
                            _ => {}
                        }
                    },
                    "Caption Message" => {
                        use magnum::container::ogg::OpusSourceOgg;

                        cmd
                            .create_interaction_response(ctx, |r| {
                                r.kind(InteractionResponseType::DeferredChannelMessageWithSource)
                            })
                            .await?;

                        let msg_id = cmd
                            .data
                            .target_id
                            .ok_or(BotError::UserMessage("No message supplied to caption"))?;

                        let msg = &cmd
                            .data
                            .resolved
                            .messages[&msg_id.to_message_id()];

                        if msg.attachments.len() != 1 {
                            return Err(
                                BotError::UserMessage(
                                    format!("Sorry, only messages with 1 attachment are supported")));
                        }

                        let attach = &msg.attachments[0];

                        let audio_bytes = attach.download().await?;

                        let mut source = OpusSourceOgg::new(std::io::Cursor::new(audio_bytes))
                            .or(Err(BotError::UserMessage("Unsupported format (only OGG/Opus supported for now)")))?;

                        let sample_rate = source.metadata.sample_rate;

                        let mut rec = vosk::Recognizer::new(MODEL.get().unwrap(), sample_rate as f32);

                        let buf: Vec<i16> = source.map(|f| (f * 32767.0) as i16).collect();

                        rec.accept_waveform_i16(&*buf);

                        let partial: std::ffi::CString = rec.partial_result_json().to_owned();
                        let json: std::ffi::CString = rec.final_result_json().to_owned();
                        let text = serde_json::from_slice::<vosk::SimpleResult>(json.to_bytes())
                            .unwrap()
                            .text;

                        cmd.edit_original_interaction_response(ctx, |r| {
                            r.content(format!("Transcript of [voice message](<{}>):\n{}", msg.link(), text))
                        })
                            .await
                            .or(Err(BotError::UserMessage("No response to edit")))?;

                    },
                    _ => {
                        return Err(
                            BotError::UserMessage(
                                format!("Command {:?} not implemented", cmd.data.name.as_str())));
                    }
                }
            },
            _ => {}
        }

        Ok(())
    }
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        eprintln!("{} is connected!", ready.user.name);

        let guild_id = GuildId(
            406972931587964928
        );

        let commands = GuildId::set_application_commands(&guild_id, &ctx.http, |commands| {
            commands
                .create_application_command(|command| {
                    command.name("caption").description("Join a voice channel to caption").create_option(|option| {
                        option
                            .name("channel")
                            .description("The channel to join")
                            .kind(ApplicationCommandOptionType::Channel)
                            .required(true)
                    })
                })
                .create_application_command(|command| {
                    command.name("set").description("Set captioning options").create_option(|option| {
                        option
                            .name("channel")
                            .description("The channel to join")
                            .kind(ApplicationCommandOptionType::SubCommand)
                            .create_sub_option(|o| {
                                o
                                    .name("channel")
                                    .description("The channel to join")
                                    .kind(ApplicationCommandOptionType::Channel)
                                    .required(true)
                            })
                    })
                })
                .create_application_command(|command| {
                    command.name("Transcribe Message").kind(CommandType::Message)
                })
        })
            .await;

        eprintln!("Commands: {:#?}", commands);
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        if let Err(why1) = self.handle_interaction(&ctx, &interaction).await {
            let mut response = CreateInteractionResponse::default();
            response.kind(InteractionResponseType::ChannelMessageWithSource);
            response.interaction_response_data(|d| {
                d.flags(InteractionApplicationCommandCallbackDataFlags::EPHEMERAL);
                d.content(
                    match &why1 {
                        BotError::UserMessage(s) => s.clone(),
                        BotError::Error(_) => "Error running command".to_string()
                    })
            });
            if let Err(why2) =
                match interaction {
                    Interaction::ApplicationCommand(i) => {
                        i.create_interaction_response(&ctx, |r| {*r = response; r})
                            .await
                    },
                    Interaction::MessageComponent(i) => {
                        i.create_interaction_response(&ctx, |r| {*r = response; r})
                            .await
                    },
                    // Interaction::ModalSubmit(i) => {
                    //     i.create_interaction_response(&ctx, |r| *r = response)
                    //         .await
                    // },
                    _ => {
                        eprintln!("Interaction {:?}: {:?}", interaction, why1);
                        Ok(())
                    }
                }
            {
                eprintln!("Cannot display error {:?}: {:?}", why1, why2);
            }
        }
    }
}
