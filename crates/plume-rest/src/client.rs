use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use plume_core::models::{Channel, Message, User};
use plume_core::wire::{WireChannel, WireMessage, WireUser};
use plume_core::{Snowflake, Token};
use reqwest::header::{AUTHORIZATION, HeaderValue};
use reqwest::{Response, StatusCode};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use tracing::{debug, warn};

use crate::errors::RestError;
use crate::ratelimit::RateLimiter;
use crate::routes::Route;
use crate::upload::{self, MAX_UPLOAD_BYTES, Upload};

/// What a request carries.
enum Body<'a> {
    None,
    Json(serde_json::Value),
    /// Rebuilt for each attempt, because a sent form can't be reused.
    Files(serde_json::Value, &'a [Upload]),
}

const API_BASE: &str = "https://discord.com/api/v9";

/// How Plume identifies itself over HTTP. Kept in this one place so it's
/// easy to update (see docs/04-discord-protocol.md, "Client identity").
pub const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";

const MAX_ATTEMPTS: u32 = 3;

/// An HTTP client with our TLS setup and user agent, for any Discord request.
pub fn build_http_client() -> reqwest::Result<reqwest::Client> {
    // reqwest is built without a default crypto backend, so pick `ring`
    // (the same one the WebSocket side uses). An error only means one is
    // already installed, which is fine.
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder().user_agent(USER_AGENT).build()
}

/// Talks to Discord's REST API as the logged-in user. Cheap to clone: all
/// clones share one connection pool and one rate limiter.
#[derive(Clone)]
pub struct RestClient {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    auth: HeaderValue,
    limiter: RateLimiter,
}

impl RestClient {
    pub fn new(token: &Token) -> Result<Self, RestError> {
        let mut auth =
            HeaderValue::from_str(token.expose()).map_err(|_| RestError::Unauthorized)?;
        // Keeps the token out of reqwest's debug output.
        auth.set_sensitive(true);
        Ok(RestClient {
            inner: Arc::new(Inner {
                http: build_http_client()?,
                auth,
                limiter: RateLimiter::default(),
            }),
        })
    }

    /// Who the token belongs to.
    pub async fn current_user(&self) -> Result<User, RestError> {
        let user: WireUser = self.request(Route::CurrentUser, None).await?;
        Ok(user.into_model())
    }

    /// The newest `limit` (max 100) messages in a channel, oldest first.
    /// With `before`, the ones just older than that message instead.
    pub async fn messages(
        &self,
        channel_id: Snowflake,
        limit: u8,
        before: Option<Snowflake>,
    ) -> Result<Vec<(Message, User)>, RestError> {
        let route = Route::GetMessages { channel_id };
        let mut query = format!("?limit={}", limit.clamp(1, 100));
        if let Some(before) = before {
            query.push_str(&format!("&before={before}"));
        }
        let wire: Vec<WireMessage> = self.request_with_query(route, &query, None).await?;
        // Discord sends newest first.
        Ok(wire
            .into_iter()
            .rev()
            .map(WireMessage::into_models)
            .collect())
    }

    /// Sends a message, optionally as a reply. Returns it as Discord stored it.
    pub async fn send_message(
        &self,
        channel_id: Snowflake,
        content: &str,
        reply_to: Option<ReplyTo>,
    ) -> Result<(Message, User), RestError> {
        let mut body = json!({
            "content": content,
            // A nonce lets Discord drop accidental duplicates.
            "nonce": now_snowflake().to_string(),
            "tts": false,
        });
        add_reply(&mut body, reply_to);
        let wire: WireMessage = self
            .request(Route::CreateMessage { channel_id }, Some(body))
            .await?;
        Ok(wire.into_models())
    }

    /// Like [`RestClient::send_message`], with files attached.
    pub async fn send_message_with_files(
        &self,
        channel_id: Snowflake,
        content: &str,
        reply_to: Option<ReplyTo>,
        files: &[Upload],
    ) -> Result<(Message, User), RestError> {
        if let Some(big) = files.iter().find(|f| f.bytes.len() > MAX_UPLOAD_BYTES) {
            return Err(RestError::Local(format!(
                "{} is bigger than 10 MB",
                big.filename
            )));
        }
        let mut payload = json!({ "content": content, "nonce": now_snowflake().to_string() });
        add_reply(&mut payload, reply_to);
        let response = self
            .send_body(
                Route::CreateMessage { channel_id },
                "",
                Body::Files(payload, files),
            )
            .await?;
        let wire: WireMessage = serde_json::from_slice(&response.bytes().await?)?;
        Ok(wire.into_models())
    }

