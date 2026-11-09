//! Player-owned local playlists, independent of connected source accounts.
use crate::controls::{action, caption, heading, primary};
use crate::{
    model::Model,
    protocol::{LocalLibraryAction as Edit, LocalPlaylist, LocalPlaylistSummary, Status},
    remote,
};
use nsh_base::{button, input, theme};
use nsh_core::prelude::*;
use tokio::sync::oneshot;

#[derive(Clone, Copy)]
struct Browser {
    owner: Owner,
    busy: Signal<bool>,
    error: Signal<String>,
    name: Signal<String>,
    lists: Signal<Vec<LocalPlaylistSummary>>,
    selected: Signal<Option<LocalPlaylist>>,
    deleting: Signal<bool>,
    creating: Signal<bool>,
    editing: Signal<bool>,
    managing: Signal<bool>,
    adding: Signal<bool>,
    importing: Signal<bool>,
}
impl Browser {
    fn work<T: Send + 'static>(
        self,
        run: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
        done: impl FnOnce(Self, T) + 'static,
    ) {
        if self.busy.get() {
            return;
        }
        self.busy.set(true);
        self.error.set(String::new());
        let (send, receive) = oneshot::channel();
        std::thread::spawn(move || {
            let _ = send.send(run());
        });
        self.owner.run(|| {
            spawn(async move {
                let result = receive.await;
                // Publish the result before the revision watcher can refresh the selection.
                batch(|| {
                    self.busy.set(false);
                    match result {
                        Ok(Ok(value)) => done(self, value),
                        Ok(Err(e)) => self.error.set(e.to_string()),
                        Err(_) => self.error.set("请求已结束，请重试".into()),
                    }
                });
            })
        });
    }
    fn lists(self) {
        self.creating.set(false);
        self.editing.set(false);
        self.managing.set(false);
        self.adding.set(false);
        self.selected.set(None);
        self.name.set(String::new());
        self.deleting.set(false);
        self.work(
            || remote::read_json::<Vec<LocalPlaylistSummary>>("/ui/local-playlists"),
            |this, lists| this.lists.set(lists),
        );
    }
    fn select(self, id: String) {
        self.creating.set(false);
        self.editing.set(false);
        self.managing.set(false);
        self.adding.set(false);
        self.deleting.set(false);
        self.work(
            move || remote::read_json::<LocalPlaylist>(&format!("/ui/local-playlists?id={id}")),
            |this, list| {
                this.name.set(list.name.clone());
                this.selected.set(Some(list));
            },
        );
    }
    fn edit(self, edit: Edit) {
        self.deleting.set(false);
        self.work(
            move || remote::post_json::<_, LocalPlaylist>("/ui/local-playlists", &edit),
            |this, list| {
                this.name.set(list.name.clone());
                this.creating.set(false);
                this.editing.set(false);
                this.selected.set(Some(list));
            },
        );
    }
    fn set_pinned(self, pinned: bool) {
        if let Some(id) = self.selected.with(|p| p.as_ref().map(|p| p.id.clone())) {
            self.edit(Edit::SetPinned { id, pinned });
        }
    }
    fn delete(self) {
        let Some(id) = self.selected.with(|p| p.as_ref().map(|p| p.id.clone())) else {
            return;
        };
        let edit = Edit::Delete { id: id.clone() };
        self.work(
            move || remote::post_json::<_, serde_json::Value>("/ui/local-playlists", &edit),
            move |this, _| {
                this.lists
                    .update(|lists| lists.retain(|list| list.id != id));
                this.lists();
            },
        );
    }
    fn add(self, queue: bool) {
        let Some(id) = self.selected.with(|p| p.as_ref().map(|p| p.id.clone())) else {
            return;
        };
        self.work(
            move || {
                let state: Status = remote::read_json("/ui/queue")?;
                let tracks = if queue {
                    state.queue.as_ref().clone()
                } else {
                    vec![state.track.ok_or_else(|| anyhow::anyhow!("当前没有歌曲"))?]
                };
                anyhow::ensure!(!tracks.is_empty(), "当前播放列表为空");
                remote::post_json::<_, LocalPlaylist>(
                    "/ui/local-playlists",
                    &Edit::Add { id, tracks },
                )
            },
            |this, list| {
                this.adding.set(false);
                this.selected.set(Some(list));
            },
        );
    }
}
pub fn library_view(model: Model) -> impl IntoElement {
    let this = Browser {
        owner: Owner::current(),
        busy: signal(false),
        error: signal(String::new()),
        name: signal(String::new()),
        lists: signal(Vec::new()),
        selected: signal(None),
        deleting: signal(false),
        creating: signal(false),
        editing: signal(false),
        managing: signal(false),
        adding: signal(false),
        importing: signal(false),
    };
    this.lists();
    // Read playback state only inside the effect, never in the page factory.
    let seen = signal(0u64);
    effect(move || {
        let revision = model.status.with(|s| s.library_revision);
        if this.busy.get() || seen.get() == revision {
            return;
        }
        seen.set(revision);
        if let Some(id) = this.selected.with(|p| p.as_ref().map(|p| p.id.clone())) {
            this.work(
                move || remote::read_json::<LocalPlaylist>(&format!("/ui/local-playlists?id={id}")),
                |this, list| this.selected.set(Some(list)),
            );
        } else {
            this.work(
                || remote::read_json::<Vec<LocalPlaylistSummary>>("/ui/local-playlists"),
                |this, lists| this.lists.set(lists),
            );
        }
    });
    view()
        .flex_1()
        .min_h(px(0.0))
        .pt(px(10.0))
        .gap(px(10.0))
        .child(when(
            move || this.importing.get(),
            move || {
                crate::sources_ui::import_view(model, move || {
                    this.importing.set(false);
                    this.lists();
                })
            },
        ))
        .child(when(
            move || !this.importing.get() && this.selected.with(Option::is_none),
            move || {
                view()
                    .flex_1()
                    .min_h(px(0.0))
                    .gap(px(12.0))
                    .child(when(
                        move || !this.creating.get() && this.lists.with(|v| !v.is_empty()),
                        move || {
                            view()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .child(caption(move || {
                                    this.lists.with(|v| format!("{} 张歌单", v.len()))
                                }))
                                .child(
                                    view()
                                        .flex_row()
                                        .gap(px(2.0))
                                        .child(action(
                                            "新建歌单",
                                            move || this.busy.get(),
                                            move || {
                                                this.name.set(String::new());
                                                this.creating.set(true);
                                            },
                                        ))
                                        .child(action(
                                            "导入歌单",
                                            move || this.busy.get(),
                                            move || this.importing.set(true),
                                        )),
                                )
                        },
                    ))
                    .child(when(
                        move || this.creating.get(),
                        move || {
                            view()
                                .gap(px(10.0))
                                .child(heading("新建歌单"))
                                .child(
                                    input()
                                        .value(move || this.name.get())
                                        .placeholder("歌单名称")
                                        .label("新歌单名称")
                                        .on_change(move |v: &str| this.name.set(v.into()))
                                        .h(px(36.0))
                                        .px(px(10.0))
                                        .rounded(px(6.0))
                                        .bg(move || theme().muted)
                                        .text_color(move || theme().foreground),
                                )
                                .child(
                                    view()
                                        .flex_row()
                                        .gap(px(6.0))
                                        .child(primary(
                                            "创建歌单",
                                            move || {
                                                this.busy.get()
                                                    || this.name.with(|v| v.trim().is_empty())
                                            },
                                            move || {
                                                this.edit(Edit::Create {
                                                    name: this.name.get(),
                                                })
                                            },
                                        ))
                                        .child(action(
                                            "取消新建",
                                            move || this.busy.get(),
                                            move || {
                                                this.creating.set(false);
                                                this.name.set(String::new());
                                                this.error.set(String::new());
                                            },
                                        )),
                                )
                        },
                    ))
                    .child(when(
                        move || {
                            !this.busy.get()
                                && !this.creating.get()
                                && this.lists.with(Vec::is_empty)
                        },
                        move || {
                            view()
                                .flex_1()
                                .items_center()
                                .justify_center()
                                .gap(px(10.0))
                                .pb(px(64.0))
                                .child(heading("还没有歌单"))
                                .child(caption("把喜欢的歌曲收集在一起。"))
                                .child(
                                    view()
                                        .mt(px(10.0))
                                        .gap(px(6.0))
                                        .items_center()
                                        .child(primary(
                                            "新建歌单",
                                            move || this.busy.get(),
                                            move || {
                                                this.name.set(String::new());
                                                this.creating.set(true);
                                            },
                                        ))
                                        .child(action(
                                            "导入歌单",
                                            move || this.busy.get(),
                                            move || this.importing.set(true),
                                        )),
                                )
                        },
                    ))
                    .child(when(
                        move || !this.creating.get() && this.lists.with(|v| !v.is_empty()),
                        move || {
                            each(
                                move || vec![this.lists.with(Vec::len)],
                                |count| *count,
                                move |_, _| {
                                    virtual_list(
                                        move || this.lists.with(Vec::len),
                                        move |at| {
                                            let item = move || {
                                                this.lists.with(|p| p.get(at.index()).cloned())
                                            };
                                            button()
                                                .accessibility_label(move || {
                                                    Some(SharedString::from(format!(
                                                        "打开本地歌单 {}",
                                                        item().map_or_else(String::new, |p| p.name)
                                                    )))
                                                })
                                                .on_press(move || {
                                                    if let Some(p) = item() {
                                                        this.select(p.id);
                                                    }
                                                })
                                                .disabled(move || this.busy.get())
                                                .w_full()
                                                .h(px(56.0))
                                                .px(px(6.0))
                                                .flex_row()
                                                .items_center()
                                                .gap(px(10.0))
                                                .rounded(px(6.0))
                                                .hover(|s| s.bg(theme().muted))
                                                .active(|s| s.opacity(0.65))
                                                .child(crate::controls::artwork(
                                                    move || item().and_then(|p| p.cover_url),
                                                    move || {
                                                        format!(
                                                            "{}封面",
                                                            item().map_or_else(String::new, |p| p
                                                                .name)
                                                        )
                                                    },
                                                    40.0,
                                                ))
                                                .child(
                                                    view()
                                                        .flex_1()
                                                        .min_w(px(0.0))
                                                        .gap(px(3.0))
                                                        .child(
                                                            text(move || {
                                                                item()
                                                                    .map_or_else(String::new, |p| {
                                                                        p.name
                                                                    })
                                                            })
                                                            .number_of_lines(1)
                                                            .text_size(px(13.0))
                                                            .text_color(move || theme().foreground),
                                                        )
                                                        .child(caption(move || {
                                                            item().map_or_else(String::new, |p| {
                                                                let prefix = if p.pinned {
                                                                    "已置顶 · "
                                                                } else {
                                                                    ""
                                                                };
                                                                format!(
                                                                    "{prefix}{} 首",
                                                                    p.track_count
                                                                )
                                                            })
                                                        })),
                                                )
                                        },
                                    )
                                    .flex_1()
                                    .min_h(px(0.0))
                                    .estimated_item_size(px(56.0))
                                    .label("本地歌单列表")
                                },
                            )
                        },
                    ))
            },
        ))
        .child(when(
            move || !this.importing.get() && this.selected.with(Option::is_some),
            move || {
                view()
                    .flex_1()
                    .min_h(px(0.0))
                    .gap(px(10.0))
                    .child(
                        view()
                            .flex_row()
                            .justify_between()
                            .child(action(
                                "所有歌单",
                                move || this.busy.get(),
                                move || this.lists(),
                            ))
                            .child(when(
                                move || !this.managing.get(),
                                move || {
                                    action(
                                        "管理歌单",
                                        move || this.busy.get(),
                                        move || {
                                            this.managing.set(true);
                                            this.editing.set(false);
                                            this.deleting.set(false);
                                            this.adding.set(false);
                                        },
                                    )
                                },
                            ))
                            .child(when(
                                move || this.managing.get(),
                                move || {
                                    action(
                                        "完成管理",
                                        move || this.busy.get(),
                                        move || {
                                            this.managing.set(false);
                                            this.editing.set(false);
                                            this.deleting.set(false);
                                        },
                                    )
                                },
                            )),
                    )
                    .child(when(
                        move || !this.editing.get(),
                        move || {
                            heading(move || {
                                this.selected.with(|p| {
                                    p.as_ref().map_or_else(String::new, |p| p.name.clone())
                                })
                            })
                        },
                    ))
                    .child(caption(move || {
                        this.selected.with(|p| {
                            p.as_ref().map_or_else(String::new, |p| {
                                format!("{} 首 · 本地歌单", p.tracks.len())
                            })
                        })
                    }))
                    .child(when(
                        move || this.editing.get(),
                        move || {
                            view()
                                .gap(px(6.0))
                                .child(
                                    input()
                                        .value(move || this.name.get())
                                        .on_change(move |v: &str| this.name.set(v.into()))
                                        .label("歌单名称")
                                        .h(px(34.0))
                                        .px(px(8.0))
                                        .bg(move || theme().muted)
                                        .text_color(move || theme().foreground)
                                        .rounded(px(5.0)),
                                )
                                .child(
                                    view()
                                        .flex_row()
                                        .gap(px(4.0))
                                        .child(primary(
                                            "保存名称",
                                            move || {
                                                this.busy.get()
                                                    || this.name.with(|v| v.trim().is_empty())
                                            },
                                            move || {
                                                if let Some(p) = this.selected.get() {
                                                    this.edit(Edit::Rename {
                                                        id: p.id,
                                                        name: this.name.get(),
                                                    });
                                                }
                                            },
                                        ))
                                        .child(action(
                                            "取消重命名",
                                            move || this.busy.get(),
                                            move || this.editing.set(false),
                                        )),
                                )
                        },
                    ))
                    .child(when(
                        move || !this.managing.get(),
                        move || {
                            view()
                                .gap(px(6.0))
                                .child(
                                    view()
                                        .flex_row()
                                        .gap(px(6.0))
                                        .child(primary(
                                            "播放歌单",
                                            move || {
                                                model.starting.get()
                                                    || this.busy.get()
                                                    || this.selected.with(|p| {
                                                        p.as_ref()
                                                            .is_none_or(|p| p.tracks.is_empty())
                                                    })
                                            },
                                            move || {
                                                if let Some(p) = this.selected.get() {
                                                    model.play_local(p.id, 0);
                                                }
                                            },
                                        ))
                                        .child(action(
                                            "添加歌曲",
                                            move || this.busy.get(),
                                            move || this.adding.update(|v| *v = !*v),
                                        )),
                                )
                                .child(when(
                                    move || this.adding.get(),
                                    move || {
                                        view()
                                            .gap(px(2.0))
                                            .child(action(
                                                "加入当前歌曲",
                                                move || {
                                                    this.busy.get()
                                                        || model.status.with(|s| s.track.is_none())
                                                },
                                                move || this.add(false),
                                            ))
                                            .child(action(
                                                "加入播放队列",
                                                move || {
                                                    this.busy.get()
                                                        || model
                                                            .status
                                                            .with(|s| s.queue_length == 0)
                                                },
                                                move || this.add(true),
                                            ))
                                    },
                                ))
                        },
                    ))
                    .child(when(
                        move || this.managing.get() && !this.editing.get(),
                        move || {
                            view()
                                .gap(px(4.0))
                                .child(when(
                                    move || {
                                        this.selected
                                            .with(|p| p.as_ref().is_some_and(|p| !p.favorite))
                                    },
                                    move || {
                                        view()
                                            .flex_row()
                                            .gap(px(4.0))
                                            .child(
                                                when(
                                                    move || {
                                                        this.selected.with(|p| {
                                                            p.as_ref().is_some_and(|p| p.pinned)
                                                        })
                                                    },
                                                    move || {
                                                        action(
                                                            "取消置顶",
                                                            move || {
                                                                this.busy.get()
                                                                    || this.deleting.get()
                                                            },
                                                            move || this.set_pinned(false),
                                                        )
                                                    },
                                                )
                                                .otherwise(move || {
                                                    action(
                                                        "置顶",
                                                        move || {
                                                            this.busy.get() || this.deleting.get()
                                                        },
                                                        move || this.set_pinned(true),
                                                    )
                                                }),
                                            )
                                            .child(action(
                                                "重命名",
                                                move || this.busy.get(),
                                                move || {
                                                    if let Some(p) = this.selected.get() {
                                                        this.name.set(p.name);
                                                        this.editing.set(true);
                                                    }
                                                },
                                            ))
                                            .child(action(
                                                "删除歌单",
                                                move || this.busy.get(),
                                                move || this.deleting.set(true),
                                            ))
                                    },
                                ))
                                .child(when(
                                    move || this.deleting.get(),
                                    move || {
                                        view()
                                            .gap(px(4.0))
                                            .child(caption(
                                                "删除这张本地歌单？当前播放队列会保留。",
                                            ))
                                            .child(
                                                view()
                                                    .flex_row()
                                                    .child(action(
                                                        "取消删除",
                                                        move || this.busy.get(),
                                                        move || this.deleting.set(false),
                                                    ))
                                                    .child(action(
                                                        "确认删除歌单",
                                                        move || this.busy.get(),
                                                        move || this.delete(),
                                                    )),
                                            )
                                    },
                                ))
                        },
                    ))
                    .child(when(
                        move || {
                            this.selected
                                .with(|p| p.as_ref().is_some_and(|p| p.tracks.is_empty()))
                        },
                        || caption("歌单为空，点「添加歌曲」加入当前歌曲或队列。"),
                    ))
                    .child(when(
                        move || {
                            this.selected
                                .with(|p| p.as_ref().is_some_and(|p| !p.tracks.is_empty()))
                        },
                        move || {
                            each(
                                move || {
                                    vec![
                                        this.selected
                                            .with(|p| p.as_ref().map_or(0, |p| p.tracks.len())),
                                    ]
                                },
                                |count| *count,
                                move |_, _| {
                                    virtual_list(
                                        move || {
                                            this.selected
                                                .with(|p| p.as_ref().map_or(0, |p| p.tracks.len()))
                                        },
                                        move |at| {
                                            let track = move || {
                                                this.selected.with(|p| {
                                                    p.as_ref().and_then(|p| {
                                                        p.tracks.get(at.index()).cloned()
                                                    })
                                                })
                                            };
                                            view()
                                                .flex_row()
                                                .items_center()
                                                .h(px(52.0))
                                                .gap(px(4.0))
                                                .child(
                                                    button()
                                                        .accessibility_label(move || {
                                                            Some(SharedString::from(format!(
                                                                "播放本地歌单第{}首 {}",
                                                                at.index() + 1,
                                                                track().map_or_else(
                                                                    String::new,
                                                                    |t| t.title
                                                                )
                                                            )))
                                                        })
                                                        .disabled(move || {
                                                            model.starting.get() || this.busy.get()
                                                        })
                                                        .on_press(move || {
                                                            if let Some(p) = this.selected.get() {
                                                                model.play_local(p.id, at.index());
                                                            }
                                                        })
                                                        .flex_1()
                                                        .min_w(px(0.0))
                                                        .h_full()
                                                        .px(px(6.0))
                                                        .flex_row()
                                                        .items_center()
                                                        .gap(px(8.0))
                                                        .rounded(px(5.0))
                                                        .hover(|s| s.bg(theme().muted))
                                                        .child(crate::controls::artwork(
                                                            move || {
                                                                track().and_then(|t| t.cover_url)
                                                            },
                                                            move || {
                                                                format!(
                                                                    "{}封面",
                                                                    track().map_or_else(
                                                                        String::new,
                                                                        |t| t.title
                                                                    )
                                                                )
                                                            },
                                                            36.0,
                                                        ))
                                                        .child(
                                                            view()
                                                                .flex_1()
                                                                .min_w(px(0.0))
                                                                .gap(px(3.0))
                                                                .child(
                                                                    text(move || {
                                                                        track().map_or_else(
                                                                            String::new,
                                                                            |t| t.title,
                                                                        )
                                                                    })
                                                                    .number_of_lines(1)
                                                                    .text_size(px(13.0))
                                                                    .text_color(move || {
                                                                        theme().foreground
                                                                    }),
                                                                )
                                                                .child(caption(move || {
                                                                    track().map_or_else(
                                                                        String::new,
                                                                        |t| t.artist,
                                                                    )
                                                                })),
                                                        ),
                                                )
                                                .child(when(
                                                    move || this.managing.get(),
                                                    move || {
                                                        action(
                                                            "移除",
                                                            move || this.busy.get(),
                                                            move || {
                                                                if let Some(p) = this.selected.get()
                                                                {
                                                                    this.edit(Edit::Remove {
                                                                        id: p.id,
                                                                        index: at.index(),
                                                                        revision: p.revision,
                                                                    });
                                                                }
                                                            },
                                                        )
                                                    },
                                                ))
                                        },
                                    )
                                    .flex_1()
                                    .min_h(px(0.0))
                                    .estimated_item_size(px(52.0))
                                    .label("本地歌单曲目")
                                },
                            )
                        },
                    ))
            },
        ))
        .child(when(
            move || this.busy.get() && !this.importing.get(),
            || caption("正在读取或保存…"),
        ))
        .child(when(
            move || this.error.with(|v| !v.is_empty()),
            move || caption(move || this.error.get()),
        ))
}
