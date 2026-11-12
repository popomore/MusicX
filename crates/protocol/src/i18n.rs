//! Shared UI language, independent of playback and stored music metadata.
use std::{
    collections::BTreeMap,
    io::Write,
    path::Path,
    sync::{
        LazyLock,
        atomic::{AtomicU8, Ordering},
    },
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    System,
    SimplifiedChinese,
    English,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Locale {
    SimplifiedChinese,
    English,
}

impl Locale {
    pub fn tag(self) -> &'static str {
        match self {
            Self::SimplifiedChinese => "zh-CN",
            Self::English => "en",
        }
    }

    fn matching(tag: &str) -> Option<Self> {
        match tag
            .trim()
            .split(['-', '_', '.'])
            .next()?
            .to_ascii_lowercase()
            .as_str()
        {
            "zh" => Some(Self::SimplifiedChinese),
            "en" => Some(Self::English),
            _ => None,
        }
    }
}

impl Language {
    pub const ALL: [Self; 3] = [Self::System, Self::SimplifiedChinese, Self::English];

    pub fn label(self) -> &'static str {
        match self {
            Self::System => text("跟随系统"),
            Self::SimplifiedChinese => "简体中文",
            Self::English => "English",
        }
    }

    pub fn resolve(self, preferred: impl IntoIterator<Item = impl AsRef<str>>) -> Locale {
        match self {
            Self::SimplifiedChinese => Locale::SimplifiedChinese,
            Self::English => Locale::English,
            Self::System => preferred
                .into_iter()
                .find_map(|tag| Locale::matching(tag.as_ref()))
                .unwrap_or(Locale::English),
        }
    }

    pub fn load() -> Self {
        crate::data_dir().map_or(Self::default(), |dir| Self::read(&dir))
    }

    pub fn save(self) -> std::io::Result<()> {
        let dir = crate::data_dir().ok_or_else(|| std::io::Error::other(text("找不到配置目录")))?;
        self.write(&dir)?;
        refresh();
        Ok(())
    }

    fn read(dir: &Path) -> Self {
        match std::fs::read_to_string(dir.join("language"))
            .as_deref()
            .map(str::trim)
        {
            Ok("zh-CN") => Self::SimplifiedChinese,
            Ok("en") => Self::English,
            _ => Self::System,
        }
    }

    fn write(self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let temp = dir.join(format!(".language.{}", std::process::id()));
        let result = (|| {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temp)?;
            file.write_all(match self {
                Self::System => b"system\n",
                Self::SimplifiedChinese => b"zh-CN\n",
                Self::English => b"en\n",
            })?;
            file.sync_all()?;
            std::fs::rename(&temp, dir.join("language"))
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temp);
        }
        result
    }
}

fn preferred_languages() -> Vec<String> {
    #[cfg(target_vendor = "apple")]
    {
        objc2_foundation::NSLocale::preferredLanguages()
            .iter()
            .map(|s| s.to_string())
            .collect()
    }
    #[cfg(not(target_vendor = "apple"))]
    {
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .into_iter()
            .filter_map(|key| std::env::var(key).ok())
            .filter(|s| !s.is_empty())
            .take(1)
            .collect()
    }
}

fn selected_locale() -> Locale {
    // Retain the existing override for plugin users and isolated QA runs.
    resolve_preference(
        Language::load(),
        std::env::var("MUSICX_LANGUAGE").ok().as_deref(),
        preferred_languages(),
    )
}

fn resolve_preference(
    preference: Language,
    override_tag: Option<&str>,
    preferred: impl IntoIterator<Item = impl AsRef<str>>,
) -> Locale {
    override_tag
        .and_then(Locale::matching)
        .unwrap_or_else(|| preference.resolve(preferred))
}

static CURRENT: LazyLock<AtomicU8> = LazyLock::new(|| AtomicU8::new(selected_locale() as u8));
static ENGLISH: LazyLock<BTreeMap<String, String>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("locales/en.json")).expect("valid bundled English catalog")
});

pub fn locale() -> Locale {
    match CURRENT.load(Ordering::Relaxed) {
        0 => Locale::SimplifiedChinese,
        _ => Locale::English,
    }
}

