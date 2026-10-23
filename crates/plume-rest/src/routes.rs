use plume_core::Snowflake;
use reqwest::Method;

/// Every endpoint we call. Keeping them in one enum means the rate limiter
/// always knows which bucket a request belongs to.
#[derive(Debug, Clone, Copy)]
pub enum Route {
    CurrentUser,
    GetMessages { channel_id: Snowflake },
    CreateMessage { channel_id: Snowflake },
    TriggerTyping { channel_id: Snowflake },
}

impl Route {
    pub fn method(self) -> Method {
        match self {
            Route::CurrentUser | Route::GetMessages { .. } => Method::GET,
            Route::CreateMessage { .. } | Route::TriggerTyping { .. } => Method::POST,
        }
    }

    pub fn path(self) -> String {
        match self {
            Route::CurrentUser => "/users/@me".to_owned(),
            Route::GetMessages { channel_id } | Route::CreateMessage { channel_id } => {
                format!("/channels/{channel_id}/messages")
            }
            Route::TriggerTyping { channel_id } => format!("/channels/{channel_id}/typing"),
        }
    }

    /// Discord rate limits per route *and* per "major parameter" (the
    /// channel or guild), so sending in two channels uses two buckets.
    pub fn bucket_key(self) -> String {
        match self {
            Route::CurrentUser => "GET /users/@me".to_owned(),
            Route::GetMessages { channel_id } => format!("GET /channels/{channel_id}/messages"),
            Route::CreateMessage { channel_id } => format!("POST /channels/{channel_id}/messages"),
            Route::TriggerTyping { channel_id } => format!("POST /channels/{channel_id}/typing"),
        }
    }
}
