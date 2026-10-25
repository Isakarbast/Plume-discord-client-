//! Avatars and other images, loaded in the background and shared through a
//! small texture cache, so the same avatar on 50 messages is decoded once.

use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};
use plume_core::models::{Guild, Status, User};
use plume_media::{Image, urls};

use crate::app::AppState;
use crate::state::bridge;

type Key = (String, u32);
type Waiter = Box<dyn FnOnce(&gdk::Texture)>;

/// Recently used textures, plus the callbacks waiting for ones in flight.
/// Bounded by bytes, not count: one photo weighs as much as 300 avatars.
pub struct TextureCache {
    budget: usize,
    used: usize,
    textures: HashMap<Key, gdk::Texture>,
    order: VecDeque<Key>,
    pending: HashMap<Key, Vec<Waiter>>,
}

fn weight(texture: &gdk::Texture) -> usize {
    texture.width() as usize * texture.height() as usize * 4
}

impl TextureCache {
    pub fn new(budget_bytes: usize) -> Self {
        TextureCache {
            budget: budget_bytes,
            used: 0,
            textures: HashMap::new(),
            order: VecDeque::new(),
            pending: HashMap::new(),
        }
    }

    fn insert(&mut self, key: Key, texture: gdk::Texture) {
        self.used += weight(&texture);
        // Oldest out first. Widgets still showing one keep their own reference.
        while self.used > self.budget {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(gone) = self.textures.remove(&oldest) {
                self.used -= weight(&gone);
            }
        }
        self.order.push_back(key.clone());
        if let Some(old) = self.textures.insert(key, texture) {
            self.used -= weight(&old);
        }
    }
}

/// Pixels per point on the screen (1, or 2 on HiDPI). Images are fetched
/// at exactly this density: a 2× avatar on a 1× screen is 4× the memory.
pub fn scale() -> u32 {
    thread_local! {
        static SCALE: u32 = gdk::Display::default()
            .map(|display| {
                let monitors = display.monitors();
                (0..monitors.n_items())
                    .filter_map(|i| monitors.item(i).and_downcast::<gdk::Monitor>())
                    .map(|m| m.scale_factor())
                    .max()
                    .unwrap_or(1)
            })
            .unwrap_or(1)
            .clamp(1, 3) as u32;
    }
    SCALE.with(|s| *s)
}

pub fn texture_from(image: &Image) -> gdk::Texture {
    let bytes = glib::Bytes::from(&image.rgba[..]);
    gdk::MemoryTexture::new(
        image.width as i32,
        image.height as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &bytes,
        image.width as usize * 4,
    )
    .upcast()
}

/// Calls `done` with the image at `url` (scaled to `size`), right away if
/// it's cached, otherwise once it's downloaded. Does nothing in demo mode
/// or if the download fails.
pub fn load(
    state: &Rc<AppState>,
    url: String,
    size: u32,
    done: impl FnOnce(&gdk::Texture) + 'static,
) {
    let Some(media) = state.media.clone() else {
        return;
    };
    let key = (url, size);
    {
        let mut cache = state.textures.borrow_mut();
        if let Some(texture) = cache.textures.get(&key) {
            done(texture);
            return;
        }
        if let Some(waiters) = cache.pending.get_mut(&key) {
            waiters.push(Box::new(done)); // already downloading: just wait for it
            return;
        }
        cache.pending.insert(key.clone(), vec![Box::new(done)]);
    }

    let state = state.clone();
    let url = key.0.clone();
    bridge::spawn(
        async move { media.image(&url, size).await },
        move |result| {
            let waiters = state
                .textures
                .borrow_mut()
                .pending
                .remove(&key)
                .unwrap_or_default();
            match result {
                Ok(image) => {
                    let texture = texture_from(&image);
                    state.textures.borrow_mut().insert(key, texture.clone());
                    for waiter in waiters {
                        waiter(&texture);
                    }
                }
                Err(error) => tracing::debug!("image {} failed: {error}", key.0),
            }
        },
    );
}

/// A round avatar: initials at first, the real picture once it's loaded.
pub fn user_avatar(state: &Rc<AppState>, user: &User, size: i32) -> adw::Avatar {
    let avatar = adw::Avatar::new(size, Some(user.display_name()), true);
    let weak = avatar.downgrade();
    load(
        state,
        urls::avatar(user, size as u32 * scale()),
        size as u32 * scale(),
        move |texture| {
            if let Some(avatar) = weak.upgrade() {
                avatar.set_custom_image(Some(texture));
            }
        },
    );
    avatar
}

pub fn guild_avatar(state: &Rc<AppState>, guild: &Guild, size: i32) -> adw::Avatar {
    let avatar = adw::Avatar::new(size, Some(&guild.name), true);
    if let Some(url) = urls::guild_icon(guild, size as u32 * scale()) {
        let weak = avatar.downgrade();
        load(state, url, size as u32 * scale(), move |texture| {
            if let Some(avatar) = weak.upgrade() {
                avatar.set_custom_image(Some(texture));
            }
        });
    }
    avatar
}

/// CSS class for a status dot.
pub fn status_class(status: Status) -> &'static str {
    match status {
        Status::Online => "status-online",
        Status::Idle => "status-idle",
        Status::Dnd => "status-dnd",
        Status::Offline => "status-offline",
    }
}

/// A user's avatar with their status dot in the corner.
pub fn with_status(state: &Rc<AppState>, user: &User, size: i32) -> gtk::Overlay {
    let status = state.store.borrow().presence(user.id).status;
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&user_avatar(state, user, size)));
    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class("status-dot");
    dot.add_css_class(status_class(status));
    dot.set_halign(gtk::Align::End);
    dot.set_valign(gtk::Align::End);
    dot.set_tooltip_text(Some(status.label()));
    overlay.add_overlay(&dot);
    overlay.set_valign(gtk::Align::Center);
    overlay
}