/// Reload the shared preference at a process boundary or on the UI's status tick.
pub fn refresh() -> bool {
    let selected = selected_locale() as u8;
    CURRENT.swap(selected, Ordering::Relaxed) != selected
}

pub fn text(source: &'static str) -> &'static str {
    text_for(locale(), source)
}

pub fn text_for(locale: Locale, source: &'static str) -> &'static str {
    match locale {
        Locale::SimplifiedChinese => source,
        Locale::English => ENGLISH.get(source).map_or(source, String::as_str),
    }
}

pub fn plural(count: u64, one: &'static str, other: &'static str) -> String {
    plural_for(locale(), count, one, other)
}

fn plural_for(locale: Locale, count: u64, one: &'static str, other: &'static str) -> String {
    format(
        text_for(locale, if count == 1 { one } else { other }),
        &[("", count.to_string())],
    )
}

pub fn tracks(count: usize) -> String {
    plural(count as u64, "1 首", "{} 首")
}

pub fn playlists(count: usize) -> String {
    plural(count as u64, "1 张歌单", "{} 张歌单")
}

pub fn relative_time(seconds: u64) -> String {
    match seconds {
        0..=59 => text("刚刚").into(),
        60..=3599 => plural(seconds / 60, "1 分钟前", "{} 分钟前"),
        3600..=86399 => plural(seconds / 3600, "1 小时前", "{} 小时前"),
        _ => plural(seconds / 86400, "1 天前", "{} 天前"),
    }
}

/// Substitute positional or named parameters once; parameter values remain opaque.
pub fn format(template: &str, arguments: &[(&str, String)]) -> String {
    let mut result = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    let mut position = 0;
    while let Some(ch) = chars.next() {
        match (ch, chars.peek()) {
            ('{', Some('{')) | ('}', Some('}')) => {
                result.push(ch);
                chars.next();
            }
            ('{', _) => {
                let mut key = String::new();
                for next in chars.by_ref() {
                    if next == '}' {
                        break;
                    }
                    key.push(next);
                }
                let value = if key.is_empty() {
                    let value = arguments
                        .iter()
                        .filter(|(name, _)| name.is_empty())
                        .nth(position);
                    position += 1;
                    value
                } else {
                    arguments.iter().find(|(name, _)| *name == key)
                };
                if let Some((_, value)) = value {
                    result.push_str(value);
                } else {
                    result.push('{');
                    result.push_str(&key);
                    result.push('}');
                }
            }
            _ => result.push(ch),
        }
    }
    result
}

/// Chinese source messages act as gettext-style keys. Parameters must be explicit.
#[macro_export]
macro_rules! tr {
    ($source:literal) => { $crate::i18n::text($source) };
    ($source:literal $(, $value:expr)* $(; $($name:ident = $named:expr),+)? $(,)?) => {
        $crate::i18n::format($crate::i18n::text($source), &[
            $(("", ($value).to_string()),)*
            $($( (stringify!($name), ($named).to_string()), )+)?
        ])
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferred_languages_regions_and_fallback() {
        assert_eq!(
            Language::System.resolve(["fr-FR", "zh-Hans-CN", "en-US"]),
            Locale::SimplifiedChinese
        );
        assert_eq!(
            Language::System.resolve(["en_GB.UTF-8", "zh-CN"]),
            Locale::English
        );
        assert_eq!(
            Language::System.resolve(["ZH_tw"]),
            Locale::SimplifiedChinese
        );
        assert_eq!(
            Language::System.resolve(["ja-JP", "fr-FR"]),
            Locale::English
        );
        assert_eq!(
            Language::System.resolve(Vec::<String>::new()),
            Locale::English
        );
        assert_eq!(Language::English.resolve(["zh-CN"]), Locale::English);
        assert_eq!(
            Language::SimplifiedChinese.resolve(["en-US"]),
            Locale::SimplifiedChinese
        );
        assert_eq!(
            resolve_preference(Language::English, Some("zh_CN"), ["en-US"]),
            Locale::SimplifiedChinese
        );
        assert_eq!(
            resolve_preference(Language::SimplifiedChinese, Some("invalid"), ["en-US"]),
            Locale::SimplifiedChinese
        );
    }

    #[test]
    fn saved_language_round_trip_and_invalid_preference() {
        let dir = std::env::temp_dir().join(format!("musicx-language-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(Language::read(&dir), Language::System);
        for language in Language::ALL {
            language.write(&dir).unwrap();
            assert_eq!(Language::read(&dir), language);
        }
        std::fs::write(dir.join("language"), "unknown\n").unwrap();
        assert_eq!(Language::read(&dir), Language::System);
        std::fs::write(dir.join("language"), [0xff]).unwrap();
        assert_eq!(Language::read(&dir), Language::System);
        std::fs::remove_file(dir.join("language")).unwrap();
        std::fs::create_dir(dir.join("language")).unwrap();
        assert!(Language::English.write(&dir).is_err());
        assert!(dir.join("language").is_dir());
        assert!(
            !dir.join(format!(".language.{}", std::process::id()))
                .exists()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn substitution_allows_reordering_and_keeps_user_text_opaque() {
        let arguments = [("name", "你好 {count} 🎵".into()), ("count", "2".into())];
        assert_eq!(
            format("{count}: {name}, {name}", &arguments),
            "2: 你好 {count} 🎵, 你好 {count} 🎵"
        );
        assert_eq!(
            format("{{{}}} {}", &[("", "a".into()), ("", "b".into())]),
            "{a} b"
        );
        assert_eq!(
            text_for(Locale::English, "missing message"),
            "missing message"
        );
        assert_eq!(text_for(Locale::SimplifiedChinese, "设置"), "设置");
        assert_eq!(text_for(Locale::English, "设置"), "Settings");
    }

    #[test]
    fn counts_and_relative_time_have_english_singulars() {
        for (n, expected) in [(0, "0 tracks"), (1, "1 track"), (2, "2 tracks")] {
            assert_eq!(plural_for(Locale::English, n, "1 首", "{} 首"), expected);
        }
        assert_eq!(
            plural_for(Locale::SimplifiedChinese, 1, "1 首", "{} 首"),
            "1 首"
        );
        assert_eq!(
            plural_for(Locale::English, 1, "1 张歌单", "{} 张歌单"),
            "1 playlist"
        );
        assert_eq!(
            plural_for(Locale::English, 1, "1 分钟前", "{} 分钟前"),
            "1 minute ago"
        );
        assert_eq!(
            plural_for(Locale::English, 2, "1 小时前", "{} 小时前"),
            "2 hours ago"
        );
        assert_eq!(
            plural_for(Locale::English, 1, "1 天前", "{} 天前"),
            "1 day ago"
        );
    }

    #[test]
    fn favorite_display_name_preserves_stored_and_user_names() {
        let mut list: crate::LocalPlaylist = serde_json::from_value(serde_json::json!({
            "id": "liked", "name": "我喜欢", "favorite": true, "pinned": false,
            "revision": 1, "tracks": [], "imported_from": null
        }))
        .unwrap();
        let stored = serde_json::to_value(&list).unwrap();
        assert_eq!(list.display_name(), text("我喜欢"));
        assert_eq!(serde_json::to_value(&list).unwrap(), stored);
        list.favorite = false;
        list.name = "我喜欢 🎵 {name}".into();
        assert_eq!(list.display_name(), "我喜欢 🎵 {name}");
    }

    fn placeholders(template: &str) -> BTreeMap<String, usize> {
        let mut result = BTreeMap::new();
        let mut remaining = template;
        while let Some(start) = remaining.find('{') {
            let rest = &remaining[start + 1..];
            let end = rest.find('}').expect("closed message placeholder");
            *result.entry(rest[..end].to_owned()).or_default() += 1;
            remaining = &rest[end + 1..];
        }
        result
    }

    #[test]
    fn catalog_preserves_all_message_parameters() {
        for (source, english) in ENGLISH.iter() {
            assert!(!english.trim().is_empty(), "empty translation: {source}");
            assert_eq!(
                placeholders(source),
                placeholders(english),
                "placeholder mismatch: {source}"
            );
        }
    }
}
