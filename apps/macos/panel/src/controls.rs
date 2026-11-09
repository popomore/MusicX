//! Shared controls for compact detail flows.
use nsh_base::{button, theme};
use nsh_core::prelude::*;

/// Small, virtualized row images share the existing bounded NSH image cache.
pub fn artwork(
    url: impl Fn() -> Option<String> + 'static,
    label: impl Fn() -> String + 'static,
    size: f32,
) -> impl IntoElement {
    use nsh_core::element::ImageSrc;
    const FALLBACK: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect width="32" height="32" rx="4" fill="#e9e7e1"/><path d="M18 8v14a4 4 0 1 1-2-3.46V10l8-2v4z" fill="#939087"/></svg>"##;
    let source = computed(move || {
        url()
            .and_then(|value| thumbnail_url(&value, (size * 2.0) as u32))
            .map_or_else(|| ImageSrc::svg_markup(FALLBACK), ImageSrc::from)
    });
    image(move || source.get())
        .w(px(size))
        .min_w(px(size))
        .h(px(size))
        .rounded(px(5.0))
        .object_fit(ResizeMode::Cover)
        .placeholder(ImageSrc::svg_markup(FALLBACK))
        .label(move || SharedString::from(label()))
}
fn thumbnail_url(value: &str, pixels: u32) -> Option<String> {
    let mut url = reqwest::Url::parse(value).ok()?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    if url.domain().is_some_and(|host| host.ends_with(".126.net")) {
        let pairs: Vec<_> = url
            .query_pairs()
            .filter(|(k, _)| k != "param")
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        url.query_pairs_mut()
            .clear()
            .extend_pairs(pairs)
            .append_pair("param", &format!("{pixels}y{pixels}"));
    }
    Some(url.to_string())
}

pub fn caption(value: impl IntoReactive<SharedString>) -> impl IntoElement {
    text(value)
        .text_size(px(11.0))
        .number_of_lines(3)
        .text_color(move || theme().muted_foreground)
}
pub fn heading(value: impl IntoReactive<SharedString>) -> impl IntoElement {
    text(value)
        .text_size(px(18.0))
        .font_weight(FontWeight::SEMIBOLD)
        .number_of_lines(2)
        .text_color(move || theme().foreground)
}
pub fn section(value: &'static str) -> impl IntoElement {
    text(value)
        .text_size(px(11.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(move || theme().muted_foreground)
}
pub fn action(
    label: &'static str,
    disabled: impl Fn() -> bool + 'static,
    press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .accessibility_label(Some(label.into()))
        .disabled(disabled)
        .on_press(press)
        .px(px(8.0))
        .py(px(7.0))
        .rounded(px(5.0))
        .hover(|s| s.bg(theme().muted))
        .active(|s| s.opacity(0.65))
        .disabled_style(|s| s.opacity(0.4))
        .child(
            text(label)
                .text_size(px(11.0))
                .text_color(move || theme().foreground),
        )
}
pub fn primary(
    label: &'static str,
    disabled: impl Fn() -> bool + 'static,
    press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .accessibility_label(Some(label.into()))
        .disabled(disabled)
        .on_press(press)
        .px(px(13.0))
        .py(px(9.0))
        .rounded(px(6.0))
        .bg(move || theme().foreground)
        .hover(|s| s.opacity(0.86))
        .active(|s| s.opacity(0.65))
        .disabled_style(|s| s.opacity(0.35))
        .child(
            text(label)
                .text_size(px(12.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().background),
        )
}
pub fn source_name(id: &str, fallback: &str) -> String {
    match id.strip_prefix("mygo:").unwrap_or(id) {
        "netease" => "网易云音乐",
        "bodian" => "波点音乐",
        "kugou-lite" => "酷狗概念版",
        _ => fallback,
    }
    .to_owned()
}

pub fn source_badge(id: impl Fn() -> String + 'static) -> impl IntoElement {
    use nsh_core::element::ImageSrc;
    view()
        .w(px(28.0))
        .min_w(px(28.0))
        .h(px(28.0))
        .rounded(px(6.0))
        .bg(move || theme().muted.opacity(0.65))
        .items_center()
        .justify_center()
        .child(
            svg(move || {
                let id = id();
                ImageSrc::svg_markup(match id.strip_prefix("mygo:").unwrap_or(&id) {
                    "netease" => r##"<svg viewBox="0 0 24 24"><path d="M12 4a8 8 0 1 0 8 8m-1-5-5 2v5a2 2 0 1 1-2-2" fill="none" stroke="#b43b48" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>"##,
                    "bodian" => r##"<svg viewBox="0 0 24 24"><path d="M4 11v3m4-6v9m4-13v17m4-13v9m4-6v3" fill="none" stroke="#876327" stroke-width="2" stroke-linecap="round"/></svg>"##,
                    "kugou" | "kugou-lite" => r##"<svg viewBox="0 0 24 24"><path d="M5 14v-2a7 7 0 0 1 14 0v2M5 13h2a1 1 0 0 1 1 1v4a1 1 0 0 1-1 1H6a2 2 0 0 1-2-2v-2a2 2 0 0 1 1-2Zm14 0h-2a1 1 0 0 0-1 1v4a1 1 0 0 0 1 1h1a2 2 0 0 0 2-2v-2a2 2 0 0 0-1-2Z" fill="none" stroke="#336e91" stroke-width="1.8" stroke-linecap="round"/></svg>"##,
                    _ => r#"<svg viewBox="0 0 24 24"><path d="M11 16V6l7-2v10m-7 2a2.5 2.5 0 1 1-2.5-2.5H11m7 .5a2.5 2.5 0 1 1-2.5-2.5H18" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>"#,
                })
            })
            .w(px(16.0))
            .h(px(16.0))
            .text_color(move || theme().muted_foreground),
        )
}
