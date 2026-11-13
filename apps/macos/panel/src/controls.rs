//! Shared controls for compact detail flows.
use crate::protocol::tr;
use nsh_base::{button, popover, theme};
use nsh_core::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    Warning,
    Error,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Notice {
    pub kind: NoticeKind,
    pub message: String,
}

impl Notice {
    pub fn warning(message: impl Into<String>) -> Self {
        Self {
            kind: NoticeKind::Warning,
            message: message.into(),
        }
    }
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            kind: NoticeKind::Error,
            message: message.into(),
        }
    }
}

fn notice_icon(kind: impl Fn() -> NoticeKind + 'static) -> impl IntoElement {
    use nsh_core::element::ImageSrc;
    const WARNING: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M10.3 4.2 2.6 17.5a2 2 0 0 0 1.7 3h15.4a2 2 0 0 0 1.7-3L13.7 4.2a2 2 0 0 0-3.4 0Z"/><path d="M12 9v4"/><circle cx="12" cy="17" r=".8" fill="currentColor" stroke="none"/></svg>"#;
    const ERROR: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"><circle cx="12" cy="12" r="9"/><path d="m9 9 6 6m0-6-6 6"/></svg>"#;
    let kind = computed(kind);
    svg(move || {
        ImageSrc::svg_markup(if kind.get() == NoticeKind::Error {
            ERROR
        } else {
            WARNING
        })
    })
    .w(px(14.0))
    .min_w(px(14.0))
    .h(px(14.0))
    .text_color(move || {
        if kind.get() == NoticeKind::Error {
            theme().destructive
        } else {
            theme().muted_foreground
        }
    })
}

/// Reuse the same entry for independent warnings and errors, with errors taking priority.
pub fn notice_badge(notices: impl Fn() -> Vec<Notice> + 'static) -> impl IntoElement {
    let notices = computed(notices);
    when(
        move || !notices.get().is_empty(),
        move || {
            let open = signal(false);
            let previous_errors = signal(Vec::<String>::new());
            effect(move || {
                let errors: Vec<_> = notices
                    .get()
                    .into_iter()
                    .filter(|notice| notice.kind == NoticeKind::Error)
                    .map(|notice| notice.message)
                    .collect();
                if errors != previous_errors.get_untracked() {
                    let show = !errors.is_empty();
                    previous_errors.set(errors);
                    open.set(show);
                }
            });
            let kind = move || {
                if notices
                    .get()
                    .iter()
                    .any(|notice| notice.kind == NoticeKind::Error)
                {
                    NoticeKind::Error
                } else {
                    NoticeKind::Warning
                }
            };
            popover()
                .open(move || Some(open.get()))
                .on_open_change(move |event: &nsh_base::PopoverEvent| {
                    if let nsh_base::PopoverEvent::OpenChange(value, _) = event {
                        open.set(*value);
                    }
                })
                .placement(Placement::Bottom)
                .align(Align::End)
                .offset(px(4.0))
                .margin(px(12.0))
                .popup_style(|s| {
                    s.w(px(240.0))
                        .p(px(10.0))
                        .bg(theme().background)
                        .border_1()
                        .border_color(theme().muted)
                        .rounded(px(6.0))
                })
                .trigger(
                    button()
                        .accessibility_label(move || {
                            Some(SharedString::from(if kind() == NoticeKind::Error {
                                tr!("View playback errors")
                            } else {
                                tr!("View playback warnings")
                            }))
                        })
                        .w(px(if cfg!(target_os = "ios") { 44.0 } else { 26.0 }))
                        .min_w(px(if cfg!(target_os = "ios") { 44.0 } else { 26.0 }))
                        .h(px(if cfg!(target_os = "ios") { 44.0 } else { 26.0 }))
                        .items_center()
                        .justify_center()
                        .rounded(px(4.0))
                        .hover(|s| s.bg(theme().muted))
                        .active(|s| s.opacity(0.65))
                        .child(notice_icon(kind)),
                )
                .child(view().gap(px(8.0)).child(each(
                    move || notices.get(),
                    |notice| notice.message.clone(),
                    move |notice, _| {
                        view()
                            .flex_row()
                            .items_start()
                            .gap(px(8.0))
                            .child(
                                view()
                                    .pt(px(1.0))
                                    .child(notice_icon(move || notice.get().kind)),
                            )
                            .child(
                                text(move || notice.get().message)
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .text_size(px(11.0))
                                    .number_of_lines(8)
                                    .text_color(move || theme().foreground),
                            )
                    },
                )))
        },
    )
}

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
