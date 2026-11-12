//! Provider pages share the existing player; this state owns browsing only.
use crate::protocol::tr;
use crate::{
    controls::*,
    model::Model,
    protocol::{MusicSourceAction as Action, SourcePage},
    remote,
};
use nsh_base::{button, theme};
use nsh_core::prelude::*;
use tokio::sync::oneshot;

#[derive(Clone, Copy)]
struct Browser {
    owner: Owner,
    action: Signal<Action>,
    history: Signal<Vec<Action>>,
    page: Signal<SourcePage>,
    busy: Signal<bool>,
    error: Signal<String>,
}
fn source(action: &Action) -> String {
    match action {
        Action::Artist { source, .. }
        | Action::Album { source, .. }
        | Action::Show { source, .. }
        | Action::Shows { source } => source.clone(),
        _ => String::new(),
    }
}
fn append_page(old: &mut SourcePage, mut page: SourcePage) -> Result<(), &'static str> {
    let mut seen: std::collections::HashSet<_> =
        old.tracks.iter().map(crate::protocol::track_key).collect();
    page.tracks
        .retain(|track| seen.insert(crate::protocol::track_key(track)));
    if old.tracks.len() + page.tracks.len() > crate::protocol::MAX_QUEUE_TRACKS
        || (page.more && (page.tracks.is_empty() || page.next_offset <= old.next_offset))
    {
        return Err(tr!(
            "Source pagination did not advance or exceeded 10,000 tracks"
        ));
    }
    old.tracks.extend(page.tracks);
    old.more = page.more;
    old.next_offset = old.next_offset.max(page.next_offset);
    Ok(())
}
impl Browser {
    fn load(self, mut action: Action, append: bool) {
        if self.busy.get() {
            return;
        }
        self.busy.set(true);
        self.error.set(String::new());
        if !append && let Action::Show { offset, .. } = &mut action {
            *offset = 0;
        }
        self.action.set(action.clone());
        if !append {
            self.page.set(SourcePage::default());
        }
        let (send, receive) = oneshot::channel();
        std::thread::spawn(move || {
            let _ = send.send(remote::post_json::<_, SourcePage>(
                "/ui/music-sources",
                &action,
            ));
        });
        self.owner.run(|| {
            spawn(async move {
                let result = receive.await;
                self.busy.set(false);
                match result {
                    Ok(Ok(page)) => {
                        if append {
                            let mut error = None;
                            self.page.update(|old| {
                                error = append_page(old, page).err();
                            });
                            if let Some(error) = error {
                                self.error.set(error.into());
                            }
                        } else {
                            self.page.set(page);
                        }
                    }
                    Ok(Err(error)) => self.error.set(error.to_string()),
                    Err(_) => self
                        .error
                        .set(tr!("Loading ended; please try again").into()),
                }
            })
        });
    }
    fn next(self, kind: &str, id: String) {
        self.history.update(|h| h.push(self.action.get()));
        let source = self.action.with(source);
        self.load(
            if kind == "album" {
                Action::Album { source, id }
            } else {
                Action::Show {
                    source,
                    id,
                    offset: 0,
                }
            },
            false,
        );
    }
}

pub fn artist_links(model: Model) -> impl IntoElement {
    view().gap(px(3.0)).child(each(
        move || {
            model.status.with(|s| {
                s.track
                    .as_ref()
                    .map_or_else(Vec::new, |t| t.artists.clone())
            })
        },
        |a| a.id.clone(),
        move |artist, _| {
            button()
                .accessibility_label(move || {
                    Some(SharedString::from(tr!(
                        "View artist {}",
                        artist.with(|a| a.name.clone())
                    )))
                })
                .on_press(move || {
                    if let Some(plugin) = model.status.with(|s| {
                        s.track
                            .as_ref()
                            .and_then(|t| t.source.as_ref().map(|s| s.plugin.clone()))
                    }) {
                        model.open_source(Action::Artist {
                            source: format!("mygo:{plugin}"),
                            id: artist.with(|a| a.id.clone()),
                        });
                    }
                })
                .py(px(5.0))
                .rounded(px(4.0))
                .hover(|s| s.bg(theme().muted))
                .child(
                    text(move || artist.with(|a| tr!("Artist · {}", a.name)))
                        .text_size(px(12.0))
                        .number_of_lines(1)
                        .text_color(move || theme().foreground),
                )
        },
    ))
}

pub fn episode_details(model: Model) -> impl IntoElement {
    view()
        .gap(px(6.0))
        .child(caption(move || {
            model.status.with(|s| {
                s.track
                    .as_ref()
                    .and_then(|t| t.episode.as_ref())
                    .map_or_else(String::new, |e| {
                        format!("{}\n{}", e.published, e.description)
                    })
            })
        }))
        .child(when(
            move || {
                model
                    .status
                    .with(|s| s.track.as_ref().is_some_and(|t| t.episode.is_some()))
            },
            move || {
                action(
                    tr!("View podcast"),
                    || false,
                    move || {
                        if let Some((plugin, id)) = model.status.with(|s| {
                            s.track.as_ref().and_then(|t| {
                                Some((
                                    t.source.as_ref()?.plugin.clone(),
                                    t.episode.as_ref()?.show_id.clone(),
                                ))
                            })
                        }) {
                            model.open_source(Action::Show {
                                source: format!("mygo:{plugin}"),
                                id,
                                offset: 0,
                            });
                        }
                    },
                )
            },
        ))
}

