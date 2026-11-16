use std::{collections::BTreeMap, fs};
pub(crate) fn language() -> String {
    musicx_core::protocol::i18n::refresh();
    musicx_core::protocol::i18n::locale().tag().into()
}
pub(crate) fn translations(id: &str) -> BTreeMap<String, String> {
    let Ok(root) = crate::mygo::plugin_root() else {
        return BTreeMap::new();
    };
    translations_at(&root.join(id).join("locales"), &language())
}

fn translations_at(directory: &std::path::Path, language: &str) -> BTreeMap<String, String> {
    let read = |language: &str| -> BTreeMap<String, String> {
        fs::read(directory.join(format!("{language}.json")))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    };
    let fallback = if language == "en" { "zh-CN" } else { "en" };
    let mut translations = read(fallback);
    translations.extend(read(language));
    translations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_plugin_locale_overlays_fallback_without_losing_missing_keys() {
        let dir =
            std::env::temp_dir().join(format!("musicx-plugin-locales-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("en.json"), r#"{"name":"Source", "help":"Help"}"#).unwrap();
        fs::write(dir.join("zh-CN.json"), r#"{"name":"来源"}"#).unwrap();
        let chinese = translations_at(&dir, "zh-CN");
        assert_eq!(chinese["name"], "来源");
        assert_eq!(chinese["help"], "Help");
        assert_eq!(translations_at(&dir, "en")["name"], "Source");
        fs::write(dir.join("zh-CN.json"), "invalid json").unwrap();
        assert_eq!(translations_at(&dir, "zh-CN")["name"], "Source");
        fs::remove_file(dir.join("en.json")).unwrap();
        assert!(translations_at(&dir, "en").is_empty());
        fs::remove_dir_all(dir).unwrap();
    }
}
