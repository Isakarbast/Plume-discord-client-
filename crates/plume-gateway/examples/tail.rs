//! Connect to Discord and print new messages as they arrive.
//!
//!     cargo run -p plume-gateway --example tail
//!     cargo run -p plume-gateway --example tail -- --list
//!     cargo run -p plume-gateway --example tail -- --dms
//!     cargo run -p plume-gateway --example tail -- --server "my server"
//!     cargo run -p plume-gateway --example tail -- --server "my server" --channel general
//!
//! Log in first with `cargo run -p plume-auth --example login`.
//! Set `RUST_LOG=debug` to see heartbeats and reconnects.

use plume_core::models::{Channel, ChannelKind, Message, User};
use plume_core::{Event, Profile, Snowflake, Store};
use plume_gateway::{Gateway, GatewayError};
use tokio::sync::mpsc;

const USAGE: &str = "\
Options:
  --list              list your servers and exit
  --dms               only show direct messages
  --server <name|id>  only show one server (part of the name is enough)
  --channel <name>    only show channels with this name (e.g. general)
  --help              show this help";

/// What the user asked for on the command line.
#[derive(Default)]
struct Options {
    list: bool,
    dms: bool,
    server: Option<String>,
    channel: Option<String>,
}

/// Which messages to print, once we know the server IDs (after READY).
enum Scope {
    Everything,
    Dms,
    Server(Snowflake),
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let options = parse_args().unwrap_or_else(|error| exit_with(&error));

    let token = match plume_auth::token_from_env() {
        Some(token) => token,
        None => tokio::task::spawn_blocking(plume_auth::keyring::load_token)
            .await??
            .ok_or("not logged in: run `cargo run -p plume-auth --example login` first")?,
    };

    let (events_tx, mut events) = mpsc::channel(256);
    let gateway = tokio::spawn(Gateway::new(token, events_tx).run());

    let mut store = Store::new(Profile::FEATHER);
    let mut scope = Scope::Everything;
    let channel_filter = options
        .channel
        .as_deref()
        .map(|name| name.trim_start_matches('#').to_lowercase());

    while let Some(event) = events.recv().await {
        if let Event::MessageCreate { message, author } = &event
            && let Some(place) = place_if_shown(&store, &scope, channel_filter.as_deref(), message)
        {
            println!("[{place}] {}: {}", author.display_name(), message.content);
        }

        let is_ready = matches!(event, Event::Ready { .. });
        let is_resumed = matches!(event, Event::Resumed);
        store.apply(event);

        if is_resumed {
            println!("(reconnected)");
        }
        if is_ready {
            let me = store.me().map_or("?", User::display_name);
            println!("Logged in as {me} ({} servers).", store.guilds().count());

            if options.list {
                print_servers(&store);
                return Ok(());
            }
            scope = resolve_scope(&store, &options).unwrap_or_else(|error| exit_with(&error));
            println!(
                "{}. Waiting for messages...",
                describe_scope(&store, &scope, &options)
            );
        }
    }

    // The channel only closes when the gateway stops, so find out why.
    match gateway.await? {
        Err(GatewayError::AuthenticationFailed) => {
            eprintln!(
                "Discord rejected the token. If it came from the keyring, log out and in again:"
            );
            eprintln!("  cargo run -p plume-auth --example login -- --logout");
        }
        Err(error) => eprintln!("gateway stopped: {error}"),
        Ok(()) => {}
    }
    Ok(())
}

/// Prints a message for the user (with real line breaks) and quits.
fn exit_with(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(2);
}

fn parse_args() -> Result<Options, String> {
    let mut options = Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--list" => options.list = true,
            "--dms" => options.dms = true,
            "--server" => options.server = Some(args.next().ok_or("--server needs a name or ID")?),
            "--channel" => options.channel = Some(args.next().ok_or("--channel needs a name")?),
            "--help" | "-h" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => return Err(format!("unknown option `{other}`\n\n{USAGE}")),
        }
    }
    if options.dms && options.server.is_some() {
        return Err("pick either --dms or --server, not both".to_owned());
    }
    Ok(options)
}

