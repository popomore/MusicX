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
            .and_then(|value| thumbnail_url(&value))
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
fn thumbnail_url(value: &str) -> Option<String> {
    let url = reqwest::Url::parse(value).ok()?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return None;
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
    if fallback.trim().is_empty() {
        id.strip_prefix("mygo:").unwrap_or(id).to_owned()
    } else {
        fallback.to_owned()
    }
}

pub fn source_badge(id: impl Fn() -> String + 'static) -> impl IntoElement {
    view()
        .w(px(28.0))
        .min_w(px(28.0))
        .h(px(28.0))
        .rounded(px(6.0))
        .bg(move || theme().muted.opacity(0.65))
        .items_center()
        .justify_center()
        .child(
            text(move || {
                let id = id();
                id.strip_prefix("mygo:")
                    .unwrap_or(&id)
                    .chars()
                    .next()
                    .map(|letter| letter.to_uppercase().to_string())
                    .unwrap_or_else(|| "♪".into())
            })
            .text_size(px(12.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(move || theme().muted_foreground),
        )
}
