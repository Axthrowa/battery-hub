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
    pub fn soundcore() -> Self {
        Self::new("soundcore")
    }
    pub fn generic() -> Self {
        Self::new("generic")
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
        // First meaningful token of the product name.
        for token in product.split(|c: char| !c.is_ascii_alphanumeric()) {
            if token.len() >= 2 && !token.eq_ignore_ascii_case("usb") && !token.eq_ignore_ascii_case("hid")
            {
                return Self::new(token);
            }
        }
        Self::generic()
    }
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
