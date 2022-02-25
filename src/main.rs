use std::io::{Read, stdin};
use byteorder::{ByteOrder, LittleEndian, ReadBytesExt};
use std::thread;
use std::sync::Mutex;
use std::sync::Arc;
use std::ffi::{CString, CStr};
use once_cell::sync::Lazy;
use std::env;

use serenity::{
    async_trait,
    model::{
        gateway::Ready,
        id::{
            GuildId,
            MessageId,
        },
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

static MODEL: Lazy<vosk::Model> = Lazy::new(|| vosk::Model::new("./model"));

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

#[tokio::main]
async fn main() {
    // Configure the client with your Discord bot token in the environment.
    let token = env::var("DISCORD_TOKEN").expect("Expected a token in the environment");

    // The Application Id is usually the Bot User Id.
    let application_id: u64 = env::var("APPLICATION_ID")
        .expect("Expected an application id in the environment")
        .parse()
        .expect("application id is not a valid id");

    // Load the model before connecting because it is slow
    Lazy::force(&MODEL);

    let songbird_config = songbird::Config::default()
        .decode_mode(songbird::driver::DecodeMode::Decode);
    

    // Build our client.
    let mut client = Client::builder(token)
        .event_handler(Handler::default())
        .application_id(application_id)
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
}

#[derive(Default)]    
struct Handler;

impl Handler {
    async fn handle_interaction(
        &self,
        ctx: &Context,
        interaction: &Interaction
    ) -> Result<(), String>
    {
        match interaction {
            Interaction::ApplicationCommand(cmd) => {
                match cmd.data.name.as_str() {
                    "caption" => {
                        let opt = cmd
                            .data
                            .options
                            .get(0)
                            .ok_or("Expected channel option")?
                            .resolved
                            .as_ref()
                            .ok_or("Expected channel object")?;

                        if let ApplicationCommandInteractionDataOptionValue::Channel(ch) = opt {
                            let guild_id = cmd.guild_id.unwrap();
                            let manager = songbird::get(ctx).await.unwrap();

                            if let (handler_lock, Ok(_)) = manager.join(guild_id, ch.id).await {
                                let mut handler = handler_lock.lock().await;
                                let webhook = ctx.http.get_webhook_from_url(env::var("WEBHOOK_URL")).await.unwrap();
                                let recv = voice_recv::ArcVoiceReceive(
                                    Arc::new(
                                        voice_recv::VoiceReceive::new(
                                            &*MODEL,
                                            ctx.cache.clone(),
                                            ctx.http.clone(),
                                            serenity::model::id::ChannelId(946600847905808414),
                                            webhook)));

                                handler.add_global_event(
                                    CoreEvent::SpeakingStateUpdate.into(),
                                    recv.clone(),
                                );

                                handler.add_global_event(
                                    CoreEvent::SpeakingUpdate.into(),
                                    recv.clone(),
                                );

                                handler.add_global_event(
                                    CoreEvent::VoicePacket.into(),
                                    recv.clone(),
                                );

                                handler.add_global_event(
                                    CoreEvent::RtcpPacket.into(),
                                    recv.clone(),
                                );

                                handler.add_global_event(
                                    CoreEvent::ClientDisconnect.into(),
                                    recv,
                                );
                            }

                            cmd
                                .create_interaction_response(ctx, |r| {
                                    r.kind(InteractionResponseType::Pong)
                                })
                                .await;
                        }
                    },
                    _ => {
                        eprintln!("Command {:?} not implemented", cmd.data.name.as_str());
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
                d.content(why1.clone())
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
                        eprintln!("Interaction {:?}: {}", interaction, why1);
                        Ok(())
                    }
                }
            {
                eprintln!("Cannot display error {:?}: {}", why1, why2);
            }
        }
    }
}
