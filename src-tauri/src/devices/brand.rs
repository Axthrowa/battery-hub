//! Brand identity — known vendors + automatic classification from product names.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct Brand(String);

impl Brand {
    pub fn new(id: impl Into<String>) -> Self {
        let raw = id.into();
        let slug = slugify(&raw);
        Self(if slug.is_empty() {
            "device".into()
        } else {
            slug
        })
    }

    pub fn label(&self) -> String {
        match self.0.as_str() {
            "razer" => "Razer".into(),
            "logitech" => "Logitech".into(),
            "ajazz" => "Ajazz".into(),
            "soundcore" | "anker" => "Soundcore".into(),
            "steelseries" => "SteelSeries".into(),
            "corsair" => "Corsair".into(),
            "hyperx" => "HyperX".into(),
            "sony" => "Sony".into(),
            "microsoft" => "Microsoft".into(),
            "apple" => "Apple".into(),
            "samsung" => "Samsung".into(),
            "jbl" => "JBL".into(),
            "bose" => "Bose".into(),
            "xiaomi" | "redmi" => "Xiaomi".into(),
            "huawei" => "Huawei".into(),
            "hp" => "HP".into(),
            "dell" => "Dell".into(),
            "lenovo" => "Lenovo".into(),
            "asus" | "rog" => "ASUS".into(),
            "keychron" => "Keychron".into(),
            "glorious" => "Glorious".into(),
            "pulsar" => "Pulsar".into(),
            "varmilo" => "Varmilo".into(),
            "bytech" | "by-tech" => "BY Tech".into(),
            "generic" | "device" => "Device".into(),
            other => title_case(other),
        }
    }

    pub fn razer() -> Self {
        Self::new("razer")
    }
    pub fn logitech() -> Self {
        Self::new("logitech")
    }
    pub fn ajazz() -> Self {
        Self::new("ajazz")
    }
    pub fn aula() -> Self {
        Self::new("aula")
    }
    pub fn soundcore() -> Self {
        Self::new("soundcore")
    }
    pub fn generic() -> Self {
        Self::new("generic")
    }

    /// USB vendor IDs, which say who built the hardware even when the strings
    /// do not. A 2.4 GHz receiver almost always names itself after the radio
    /// ("2.4G Wireless Receiver", "USB Gaming Mouse") and leaves the
    /// manufacturer field to the chipset house, so guessing a brand from those
    /// strings is how a keyboard ends up filed under "Cx" or "4G".
    const VENDOR_IDS: &[(u16, &str)] = &[
        (0x1532, "razer"),
        (0x046D, "logitech"),
        (0x3151, "ajazz"),
        (0x3554, "aula"),
        (0x258A, "aula"),
        (0x372E, "aula"),
        (0x1038, "steelseries"),
        (0x1B1C, "corsair"),
        (0x0951, "hyperx"),
        (0x03F0, "hp"),
        (0x045E, "microsoft"),
        (0x05AC, "apple"),
        (0x054C, "sony"),
        (0x0B05, "asus"),
        (0x28DA, "glorious"),
        (0x3367, "keychron"),
        (0x3434, "keychron"),
        (0x320F, "akko"),
        (0x24AE, "redragon"),
        (0x24F0, "redragon"),
        (0x2717, "xiaomi"),
        (0x25A7, "rapoo"),
        (0x2EA8, "pulsar"),
        (0x291A, "anker"),
    ];

    /// Who built the hardware, from the USB vendor ID alone.
    pub fn from_vendor_id(vendor_id: u16) -> Option<Self> {
        Self::VENDOR_IDS
            .iter()
            .find(|(id, _)| *id == vendor_id)
            .map(|(_, slug)| Self::new(*slug))
    }

    /// The vendor ID when it is known, the strings otherwise. Prefer this over
    /// `classify` wherever a vendor ID is in reach.
    pub fn identify(vendor_id: u16, manufacturer: &str, product: &str) -> Self {
        Self::from_vendor_id(vendor_id).unwrap_or_else(|| Self::classify(manufacturer, product))
    }