    /// Changes the text of one of our messages.
    pub async fn edit_message(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
        content: &str,
    ) -> Result<(Message, User), RestError> {
        let route = Route::EditMessage {
            channel_id,
            message_id,
        };
        let wire: WireMessage = self
            .request(route, Some(json!({ "content": content })))
            .await?;
        Ok(wire.into_models())
    }

    pub async fn delete_message(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
    ) -> Result<(), RestError> {
        self.send(
            Route::DeleteMessage {
                channel_id,
                message_id,
            },
            "",
            None,
        )
        .await?;
        Ok(())
    }

    /// Adds or removes our reaction. `emoji` is from [`ReactionEmoji::api_name`].
    ///
    /// [`ReactionEmoji::api_name`]: plume_core::models::ReactionEmoji::api_name
    pub async fn set_reaction(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
        emoji: &str,
        add: bool,
    ) -> Result<(), RestError> {
        let route = Route::OwnReaction {
            channel_id,
            message_id,
            emoji: emoji.to_owned(),
            add,
        };
        self.send(route, "", None).await?;
        Ok(())
    }

    /// Tells Discord we've read up to this message, like the official client
    /// does, so other devices stop showing it as unread.
    pub async fn ack_message(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
    ) -> Result<(), RestError> {
        let route = Route::AckMessage {
            channel_id,
            message_id,
        };
        self.send(route, "", Some(json!({ "token": null }))).await?;
        Ok(())
    }

    /// Shows "… is typing" to others for ~10 seconds.
    pub async fn trigger_typing(&self, channel_id: Snowflake) -> Result<(), RestError> {
        self.send(Route::TriggerTyping { channel_id }, "", None)
            .await?;
        Ok(())
    }

    /// Ring everyone in a DM or group DM after starting a call there.
    pub async fn ring_call(&self, channel_id: Snowflake) -> Result<(), RestError> {
        let body = json!({ "recipients": null });
        self.send(Route::RingCall { channel_id }, "", Some(body))
            .await?;
        Ok(())
    }

    /// Someone's profile: bio, pronouns and the servers you share.
    pub async fn user_profile(&self, user_id: Snowflake) -> Result<Profile, RestError> {
        let wire: WireProfile = self
            .request_with_query(
                Route::UserProfile { user_id },
                "?with_mutual_guilds=true",
                None,
            )
            .await?;
        let details = wire.user_profile.unwrap_or_default();
        Ok(Profile {
            bio: details
                .bio
                .or(wire.user.bio)
                .filter(|b| !b.trim().is_empty()),
            pronouns: details.pronouns.filter(|p| !p.trim().is_empty()),
            mutual_guilds: wire.mutual_guilds.into_iter().map(|g| g.id).collect(),
        })
    }

    /// A channel's pinned messages, newest first.
    pub async fn pins(&self, channel_id: Snowflake) -> Result<Vec<(Message, User)>, RestError> {
        let wire: Vec<WireMessage> = self.request(Route::Pins { channel_id }, None).await?;
        Ok(wire.into_iter().map(WireMessage::into_models).collect())
    }

    /// Messages matching `text` in a server (or a DM when `guild_id` is
    /// `None`, searching `channel_id`). Newest first, up to 25.
    pub async fn search(
        &self,
        guild_id: Option<Snowflake>,
        channel_id: Snowflake,
        text: &str,
    ) -> Result<Vec<(Message, User)>, RestError> {
        #[derive(Deserialize)]
        struct Results {
            #[serde(default)]
            messages: Vec<Vec<WireMessage>>,
        }
        let route = Route::Search {
            id: guild_id.unwrap_or(channel_id),
            guild: guild_id.is_some(),
        };
        let query = format!("?content={}", crate::routes::encode_query(text));
        let results: Results = self.request_with_query(route, &query, None).await?;
        // Each hit comes with surrounding context; the first one is the match.
        Ok(results
            .messages
            .into_iter()
            .filter_map(|group| group.into_iter().next())
            .map(WireMessage::into_models)
            .collect())
    }

    /// Send a sticker on its own.
    pub async fn send_sticker(
        &self,
        channel_id: Snowflake,
        sticker_id: Snowflake,
    ) -> Result<(Message, User), RestError> {
        let body = json!({
            "sticker_ids": [sticker_id.to_string()],
            "nonce": now_snowflake().to_string(),
        });
        let wire: WireMessage = self
            .request(Route::CreateMessage { channel_id }, Some(body))
            .await?;
        Ok(wire.into_models())
    }

    /// GIFs from Discord's Tenor search: trending when `query` is empty.
    pub async fn gifs(&self, query: &str) -> Result<Vec<Gif>, RestError> {
        let search = !query.trim().is_empty();
        let mut params = "?media_format=gif&provider=tenor&locale=en-US&limit=30".to_owned();
        if search {
            params.push_str(&format!("&q={}", crate::routes::encode_query(query.trim())));
        }
        self.request_with_query(Route::Gifs { search }, &params, None)
            .await
    }

