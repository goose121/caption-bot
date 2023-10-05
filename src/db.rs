use sqlx::{SqlitePool, Result as SqlResult, ConnectOptions};
use std::convert::TryInto;
use std::path::Path;

use std::error::Error;

use serenity::model::id::{ChannelId, GuildId};

pub struct BotDb {
    conn: SqlitePool
}

impl BotDb {
    pub async fn new(p: impl AsRef<Path>) -> SqlResult<BotDb> {
        Ok(
            Self {
                conn: sqlx::sqlite::SqlitePoolOptions::new()
                    .max_connections(5)
                    .connect_with(
                        sqlx::sqlite::SqliteConnectOptions::new()
                            .filename(p)
                            .create_if_missing(true),
                    )
                    .await?

            }
        )
    }

    pub async fn create_table(&self) -> SqlResult<()> {
        sqlx::query![
            "CREATE TABLE IF NOT EXISTS guilds (
                 guild_id BIGINT PRIMARY KEY,
                 caption_channel BIGINT,
                 lang CHAR(3)
             )"]
            .execute(&self.conn)
            .await?;

        Ok(())
    }

    pub async fn add_guild(&self, guild: impl Into<GuildId>) -> SqlResult<GuildConfig> {
        let g = guild.into().0 as i64;
        sqlx::query![
            "INSERT INTO guilds (guild_id, caption_channel, lang) VALUES (?1, ?2, ?3)",
            g,
            None as Option<i64>,
            None as Option<String>]
            .execute(&self.conn)
            .await?;

        Ok(
            GuildConfig {
                caption_channel: None,
                lang: None
            }
        )
    }

    pub async fn guild_config(&self, guild: impl Into<GuildId>) -> SqlResult<Option<GuildConfig>> {
        let g = guild.into().0 as i64;
        let row = sqlx::query![
            "SELECT caption_channel, lang FROM guilds WHERE guilds.guild_id = ?",
            g]
            .fetch_optional(&self.conn)
            .await?;
                

        row.map(
            |row| {
                use sqlx::Row;
                Ok(
                    GuildConfig {
                        caption_channel: row.caption_channel.map(|id| ChannelId(id as u64)),
                        lang: row.lang.map(|l| l.as_bytes().try_into().unwrap())
                    }
                )
            })
            .transpose()
    }

    pub async fn set_caption_channel(
        &self,
        guild: impl Into<GuildId>,
        channel: Option<impl Into<ChannelId>>
    ) -> SqlResult<()>
    {
        let g = guild.into().0 as i64;
        let c = channel.map(|c| c.into().0 as i64);
        sqlx::query![
            "UPDATE guilds SET caption_channel = ?1
                 WHERE guild_id = ?2",
            c,
            g]
            .execute(&self.conn)
            .await?;

        Ok(())
    }

    pub async fn set_lang(
        &self,
        guild: impl Into<GuildId>,
        lang: Option<[u8; 3]>
    ) -> SqlResult<()>
    {
        let g = guild.into().0 as i64;
        let l = lang.as_ref().map(|lang| std::str::from_utf8(lang).unwrap());
        sqlx::query![
            "UPDATE guilds SET caption_channel = ?1
                 WHERE guild_id = ?2",
            g,
            l]
            .execute(&self.conn)
            .await?;

        Ok(())
    }
}

pub struct GuildConfig {
    pub caption_channel: Option<ChannelId>,
    pub lang: Option<[u8; 3]>
}