    /// Infer brand from manufacturer / product strings (any vendor).
    pub fn classify(manufacturer: &str, product: &str) -> Self {
        let hay = format!("{manufacturer} {product}").to_ascii_lowercase();
        const RULES: &[(&str, &str)] = &[
            ("razer", "razer"),
            ("logitech", "logitech"),
            ("logi ", "logitech"),
            ("ajazz", "ajazz"),
            ("soundcore", "soundcore"),
            ("anker", "soundcore"),
            ("steelseries", "steelseries"),
            ("corsair", "corsair"),
            ("hyperx", "hyperx"),
            ("kingston", "hyperx"),
            ("sony", "sony"),
            ("microsoft", "microsoft"),
            ("apple", "apple"),
            ("samsung", "samsung"),
            ("jbl", "jbl"),
            ("harman", "jbl"),
            ("bose", "bose"),
            ("xiaomi", "xiaomi"),
            ("redmi", "xiaomi"),
            ("huawei", "huawei"),
            ("keychron", "keychron"),
            ("aula", "aula"),
            ("redragon", "redragon"),
            ("rapoo", "rapoo"),
            ("akko", "akko"),
            ("glorious", "glorious"),
            ("pulsar", "pulsar"),
            ("varmilo", "varmilo"),
            ("by tech", "bytech"),
            ("asus", "asus"),
            ("rog ", "asus"),
            ("lenovo", "lenovo"),
            ("dell", "dell"),
            ("hewlett", "hp"),
            (" hp ", "hp"),
        ];
        for (needle, id) in RULES {
            if hay.contains(needle) {
                return Self::new(*id);
            }
        }
        // Fall back to the first token that could actually be a brand. A
        // generic OEM string like "2.4G Wireless Receiver" has none, and
        // inventing one from it produced brands such as "4g".
        for token in product.split(|c: char| !c.is_ascii_alphanumeric()) {
            if token.len() < 3 || is_noise_token(token) {
                continue;
            }
            // A brand does not start with a digit ("2", "4G", "8K").
            if !token.starts_with(|c: char| c.is_ascii_alphabetic()) {
                continue;
            }
            return Self::new(token);
        }
        Self::generic()
    }
}

/// Words that describe the hardware, not who made it.
fn is_noise_token(token: &str) -> bool {
    const NOISE: &[&str] = &[
        "usb", "hid", "wireless", "receiver", "dongle", "adapter", "gaming", "keyboard", "mouse",
        "headset", "headphone", "earbuds", "device", "composite", "control", "controls", "rgb",
        "ghz", "bluetooth", "generic", "input",
    ];
    NOISE.iter().any(|word| token.eq_ignore_ascii_case(word))
}

fn slugify(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

fn title_case(slug: &str) -> String {
    slug.split('-')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut c = p.chars();
            match c.next() {
                Some(f) => f.to_ascii_uppercase().to_string() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::Brand;

    #[test]
    fn known_vendors_win_over_token_guessing() {
        assert_eq!(Brand::classify("Logitech", "USB Receiver").label(), "Logitech");
        assert_eq!(Brand::classify("", "AJAZZ 2.4G 8K").label(), "Ajazz");
        assert_eq!(Brand::classify("", "Aula F75 Keyboard").label(), "Aula");
        assert_eq!(Brand::classify("", "Razer BlackShark V2").label(), "Razer");
    }

    #[test]
    fn generic_oem_strings_do_not_invent_a_brand() {
        // Used to yield "4g" from the "4G" token and show it as the device name.
        for product in [
            "2.4G Wireless Receiver",
            "2.4G Wireless Device",
            "USB Gaming Keyboard",
            "Wireless Dongle",
            "",
        ] {
            assert_eq!(
                Brand::classify("", product).label(),
                "Device",
                "product: {product}"
            );
        }
    }

    #[test]
    fn unknown_but_real_brands_are_title_cased() {
        assert_eq!(Brand::classify("", "Keychron K8 Pro").label(), "Keychron");
        assert_eq!(Brand::classify("", "Zuoya GMK87").label(), "Zuoya");
    }
}