/// Turns `--dms` / `--server` into a concrete scope, now that we know the servers.
fn resolve_scope(store: &Store, options: &Options) -> Result<Scope, String> {
    if options.dms {
        return Ok(Scope::Dms);
    }
    let Some(query) = &options.server else {
        return Ok(Scope::Everything);
    };

    // An exact ID wins.
    if let Ok(id) = query.parse::<Snowflake>()
        && store.guild(id).is_some()
    {
        return Ok(Scope::Server(id));
    }

    let query = query.to_lowercase();
    let matches: Vec<_> = store
        .guilds()
        .filter(|g| g.name.to_lowercase().contains(&query))
        .collect();

    // If several match, an exact name match settles it ("Rust" vs "Rust Gamedev").
    let exact = matches.iter().find(|g| g.name.to_lowercase() == query);
    match (matches.as_slice(), exact) {
        (_, Some(guild)) => Ok(Scope::Server(guild.id)),
        ([guild], None) => Ok(Scope::Server(guild.id)),
        ([], None) => Err(format!(
            "no server matches \"{query}\". Try --list to see them all."
        )),
        (several, None) => {
            let names: Vec<String> = several
                .iter()
                .map(|g| format!("  {} ({})", g.name, g.id))
                .collect();
            Err(format!(
                "\"{query}\" matches several servers; be more specific or use the ID:\n{}",
                names.join("\n")
            ))
        }
    }
}

/// Where a message was sent (for printing), or `None` if the filters hide it.
fn place_if_shown(
    store: &Store,
    scope: &Scope,
    channel_filter: Option<&str>,
    message: &Message,
) -> Option<String> {
    let channel = store.channel(message.channel_id);
    let is_dm = channel.is_some_and(|c| matches!(c.kind, ChannelKind::Dm | ChannelKind::GroupDm));

    let in_scope = match scope {
        Scope::Everything => true,
        Scope::Dms => is_dm,
        Scope::Server(id) => channel.and_then(|c| c.guild_id) == Some(*id),
    };
    if !in_scope {
        return None;
    }
    if let Some(wanted) = channel_filter {
        let name = channel.and_then(|c| c.name.as_deref()).unwrap_or_default();
        if name.to_lowercase() != wanted {
            return None;
        }
    }

    Some(match channel {
        Some(channel) if is_dm => format!("DM {}", dm_label(store, channel)),
        Some(channel) => {
            let guild = channel
                .guild_id
                .and_then(|id| store.guild(id))
                .map_or("?", |g| g.name.as_str());
            format!("{guild} #{}", channel.name.as_deref().unwrap_or("?"))
        }
        None => format!("channel {}", message.channel_id),
    })
}

/// "with Alice" for a DM, or the group's name / members for a group DM.
fn dm_label(store: &Store, channel: &Channel) -> String {
    if let Some(name) = &channel.name
        && !name.is_empty()
    {
        return name.clone();
    }
    let names: Vec<&str> = channel
        .recipients
        .iter()
        .filter_map(|&id| store.user(id))
        .map(User::display_name)
        .collect();
    if names.is_empty() {
        "(unknown)".to_owned()
    } else {
        format!("with {}", names.join(", "))
    }
}

fn describe_scope(store: &Store, scope: &Scope, options: &Options) -> String {
    let base = match scope {
        Scope::Everything => "Showing everything".to_owned(),
        Scope::Dms => "Showing DMs only".to_owned(),
        Scope::Server(id) => {
            let name = store.guild(*id).map_or("?", |g| g.name.as_str());
            format!("Showing server \"{name}\"")
        }
    };
    match &options.channel {
        Some(channel) => format!("{base}, channel #{}", channel.trim_start_matches('#')),
        None => base,
    }
}

fn print_servers(store: &Store) {
    let mut guilds: Vec<_> = store.guilds().collect();
    guilds.sort_by_key(|g| g.name.to_lowercase());
    println!("\nYour servers:");
    for guild in guilds {
        println!("  {}  ({})", guild.name, guild.id);
    }
    println!("\nPick one with: --server \"<name>\"  (or the ID)");
}