pub fn browse(model: Model) -> impl IntoElement {
    let initial = model.discovery.get().unwrap_or(Action::List);
    let this = Browser {
        owner: Owner::current(),
        action: signal(initial.clone()),
        history: signal(Vec::new()),
        page: signal(SourcePage::default()),
        busy: signal(false),
        error: signal(String::new()),
    };
    this.load(initial, false);
    view()
        .flex_1()
        .min_h(px(0.0))
        .gap(px(8.0))
        .pt(px(10.0))
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .child(action(
                    tr!("Back"),
                    move || this.busy.get(),
                    move || {
                        let mut history = this.history.get();
                        let previous = history.pop();
                        this.history.set(history);
                        if let Some(previous) = previous {
                            this.load(previous, false);
                        } else {
                            model.open(model.discovery_back.get());
                        }
                    },
                ))
                .child(action(
                    tr!("Refresh"),
                    move || this.busy.get(),
                    move || this.load(this.action.get(), false),
                )),
        )
        .child(heading(move || this.page.with(|p| p.title.clone())))
        .child(caption(move || this.page.with(|p| p.description.clone())))
        .child(when(
            move || this.busy.get(),
            || caption(tr!("Loading source content…")),
        ))
        .child(when(
            move || this.error.with(|e| !e.is_empty()),
            move || caption(move || this.error.get()),
        ))
        .child(when(
            move || {
                !this.busy.get()
                    && this.error.with(String::is_empty)
                    && this
                        .page
                        .with(|p| p.tracks.is_empty() && p.collections.is_empty())
            },
            || caption(tr!("No content from this source. Try refreshing.")),
        ))
        .child(
            virtual_list(
                move || this.page.with(|p| p.collections.len() + p.tracks.len()),
                move |at| {
                    let collection =
                        move || this.page.with(|p| p.collections.get(at.index()).cloned());
                    let track = move || {
                        this.page.with(|p| {
                            at.index()
                                .checked_sub(p.collections.len())
                                .and_then(|i| p.tracks.get(i).cloned())
                        })
                    };
                    button()
                        .disabled(move || this.busy.get() || model.starting.get())
                        .accessibility_label(move || {
                            Some(SharedString::from(
                                collection()
                                    .map(|c| tr!("Open {}", c.name))
                                    .or_else(|| track().map(|t| tr!("Play {}", t.title)))
                                    .unwrap_or_default(),
                            ))
                        })
                        .on_press(move || {
                            if let Some(c) = collection() {
                                this.next(&c.kind, c.id);
                            } else if let Some(index) = this
                                .page
                                .with(|p| at.index().checked_sub(p.collections.len()))
                            {
                                model.play_source(this.page.get(), this.action.with(source), index);
                            }
                        })
                        .w_full()
                        .h(px(64.0))
                        .px(px(6.0))
                        .gap(px(8.0))
                        .flex_row()
                        .items_center()
                        .rounded(px(6.0))
                        .hover(|s| s.bg(theme().muted))
                        .child(artwork(
                            move || {
                                collection()
                                    .and_then(|c| c.cover_url)
                                    .or_else(|| track().and_then(|t| t.cover_url))
                            },
                            move || tr!("Source artwork").into(),
                            36.0,
                        ))
                        .child(
                            view()
                                .flex_1()
                                .min_w(px(0.0))
                                .gap(px(3.0))
                                .child(
                                    text(move || {
                                        collection()
                                            .map(|c| c.name)
                                            .or_else(|| track().map(|t| t.title))
                                            .unwrap_or_default()
                                    })
                                    .text_size(px(13.0))
                                    .number_of_lines(1)
                                    .text_color(move || theme().foreground),
                                )
                                .child(caption(move || {
                                    collection()
                                        .map(|c| c.description)
                                        .or_else(|| {
                                            track().map(|t| {
                                                t.episode.map_or(t.artist, |e| e.published)
                                            })
                                        })
                                        .unwrap_or_default()
                                })),
                        )
                },
            )
            .flex_1()
            .min_h(px(0.0))
            .estimated_item_size(px(64.0))
            .label(tr!("Source releases and podcasts")),
        )
        .child(when(
            move || {
                this.page.with(|p| p.more) && this.action.with(|a| matches!(a, Action::Show { .. }))
            },
            move || {
                action(
                    tr!("Load more episodes"),
                    move || this.busy.get(),
                    move || {
                        if let Action::Show { source, id, .. } = this.action.get() {
                            this.load(
                                Action::Show {
                                    source,
                                    id,
                                    offset: this.page.with(|p| p.next_offset),
                                },
                                true,
                            );
                        }
                    },
                )
            },
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_terminal_page_finishes_podcast_pagination() {
        let track = serde_json::from_value(serde_json::json!({
            "id":1,"title":"Episode","artist":"Podcast","duration":60.0
        }))
        .unwrap();
        let mut old = SourcePage {
            tracks: vec![track],
            more: true,
            next_offset: 1,
            ..Default::default()
        };
        let mut terminal = SourcePage {
            next_offset: 1,
            ..Default::default()
        };
        terminal.more = true;
        assert!(append_page(&mut old, terminal.clone()).is_err());
        assert!(old.more);
        terminal.more = false;
        append_page(&mut old, terminal).unwrap();
        assert!(!old.more);
        assert_eq!(old.tracks.len(), 1);
        assert_eq!(old.next_offset, 1);
    }
}