    pub async fn kick(&self, guild_id: Snowflake, user_id: Snowflake) -> Result<(), RestError> {
        let route = Route::Member {
            guild_id,
            user_id,
            remove: true,
        };
        self.send(route, "", None).await?;
        Ok(())
    }

    pub async fn ban(&self, guild_id: Snowflake, user_id: Snowflake) -> Result<(), RestError> {
        let body = json!({ "delete_message_seconds": 0 });
        self.send(Route::Ban { guild_id, user_id }, "", Some(body))
            .await?;
        Ok(())
    }

    /// Time someone out until `until_unix` (seconds). 0 lifts it.
    pub async fn timeout(
        &self,
        guild_id: Snowflake,
        user_id: Snowflake,
        until_unix: i64,
    ) -> Result<(), RestError> {
        let until = if until_unix == 0 {
            serde_json::Value::Null
        } else {
            json!(rfc3339(until_unix))
        };
        let route = Route::Member {
            guild_id,
            user_id,
            remove: false,
        };
        self.send(
            route,
            "",
            Some(json!({ "communication_disabled_until": until })),
        )
        .await?;
        Ok(())
    }

    pub async fn set_pinned(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
        pinned: bool,
    ) -> Result<(), RestError> {
        let route = Route::Pin {
            channel_id,
            message_id,
            add: pinned,
        };
        self.send(route, "", None).await?;
        Ok(())
    }

    /// Change our display name.
    pub async fn set_display_name(&self, name: &str) -> Result<(), RestError> {
        let name = if name.trim().is_empty() {
            serde_json::Value::Null
        } else {
            json!(name.trim())
        };
        self.send(Route::EditMe, "", Some(json!({ "global_name": name })))
            .await?;
        Ok(())
    }

    /// Change our avatar to an image file's bytes.
    pub async fn set_avatar(&self, bytes: &[u8], mime: &str) -> Result<(), RestError> {
        use base64::Engine;
        let data = base64::engine::general_purpose::STANDARD.encode(bytes);
        let body = json!({ "avatar": format!("data:{mime};base64,{data}") });
        self.send(Route::EditMe, "", Some(body)).await?;
        Ok(())
    }

    /// Notifications for a server: 0 all messages, 1 only @mentions, 2 nothing.
    pub async fn set_notification_level(
        &self,
        guild_id: Snowflake,
        level: u8,
    ) -> Result<(), RestError> {
        let body = json!({ "message_notifications": level });
        self.send(Route::GuildSettings { guild_id }, "", Some(body))
            .await?;
        Ok(())
    }

    /// Join a server from an invite code or link.
    pub async fn join_invite(&self, code: &str) -> Result<(), RestError> {
        let code = code
            .trim()
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_owned();
        self.send(Route::Invite { code }, "", Some(json!({})))
            .await?;
        Ok(())
    }

    /// Accept a friend request.
    pub async fn accept_friend(&self, user_id: Snowflake) -> Result<(), RestError> {
        let route = Route::Relationship { user_id, add: true };
        self.send(route, "", Some(json!({}))).await?;
        Ok(())
    }

    /// Unfriend, or decline/cancel a friend request.
    pub async fn remove_relationship(&self, user_id: Snowflake) -> Result<(), RestError> {
        let route = Route::Relationship {
            user_id,
            add: false,
        };
        self.send(route, "", None).await?;
        Ok(())
    }

    /// Send a friend request to a username.
    pub async fn send_friend_request(&self, username: &str) -> Result<(), RestError> {
        let body = json!({ "username": username, "discriminator": null });
        self.send(Route::FriendRequest, "", Some(body)).await?;
        Ok(())
    }

    /// Mute or unmute a whole server (`None` = the DM settings).
    pub async fn set_guild_muted(&self, guild_id: Snowflake, muted: bool) -> Result<(), RestError> {
        let body = json!({ "muted": muted });
        self.send(Route::GuildSettings { guild_id }, "", Some(body))
            .await?;
        Ok(())
    }

    /// The DM channel with someone, created if needed.
    pub async fn open_dm(&self, user_id: Snowflake) -> Result<(Channel, Vec<User>), RestError> {
        let body = json!({ "recipients": [user_id.to_string()] });
        let channel: WireChannel = self.request(Route::CreateDm, Some(body)).await?;
        Ok(channel.into_parts(None))
    }

    async fn request<T: DeserializeOwned>(
        &self,
        route: Route,
        body: Option<serde_json::Value>,
    ) -> Result<T, RestError> {
        self.request_with_query(route, "", body).await
    }

