use std::{
    env,
    sync::Arc,
    time::{Duration, Instant},
};

use moka::future::Cache;
use serenity::{
    client::{Context, EventHandler},
    model::{channel::Message, gateway::Ready, prelude::UserId},
    prelude::{GatewayIntents, TypeMapKey},
    utils::MessageBuilder,
    Client,
};
use tokio::sync::RwLock;

/// Time in seconds until a message is automatically evicted from the tracking cache.
const TIME_TO_IDLE_IN_SECS: u64 = 120;
/// Minimum length of messages to be tracked. Anything shorter than this is ignored entirely.
const MIN_MESSAGE_LENGTH: usize = 50;

struct MessageCache;

impl TypeMapKey for MessageCache {
    type Value = Arc<RwLock<Cache<(UserId, String), Instant>>>;
}

struct Handler;

#[serenity::async_trait]
impl EventHandler for Handler {
    async fn message(&self, context: Context, msg: Message) {
        if msg.content.len() <= MIN_MESSAGE_LENGTH {
            return;
        }

        let cache_lock = {
            let data_read = context.data.read().await;
            data_read
                .get::<MessageCache>()
                .expect("Expected MessageCache in TypeMap.")
                .clone()
        };

        let now = Instant::now();

        let key = (msg.author.id, msg.content.clone());
        let timestamp = { cache_lock.read().await.get(&key) };
        {
            cache_lock.write().await.insert(key, now).await;
        }

        if let Some(timestamp) = timestamp {
            let duration = now.checked_duration_since(timestamp);
            if let Some(duration) = duration {
                if duration.as_secs() <= TIME_TO_IDLE_IN_SECS {
                    let dm_intro = match msg.guild(&context) {
                        Some(guild) => format!(
                            "Your recent message in the {} Discord server has been automatically deleted.",
                            guild.name
                        ),
                        None => "Your recent message in a Discord server has been automatically deleted."
                            .to_string(),
                    };

                    let content = MessageBuilder::new()
                        .push(format!("{} It was recognized as a duplicate that you posted in several channels in quick succession. Please be patient and refrain from posting the same message in multiple channels.", dm_intro))
                        .build();

                    if let Err(e) = msg.delete(&context).await {
                        tracing::error!("There was an error while attempting to delete a duplicate message: {:?}", e);
                    } else if let Err(e) = msg.author.dm(&context, |m| m.content(content)).await {
                        tracing::error!("There was an error while attempting to message an author of a deleted message: {:?}", e);
                    }
                }
            }
        }
    }

    async fn ready(&self, _: Context, data: Ready) {
        tracing::info!("{} is connected and running.", data.user.name);
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let token =
        env::var("DISCORD_TOKEN").expect("Could not find the DISCORD_TOKEN environment variable.");
    let intents =
        GatewayIntents::GUILD_MESSAGES | GatewayIntents::MESSAGE_CONTENT | GatewayIntents::GUILDS;
    let mut client = Client::builder(&token, intents)
        .event_handler(Handler)
        .await
        .expect("There was an unexpected error while attempting to create a client.");

    {
        let mut data = client.data.write().await;
        data.insert::<MessageCache>(Arc::new(RwLock::new(
            Cache::builder()
                .time_to_idle(Duration::from_secs(TIME_TO_IDLE_IN_SECS))
                .build(),
        )))
    }

    if let Err(reason) = client.start().await {
        tracing::error!(
            "An unexpected client error occurred during runtime: {:?}",
            reason
        );
    }
}
