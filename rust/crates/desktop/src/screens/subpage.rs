//! Shared pieces for the pages reached from the Agent screen: a back link, the page frame, a
//! segmented filter, list rows and one-line tool status.

use rune::{Color, El, ScrollSpec, col, el, icon, row, text};

use crate::app::State;
use crate::icons;
use crate::model::Screen;
use crate::ui::{BODY, BODY_MED, PAGE_TOP, PAGE_X, SMALL, Ui};

/// A quiet link back to the Agent screen, 32 points tall, sitting on the page's left edge.
pub fn back(u: &Ui) -> El {
    row().w_full().h(32.0).center_y().child(
        row()
            .key("subpage-back")
            .h(28.0)
            .pl(6.0)
            .pr(8.0)
            .ml(-6.0)
            .gap(6.0)
            .center_y()
            .radius(crate::ui::r(6.0))
            .hover(u.hover)
            .focusable(true)
            .role("button")
            .label("Back to Agent")
            .cursor(rune::Cursor::Pointer)
            .on_press(|st: &mut State| st.go(Screen::Agent))
            .child(icon(&icons::ARROW_LEFT, 14.0).color(u.muted).a11y_hidden())
            .child(text("Agent").font(BODY).color(u.muted).a11y_hidden()),
    )
}

/// Back link, then the standard page head.
pub fn head(u: &Ui, title: &str, detail: &str, actions: Option<El>) -> El {
    col()
        .w_full()
        .gap(4.0)
        .child(back(u))
        .child(u.page_head("", title, detail, actions))
}

/// A scrolling page with one left edge.
pub fn page(key: &str, head: El, body: El) -> El {
    col()
        .key(format!("{key}-scroll"))
        .w_full()
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .key(format!("{key}-page"))
                .w_full()
                .px(PAGE_X)
                .pt(PAGE_TOP - 8.0)
                .pb(32.0)
                .gap(16.0)
                .child(head)
                .child(body),
        )
}

/// A fixed-height page whose body fills the window (for list and detail splits).
pub fn fixed_page(key: &str, head: El, body: El) -> El {
    col()
        .key(format!("{key}-page"))
        .w_full()
        .h_full()
        .min_h(0.0)
        .px(PAGE_X)
        .pt(PAGE_TOP - 8.0)
        .gap(16.0)
        .child(head)
        .child(body.grow(1.0).min_h(0.0))
}

/// A segmented control: a hairline group of 32 point options. The caller attaches handlers.
pub fn segmented(u: &Ui, items: Vec<El>) -> El {
    let mut r = row()
        .h(32.0)
        .padding(2.0)
        .gap(2.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .border(1.0, u.hair);
    for item in items {
        r = r.child(item);
    }
    r
}

/// One option inside `segmented`.
pub fn segment(u: &Ui, label: &str, on: bool) -> El {
    row()
        .h(26.0)
        .px(10.0)
        .center()
        .radius(crate::ui::r(6.0))
        .fill(if on { u.selected } else { Color::TRANSPARENT })
        .hover(u.hover)
        .focusable(true)
        .role("button")
        .label(label.to_string())
        .checked(on)
        .child(
            text(label.to_string())
                .font(BODY)
                .color(if on { u.ink } else { u.muted })
                .a11y_hidden(),
        )
}

/// A section title with an optional count, 32 points tall, matching Installed's group heads.
pub fn group_head(u: &Ui, label: &str, meta: &str) -> El {
    let mut r = row()
        .w_full()
        .h(32.0)
        .gap(6.0)
        .center_y()
        .child(text(label.to_string()).font(BODY_MED).color(u.muted));
    if !meta.is_empty() {
        r = r.child(text(meta.to_string()).font(BODY).color(u.faint));
    }
    r
}

/// A hairline key and value row for detail panes.
pub fn kv(u: &Ui, key: &str, value: El) -> El {
    row()
        .w_full()
        .min_h(36.0)
        .py(8.0)
        .gap(16.0)
        .items_start()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(text(key.to_string()).font(BODY).color(u.muted).w(140.0))
        .child(value.grow(1.0).min_w(0.0))
}

/// A plain text value for `kv`.
pub fn val(u: &Ui, s: impl Into<String>) -> El {
    text(s.into()).font(BODY).color(u.ink).wrap_anywhere()
}

/// A hairline list container: a top rule, rows bring their own bottom rule.
pub fn rows(u: &Ui) -> El {
    col().w_full().border_sides([1.0, 0.0, 0.0, 0.0], u.hair)
}

/// One tool on one line: name, its state ("Available" muted or "Not installed" amber), an
/// optional detail, and the action on the right.
pub fn tool_line(u: &Ui, name: &str, available: bool, detail: &str, action: Option<El>) -> El {
    let mut r = row()
        .w_full()
        .h(48.0)
        .gap(16.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .child(
            text(name.to_string())
                .font(BODY_MED)
                .color(u.ink)
                .w(200.0)
                .truncate(),
        )
        .child(
            text(if available {
                "Available"
            } else {
                "Not installed"
            })
            .font(BODY)
            .color(if available { u.muted } else { u.amber })
            .w(104.0),
        )
        .child(
            text(detail.to_string())
                .font(SMALL)
                .color(u.faint)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        );
    if let Some(a) = action {
        r = r.child(a);
    } else {
        r = r.child(el().w(0.0));
    }
    r
}
