use std::{collections::BTreeMap, fs};
pub(crate) fn language() -> String {
    match std::env::var("MUSICX_LANGUAGE").as_deref() {
        Ok("en") => "en",
        _ => "zh-CN",
    }
    .into()
}
pub(crate) fn translations(id: &str) -> BTreeMap<String, String> {
    crate::mygo::plugin_root()
        .ok()
        .and_then(|root| {
            fs::read(
                root.join(id)
                    .join("locales")
                    .join(format!("{}.json", language())),
            )
            .ok()
        })
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
