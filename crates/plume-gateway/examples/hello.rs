//! The learning example from the start: connect, log in, heartbeat, print.
//!
//!     read -rs DISCORD_TOKEN && export DISCORD_TOKEN
//!     cargo run -p plume-gateway --example hello

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const GATEWAY_URL: &str = "wss://gateway.discord.gg/?v=10&encoding=json";

/// Every message from the gateway has this shape.
#[derive(Debug, Deserialize)]
struct GatewayPayload {
    op: u8,                       // what kind of message this is
    d: Option<serde_json::Value>, // the data; its shape depends on `op`
    s: Option<u64>,               // sequence number (only on events)
    t: Option<String>,            // event name (only on events)
}

/// The data inside a HELLO (op 10).
#[derive(Debug, Deserialize)]
struct Hello {
    heartbeat_interval: u64,
}

/// The parts of a MESSAGE_CREATE event we care about for now.
#[derive(Debug, Deserialize)]
struct MessageCreate {
    channel_id: String, // Discord sends IDs as strings in JSON
    content: String,
    author: Author,
}

#[derive(Debug, Deserialize)]
struct Author {
    username: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Read the token from the environment, never from the code.
    let token = std::env::var("DISCORD_TOKEN").map_err(|_| "DISCORD_TOKEN is not set")?;

    println!("Connecting to {GATEWAY_URL} ...");
    let (mut socket, _response) = connect_async(GATEWAY_URL).await?;

    // The first message must be HELLO.
    let first = socket.next().await.ok_or("closed before HELLO")??;
    let payload: GatewayPayload = serde_json::from_str(first.to_text()?)?;
    if payload.op != 10 {
        return Err(format!("expected HELLO (op 10), got op {}", payload.op).into());
    }
    let hello: Hello = serde_json::from_value(payload.d.ok_or("HELLO had no data")?)?;
    println!("HELLO: heartbeat every {} ms", hello.heartbeat_interval);

    // IDENTIFY (op 2): log in. Never print this payload, it contains the token.
    let identify = json!({
        "op": 2,
        "d": {
            "token": token,
            "properties": { "os": "Linux", "browser": "Discord Client", "device": "" }
        }
    });
    socket.send(Message::text(identify.to_string())).await?;
    println!("-> IDENTIFY sent");

    let mut heartbeat = tokio::time::interval(Duration::from_millis(hello.heartbeat_interval));
    heartbeat.tick().await; // the first tick fires immediately, so skip it
    let mut last_sequence: Option<u64> = None;

    loop {
        tokio::select! {
            // Branch 1: it's time to send a heartbeat.
            _ = heartbeat.tick() => {
                let beat = json!({ "op": 1, "d": last_sequence });
                socket.send(Message::text(beat.to_string())).await?;
                println!("-> heartbeat (d = {last_sequence:?})");
            }

            // Branch 2: Discord sent us something.
            message = socket.next() => {
                let Some(message) = message else {
                    println!("Connection ended");
                    break;
                };
                let message = message?;
                if let Message::Close(frame) = &message {
                    println!("Discord closed the connection: {frame:?}");
                    break;
                }

                let payload: GatewayPayload = serde_json::from_str(message.to_text()?)?;
                if payload.s.is_some() {
                    last_sequence = payload.s;
                }

                match payload.op {
                    // op 0 = an event ("dispatch"); `t` says which one.
                    0 => match payload.t.unwrap_or_default().as_str() {
                        "READY" => {
                            let username = payload
                                .d
                                .as_ref()
                                .and_then(|d| d["user"]["username"].as_str())
                                .unwrap_or("?");
                            println!("<- READY: logged in as {username}");
                        }
                        "MESSAGE_CREATE" => {
                            let msg: MessageCreate = serde_json::from_value(payload.d.ok_or("message had no data")?)?;
                            println!("[{}] {}: {}", msg.channel_id, msg.author.username, msg.content);
                        }
                        other => println!("<- {other}"),
                    },
                    7 => println!("<- Discord asked us to reconnect"),
                    9 => println!("<- invalid session"),
                    11 => println!("<- heartbeat ACK"),
                    op => println!("<- op {op}"),
                }
            }
        }
    }
    Ok(())
}
