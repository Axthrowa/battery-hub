//! What a device is, when nobody has given it artwork.
//!
//! A receiver reports a generic product string and a brand the app may have no
//! logo for, which leaves every such device wearing the same placeholder. The
//! shape of the thing is knowable though: the readers that speak a vendor
//! protocol know what they are talking to, HID says so outright for anything
//! plugged in directly, and the rest is legible in the name.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Keyboard,
    Mouse,
    Headset,
    Device,
}

impl DeviceKind {
    /// HID says what a directly attached device is. A 2.4 GHz receiver publishes
    /// a collection per peripheral it can pair with, so this only settles the
    /// question for devices that speak for themselves.
    pub fn from_usage(usage_page: u16, usage: u16) -> Option<Self> {
        match (usage_page, usage) {
            (0x01, 0x06) => Some(Self::Keyboard),
            (0x01, 0x02) => Some(Self::Mouse),
            (0x0B, _) => Some(Self::Headset),
            _ => None,
        }
    }

    /// Last resort: the product name. Matched on whole words where a fragment
    /// would be ambiguous — "kb" is a keyboard, "kbd" inside a longer word is
    /// not necessarily anything.
    pub fn from_name(name: &str) -> Self {
        let hay = name.to_ascii_lowercase();
        const HEADSET: &[&str] = &[
            "headset", "headphone", "earphone", "earbud", "buds", "airpods", "kulaklik",
            "blackshark", "kraken", "barracuda", "nari", "soundcore", "select", "liberty",
            "life q", "wh-", "wf-",
        ];
        const KEYBOARD: &[&str] = &["keyboard", "klavye", "keeb", "keychron", "keydous"];
        const MOUSE: &[&str] = &["mouse", "mice", "fare", "superlight", "superstrike", "deathadder", "viper", "basilisk"];
        if HEADSET.iter().any(|hint| hay.contains(hint)) {
            return Self::Headset;
        }
        if KEYBOARD.iter().any(|hint| hay.contains(hint)) {
            return Self::Keyboard;
        }
        if MOUSE.iter().any(|hint| hay.contains(hint)) {
            return Self::Mouse;
        }
        Self::Device
    }

    /// What HID knows, falling back to what the name suggests.
    pub fn infer(usage_page: u16, usage: u16, name: &str) -> Self {
        Self::from_usage(usage_page, usage).unwrap_or_else(|| Self::from_name(name))
    }
}