    async fn request_with_query<T: DeserializeOwned>(
        &self,
        route: Route,
        query: &str,
        body: Option<serde_json::Value>,
    ) -> Result<T, RestError> {
        let response = self.send(route, query, body).await?;
        let bytes = response.bytes().await?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Sends a request through the rate limiter, retrying on 429.
    async fn send(
        &self,
        route: Route,
        query: &str,
        body: Option<serde_json::Value>,
    ) -> Result<Response, RestError> {
        self.send_body(route, query, body.map_or(Body::None, Body::Json))
            .await
    }

    async fn send_body(
        &self,
        route: Route,
        query: &str,
        body: Body<'_>,
    ) -> Result<Response, RestError> {
        let key = route.bucket_key();
        let url = format!("{API_BASE}{}{query}", route.path());

        for _ in 0..MAX_ATTEMPTS {
            self.inner.limiter.acquire(&key).await;

            let mut request = self
                .inner
                .http
                .request(route.method(), &url)
                .header(AUTHORIZATION, self.inner.auth.clone());
            request = match &body {
                Body::None => request,
                Body::Json(json) => request.json(json),
                Body::Files(payload, files) => {
                    request.multipart(upload::form(payload.clone(), files))
                }
            };
            debug!("{} {}", route.method(), route.path());
            let response = request.send().await?;
            self.inner.limiter.update(&key, response.headers());

            match response.status() {
                status if status.is_success() => return Ok(response),
                StatusCode::TOO_MANY_REQUESTS => {
                    let limit: TooManyRequests = response.json().await.unwrap_or_default();
                    let wait = Duration::from_secs_f64(limit.retry_after.max(0.5));
                    warn!(
                        "rate limited on {key} (global: {}); waiting {wait:.1?}",
                        limit.global
                    );
                    self.inner.limiter.penalize(&key, wait, limit.global);
                }
                StatusCode::UNAUTHORIZED => return Err(RestError::Unauthorized),
                StatusCode::FORBIDDEN => return Err(RestError::Forbidden),
                StatusCode::NOT_FOUND => return Err(RestError::NotFound),
                status => {
                    let error: ApiError = response.json().await.unwrap_or_default();
                    return Err(RestError::Api {
                        status,
                        message: error.message,
                    });
                }
            }
        }
        Err(RestError::RateLimited)
    }
}

#[derive(Default, Deserialize)]
struct TooManyRequests {
    #[serde(default)]
    retry_after: f64,
    #[serde(default)]
    global: bool,
}

#[derive(Default, Deserialize)]
struct ApiError {
    #[serde(default)]
    message: String,
}

fn now_snowflake() -> Snowflake {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    Snowflake::from_timestamp_ms(now_ms)
}

/// What a profile shows beyond the basic user.
#[derive(Debug, Clone, Default)]
pub struct Profile {
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    /// Servers you're both in.
    pub mutual_guilds: Vec<Snowflake>,
}

#[derive(Deserialize)]
struct WireProfile {
    user: WireProfileUser,
    user_profile: Option<WireProfileDetails>,
    #[serde(default)]
    mutual_guilds: Vec<WireMutualGuild>,
}

#[derive(Deserialize)]
struct WireProfileUser {
    bio: Option<String>,
}

#[derive(Default, Deserialize)]
struct WireProfileDetails {
    bio: Option<String>,
    pronouns: Option<String>,
}

#[derive(Deserialize)]
struct WireMutualGuild {
    id: Snowflake,
}

/// Which message we're replying to, and whether to ping its author.
#[derive(Debug, Clone, Copy)]
pub struct ReplyTo {
    pub message_id: Snowflake,
    pub ping: bool,
}

fn add_reply(body: &mut serde_json::Value, reply_to: Option<ReplyTo>) {
    if let Some(reply) = reply_to {
        body["message_reference"] = json!({ "message_id": reply.message_id });
        body["allowed_mentions"] = json!({
            "parse": ["users", "roles", "everyone"],
            "replied_user": reply.ping,
        });
    }
}

/// One result from the GIF picker.
#[derive(Debug, Clone, Deserialize)]
pub struct Gif {
    /// What gets sent: Discord turns it into a playing GIF.
    pub url: String,
    /// A small still or animated preview for the picker.
    #[serde(default)]
    pub preview: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
}

/// `2026-10-25T12:00:00Z` from Unix seconds (UTC), without a date library.
fn rfc3339(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

#[cfg(test)]
mod time_tests {
    #[test]
    fn rfc3339() {
        assert_eq!(super::rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(super::rfc3339(1_793_000_000), "2026-10-26T07:33:20Z");
    }
}
