//! Autoexec: the console commands the game runs at every start, edited as blocks.
//!
//! The file is `game/citadel/cfg/autoexec.cfg` under the install
//! (`deadlock_core::autoexec::autoexec_path`). It loads through
//! `deadlock_core::autoexec::parse_cfg` into one block per line and saves through
//! `serialise_cfg`, so comments, blank lines and lines Sigil does not understand come back
//! byte for byte. A save is a button press and a confirm; it writes through
//! `save_autoexec_if_unchanged`, which backs up the old file and refuses when another writer
//! changed it since it was opened.
//!
//! The premade catalog above the editor is Grimoire's (MIT), ported in
//! `deadlock-core/data/autoexec-presets.json`.

use std::path::PathBuf;

use deadlock_core::autoexec::{
    self as core, AutoexecSave, CfgDocument, CfgLine, CfgLineKind, LineEnding, Placement,
    RevealTarget,
};
use rune::{
    ClipboardContent, Drag, El, Icon, Key, KeyEvent, Phase, ScrollSpec, TextEdit, Tx, col, el,
    icon, row, spacer, text,
};

use deadlock_core::autoexec::launch_options::{self, LaunchSync, Saved, SteamState};

use crate::app::State;
use crate::icons;
use crate::jobs::Job;
use crate::model::Screen;
use crate::screens::settings::i18n::fill;
use crate::ui::{ActionKind, BODY, BODY_MED, H1, H2, MONO, PAGE_TOP, PAGE_X, SMALL, Ui};

#[path = "autoexec_de.rs"]
mod de;

/// The UI text in the player's language: this page's German table first, then Sigil's.
fn tr(text: &str) -> &str {
    use crate::screens::settings::i18n::{self, Lang};
    if i18n::current() == Lang::German
        && let Some(de) = de::get(text)
    {
        return de;
    }
    i18n::tr(text)
}

/// A command row, a comment heading and a blank line, in points. The drag math walks these.
const COMMAND_H: f32 = 40.0;
const COMMENT_H: f32 = 44.0;
const BLANK_H: f32 = 12.0;
/// The right end of a block: the preset name at rest, the row's actions on hover.
const TRAIL_W: f32 = 168.0;

const DESCRIPTION: &str = "Console commands the game runs every time it starts.";

/// Lucide grip-vertical and arrow-down (ISC), drawn like `icons.rs`.
static GRIP_VERTICAL: Icon = Icon {
    name: "sigil-autoexec-grip-vertical",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><circle cx="9" cy="12" r="1"/><circle cx="9" cy="5" r="1"/><circle cx="9" cy="19" r="1"/><circle cx="15" cy="12" r="1"/><circle cx="15" cy="5" r="1"/><circle cx="15" cy="19" r="1"/></g>"#,
};
static ARROW_DOWN: Icon = Icon {
    name: "sigil-autoexec-arrow-down",
    view: 24.0,
    body: r#"<g fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M12 5v14"/><path d="m19 12-7 7-7-7"/></g>"#,
};

pub struct Read {
    pub path: String,
    pub exists: bool,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BlockKind {
    Blank,
    /// `prefix` is the `//` marker with the spaces around it; the field edits the words.
    Comment { prefix: String },
    Command,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub id: u64,
    pub kind: BlockKind,
    pub edit: TextEdit,
    pub ending: LineEnding,
}

impl Block {
    /// The line exactly as it goes back into the file.
    pub fn raw(&self) -> String {
        match &self.kind {
            BlockKind::Comment { prefix } => format!("{prefix}{}", self.edit.text),
            _ => self.edit.text.clone(),
        }
    }

    /// The crosshair page's managed block markers: `Some(true)` opens it, `Some(false)` closes
    /// it. They show as one quiet heading and are not edited here.
    fn marker(&self) -> Option<bool> {
        if !matches!(self.kind, BlockKind::Comment { .. }) {
            return None;
        }
        let raw = self.raw();
        let raw = raw.trim();
        if raw == deadlock_core::crosshair::AUTOEXEC_BEGIN {
            Some(true)
        } else if raw == deadlock_core::crosshair::AUTOEXEC_END {
            Some(false)
        } else {
            None
        }
    }

    fn height(&self) -> f32 {
        match self.marker() {
            Some(true) => return COMMENT_H,
            Some(false) => return 0.0,
            None => {}
        }
        match self.kind {
            BlockKind::Blank => BLANK_H,
            BlockKind::Comment { .. } => COMMENT_H,
            BlockKind::Command => COMMAND_H,
        }
    }
}

pub struct View {
    pub loaded: bool,
    pub error: Option<String>,
    pub path: String,
    pub exists: bool,
    pub confirm_save: bool,
    pub confirm_clear: bool,
    pub busy: bool,
    pub last: Option<String>,
    pub blocks: Vec<Block>,
    pub bom: bool,
    pub next_id: u64,
    /// The premade tab on show.
    pub category: usize,
    /// A drag in progress: the block's index before the drag and where it would land.
    pub drag: Option<(usize, usize)>,
    /// The file on disk changed since it was opened; this is what it holds now.
    pub conflict: Option<String>,
    /// The text last copied, so the button can say Copied until the list changes.
    pub copied: Option<String>,
    /// Edits routed to a block that no longer exists land here and go nowhere.
    pub scratch: TextEdit,
    /// Steam launch options, written into Steam when Sigil launches the game.
    pub launch: Launch,
}

/// The launch options section: the saved value, the field, and what Steam holds.
pub struct Launch {
    pub loaded: bool,
    pub field: TextEdit,
    pub saved: Saved,
    /// What Steam's localconfig.vdf holds, once read.
    pub steam: Option<SteamState>,
    pub read: Job<(Saved, SteamState)>,
    pub write: Job<String>,
    /// One line after a save or a write, and whether it is a problem.
    pub line: Option<(String, bool)>,
}

impl Default for Launch {
    fn default() -> Self {
        Launch {
            loaded: false,
            field: TextEdit::new(""),
            saved: Saved::default(),
            steam: None,
            read: Job::default(),
            write: Job::default(),
            line: None,
        }
    }
}

impl Default for View {
    fn default() -> Self {
        View {
            loaded: false,
            error: None,
            path: String::new(),
            exists: false,
            confirm_save: false,
            confirm_clear: false,
            busy: false,
            last: None,
            blocks: Vec::new(),
            bom: false,
            next_id: 1,
            category: 0,
            drag: None,
            conflict: None,
            copied: None,
            scratch: TextEdit::new(""),
            launch: Launch::default(),
        }
    }
}

impl View {
    /// Replace the blocks with the file's lines.
    pub fn set_text(&mut self, text: &str) {
        let document = core::parse_cfg(text);
        self.bom = document.bom;
        self.blocks = Vec::with_capacity(document.lines.len());
        for line in document.lines {
            let block = self.block_from_line(&line.text, line.ending);
            self.blocks.push(block);
        }
        self.drag = None;
        self.conflict = None;
        self.copied = None;
        self.confirm_clear = false;
    }

    /// The file as it would be written.
    pub fn text(&self) -> String {
        core::serialise_cfg(&self.document())
    }

    fn document(&self) -> CfgDocument {
        CfgDocument {
            bom: self.bom,
            lines: self
                .blocks
                .iter()
                .map(|block| CfgLine {
                    text: block.raw(),
                    ending: block.ending,
                })
                .collect(),
        }
    }

    /// The text as it stands, with the same trailing-newline rule the writer applies, so the
    /// dirty check matches what would land on disk.
    pub fn dirty(&self, original: &str) -> bool {
        core::normalize_trailing_newline(&self.text())
            != core::normalize_trailing_newline(original)
    }

    pub fn line_count(&self) -> usize {
        self.blocks.len()
    }

    pub fn command_count(&self) -> usize {
        self.blocks
            .iter()
            .filter(|block| block.kind == BlockKind::Command && !block.edit.text.trim().is_empty())
            .count()
    }

    fn block_from_line(&mut self, line: &str, ending: LineEnding) -> Block {
        let (kind, words) = match core::classify_line(line) {
            CfgLineKind::Blank => (BlockKind::Blank, line.to_string()),
            CfgLineKind::Comment => {
                let (prefix, body) = core::split_comment(line).unwrap_or(("//", ""));
                (
                    BlockKind::Comment {
                        prefix: prefix.to_string(),
                    },
                    body.to_string(),
                )
            }
            CfgLineKind::Command => (BlockKind::Command, line.to_string()),
        };
        let id = self.next_id;
        self.next_id += 1;
        Block {
            id,
            kind,
            edit: TextEdit::new(words),
            ending,
        }
    }

    fn new_command(&mut self, text: &str) -> Block {
        let ending = self.document().preferred_ending();
        let mut block = self.block_from_line(text, ending);
        // A typed line stays a command block while it is being written.
        if block.kind != BlockKind::Command && text.trim().is_empty() {
            block.kind = BlockKind::Command;
        }
        block
    }

    fn index_of(&self, id: u64) -> Option<usize> {
        self.blocks.iter().position(|block| block.id == id)
    }

    /// The field for block `id`. A stale id (the block was deleted between frames) gets the
    /// scratch field, so a late keystroke never lands in the wrong line.
    pub fn edit_mut(&mut self, id: u64) -> &mut TextEdit {
        match self.blocks.iter().position(|block| block.id == id) {
            Some(index) => &mut self.blocks[index].edit,
            None => &mut self.scratch,
        }
    }

    fn focus(&mut self, id: u64, caret_at_end: bool) {
        for block in &mut self.blocks {
            if block.id == id {
                block.edit.focused = true;
                if caret_at_end {
                    let end = block.edit.text.len();
                    block.edit.set_caret(end);
                }
            } else {
                block.edit.focused = false;
            }
        }
    }

    /// Add an empty command after `after` (or at the end) and focus it.
    pub fn insert_after(&mut self, after: Option<usize>) -> u64 {
        let block = self.new_command("");
        let id = block.id;
        let at = after
            .map(|index| (index + 1).min(self.blocks.len()))
            .unwrap_or(self.blocks.len());
        self.blocks.insert(at, block);
        self.focus(id, true);
        id
    }

    pub fn remove(&mut self, id: u64) {
        if let Some(index) = self.index_of(id) {
            self.blocks.remove(index);
        }
    }

    pub fn move_block(&mut self, from: usize, to: usize) {
        if from >= self.blocks.len() || from == to {
            return;
        }
        let block = self.blocks.remove(from);
        let to = to.min(self.blocks.len());
        self.blocks.insert(to, block);
    }

    /// A command line typed as `// words` becomes a heading.
    fn settle(&mut self, id: u64) {
        let Some(index) = self.index_of(id) else {
            return;
        };
        let block = &self.blocks[index];
        if block.kind == BlockKind::Command
            && core::classify_line(&block.edit.text) == CfgLineKind::Comment
        {
            let line = block.edit.text.clone();
            let ending = block.ending;
            let mut settled = self.block_from_line(&line, ending);
            settled.id = id;
            self.blocks[index] = settled;
        }
    }

    fn command_lines(&self) -> Vec<String> {
        self.blocks
            .iter()
            .map(|block| match block.kind {
                BlockKind::Command => block.edit.text.clone(),
                _ => String::new(),
            })
            .collect()
    }

    /// Where a premade command would go.
    pub fn placement(&self, command: &str) -> Placement {
        core::place_command(&self.command_lines(), command)
    }

    /// Add a premade command: nothing when it is there, over the line that sets the same
    /// variable, else at the end.
    pub fn add_preset(&mut self, command: &str) {
        match self.placement(command) {
            Placement::Present(_) => {}
            Placement::Replace(index) => {
                self.blocks[index].edit = TextEdit::new(command);
            }
            Placement::Append => {
                // Fill a trailing empty command block instead of leaving it dangling.
                if let Some(last) = self.blocks.last_mut()
                    && last.kind == BlockKind::Command
                    && last.edit.text.trim().is_empty()
                {
                    last.edit = TextEdit::new(command);
                    return;
                }
                let block = self.new_command(command);
                self.blocks.push(block);
            }
        }
    }

    /// Lines pasted into block `id`: the first fills it when it is empty, the rest follow it.
    fn paste_lines(&mut self, id: u64, pasted: &str) {
        let Some(mut index) = self.index_of(id) else {
            return;
        };
        let lines = core::parse_cfg(pasted).lines;
        let mut last_id = id;
        for (n, line) in lines.iter().enumerate() {
            if n == 0 && self.blocks[index].edit.text.trim().is_empty() {
                let ending = self.blocks[index].ending;
                let mut block = self.block_from_line(&line.text, ending);
                block.id = id;
                self.blocks[index] = block;
                continue;
            }
            let ending = self.document().preferred_ending();
            let block = self.block_from_line(&line.text, ending);
            last_id = block.id;
            index += 1;
            self.blocks.insert(index, block);
        }
        self.focus(last_id, true);
    }
}

/// Where a block dragged `dy` points from block `from` lands: the block under its centre.
pub fn drop_index(heights: &[f32], from: usize, dy: f32) -> usize {
    if heights.is_empty() {
        return 0;
    }
    let from = from.min(heights.len() - 1);
    let top: f32 = heights[..from].iter().sum();
    let centre = top + heights[from] / 2.0 + dy;
    let mut y = 0.0;
    for (index, height) in heights.iter().enumerate() {
        y += height;
        if centre < y {
            return index;
        }
    }
    heights.len() - 1
}

// --- view ----------------------------------------------------------------------------------

pub fn view(s: &State, u: &Ui) -> El {
    let v = &s.autoexec;

    let mut actions = row().gap(8.0).center_y();
    if s.game_detected && s.game_root.is_some() {
        actions = actions
            .child(
                u.icon_action(&icons::SQUARE_PEN, tr("Edit in editor"), ActionKind::Secondary)
                    .key("autoexec-edit-external")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(open_in_editor),
            )
            .child(
                u.icon_action(&icons::FOLDER_OPEN, tr("Open"), ActionKind::Secondary)
                    .key("autoexec-open")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(reveal_autoexec),
            );
    }

    let mut page = col()
        .key("autoexec-page")
        .w_full()
        .px(PAGE_X)
        .pt(PAGE_TOP)
        .pb(48.0)
        .gap(16.0)
        .child(u.page_head("", tr("Autoexec"), tr(DESCRIPTION), Some(actions)));

    if let Some(last) = &v.last {
        page = page.child(result_line(u, last));
    }

    if !s.game_detected {
        page = page.child(u.empty_state(
            "",
            tr("Deadlock was not found"),
            tr("Set the game folder in Settings to edit autoexec.cfg."),
            Some(
                u.action(tr("Open Settings"), ActionKind::Secondary)
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| st.go(Screen::Settings)),
            ),
        ));
    } else if let Some(error) = &v.error {
        page = page.child(u.empty_state("", tr("autoexec.cfg could not be read"), error, None));
    } else if v.loaded {
        page = page
            .child(premade(s, u))
            .child(el().h(8.0))
            .child(editor(s, u))
            .child(el().h(8.0))
            .child(launch_section(s, u));
    }

    col()
        .key("autoexec-scroll")
        .w_full()
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(page)
}

/// Mounted at the root so the save and clear confirms overlay the shell.
pub fn dialog(s: &State, u: &Ui) -> Option<El> {
    if s.autoexec.confirm_save {
        Some(save_layer(s, u))
    } else if s.autoexec.confirm_clear {
        Some(clear_layer(s, u))
    } else {
        None
    }
}

/// The outcome of the last save, as one quiet line.
fn result_line(u: &Ui, message: &str) -> El {
    let lower = message.to_lowercase();
    let bad = ["not ", "could not", "failed", "nothing", "refus", "error"]
        .iter()
        .any(|word| lower.contains(word));
    row()
        .key("autoexec-result")
        .w_full()
        .gap(8.0)
        .items_start()
        .child(
            el().pt(2.0).child(
                icon(
                    if bad {
                        &icons::TRIANGLE_ALERT
                    } else {
                        &icons::CHECK
                    },
                    14.0,
                )
                .color(if bad { u.amber } else { u.muted })
                .a11y_hidden(),
            ),
        )
        .child(
            text(message.to_string())
                .font(BODY)
                .color(u.muted)
                .grow(1.0)
                .min_w(0.0)
                .wrap(),
        )
}

// --- premade -------------------------------------------------------------------------------

fn premade(s: &State, u: &Ui) -> El {
    let catalog = core::autoexec_presets();
    let selected = s.autoexec.category.min(catalog.len().saturating_sub(1));

    let mut tabs = row().key("autoexec-tabs").gap(4.0).center_y();
    for (index, category) in catalog.iter().enumerate() {
        let on = index == selected;
        tabs = tabs.child(
            row()
                .key(format!("autoexec-tab-{}", category.id))
                .h(28.0)
                .px(10.0)
                .center()
                .radius(crate::ui::r(6.0))
                .fill(if on { u.selected } else { rune::Color::TRANSPARENT })
                .hover(u.hover)
                .focusable(true)
                .role("tab")
                .checked(on)
                .label(category.name.clone())
                .cursor(rune::Cursor::Pointer)
                .on_press(move |st: &mut State| st.autoexec.category = index)
                .child(
                    text(category.name.clone())
                        .font(BODY_MED)
                        .color(if on { u.ink } else { u.muted })
                        .a11y_hidden(),
                ),
        );
    }

    let mut grid = col().key("autoexec-presets").w_full();
    if let Some(category) = catalog.get(selected) {
        for pair in category.presets.chunks(2) {
            let mut line = row().w_full().gap(32.0);
            for preset in pair {
                line = line.child(preset_row(s, u, preset));
            }
            if pair.len() == 1 {
                line = line.child(el().grow(1.0));
            }
            grid = grid.child(line);
        }
    }

    col()
        .key("autoexec-premade")
        .w_full()
        .gap(12.0)
        .child(
            row()
                .w_full()
                .h(32.0)
                .gap(16.0)
                .center_y()
                .child(u.section(tr("Premade commands")))
                .child(spacer())
                .child(tabs),
        )
        .child(grid)
}

fn preset_row(s: &State, u: &Ui, preset: &'static core::AutoexecPreset) -> El {
    let placement = s.autoexec.placement(&preset.command);
    let present = matches!(placement, Placement::Present(_));
    let (detail, label) = match placement {
        Placement::Present(_) => (
            preset.description.clone(),
            format!("{} is in your list", preset.name),
        ),
        Placement::Replace(index) => {
            let current = s
                .autoexec
                .blocks
                .get(index)
                .map(|block| block.edit.text.trim().to_string())
                .unwrap_or_default();
            (
                format!("Replaces {current}"),
                format!("Add {}, replacing {current}", preset.name),
            )
        }
        Placement::Append => (preset.description.clone(), format!("Add {}", preset.name)),
    };

    let mut r = row()
        .key(format!("autoexec-preset-{}", preset.id))
        .grow(1.0)
        .min_w(0.0)
        .h(56.0)
        .gap(12.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
        .label(label)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(
                    text(preset.name.clone())
                        .font(BODY_MED)
                        .color(if present { u.muted } else { u.ink })
                        .truncate(),
                )
                .child(text(detail).font(SMALL).color(u.faint).truncate()),
        )
        .child(
            text(preset.command.clone())
                .font(MONO)
                .color(u.faint)
                .max_w(220.0)
                .truncate(),
        )
        .child(
            el().size(28.0, 28.0).center().child(
                icon(if present { &icons::CHECK } else { &icons::PLUS }, 16.0)
                    .color(if present { u.faint } else { u.muted })
                    .a11y_hidden(),
            ),
        );
    if !present {
        let command = preset.command.clone();
        r = r
            .hover(u.hover)
            .focusable(true)
            .role("button")
            .cursor(rune::Cursor::Pointer)
            .on_press(move |st: &mut State| st.autoexec.add_preset(&command));
    }
    r
}

// --- editor --------------------------------------------------------------------------------

fn editor(s: &State, u: &Ui) -> El {
    let v = &s.autoexec;
    let dirty = v.dirty(&s.autoexec_original);
    let empty = v.blocks.is_empty();
    let count = v.command_count();
    let text_now = v.text();
    let copied = v.copied.as_deref() == Some(text_now.as_str()) && !empty;

    let mut tools = row().gap(8.0).center_y();
    if dirty {
        tools = tools.child(text(tr("Unsaved changes")).font(SMALL).color(u.muted));
    }
    tools = tools
        .child(
            u.icon_action(&icons::PLUS, tr("Add custom"), ActionKind::Ghost)
                .key("autoexec-add")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| {
                    st.autoexec.insert_after(None);
                }),
        )
        .child(
            u.action(tr("Clear all"), ActionKind::Ghost)
                .key("autoexec-clear")
                .disabled(empty)
                .opacity(if empty { 0.45 } else { 1.0 })
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| {
                    if !st.autoexec.blocks.is_empty() {
                        st.autoexec.confirm_clear = true;
                    }
                }),
        )
        .child(
            u.icon_action(
                if copied { &icons::CHECK } else { &icons::COPY },
                tr(if copied { "Copied" } else { "Copy all" }),
                ActionKind::Ghost,
            )
            .key("autoexec-copy")
            .disabled(empty)
            .opacity(if empty { 0.45 } else { 1.0 })
            .cursor(rune::Cursor::Pointer)
            .on_press_tx(|st: &mut State, tx: &mut Tx| {
                if st.autoexec.blocks.is_empty() {
                    return;
                }
                let all = st.autoexec.text();
                tx.copy(all.clone());
                st.autoexec.copied = Some(all);
            }),
        );
    if dirty {
        tools = tools.child(
            u.action(tr("Save"), ActionKind::Primary)
                .key("autoexec-save")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| {
                    if st.autoexec.loaded {
                        st.autoexec.conflict = None;
                        st.autoexec.confirm_save = true;
                    }
                }),
        );
    }

    let head = row()
        .key("autoexec-editor-head")
        .w_full()
        .h(32.0)
        .gap(8.0)
        .center_y()
        .child(u.section(tr("Your commands")))
        .child(text(count.to_string()).font(BODY).color(u.faint))
        .child(spacer())
        .child(tools);

    let mut list = col().key("autoexec-blocks").w_full();
    if empty {
        list = list.child(
            row()
                .key("autoexec-empty")
                .w_full()
                .h(COMMAND_H)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                .child(
                    text(tr(if v.exists {
                        "No commands yet. Pick one above, or write your own."
                    } else {
                        "No autoexec.cfg yet. Add a command and save to create it."
                    }))
                    .font(BODY)
                    .color(u.muted),
                ),
        );
    } else {
        let count = v.blocks.len();
        for (index, block) in v.blocks.iter().enumerate() {
            list = list.child(block_row(u, block, index, count, v.drag));
        }
    }
    list = list.child(add_row(u));

    col()
        .key("autoexec-editor")
        .w_full()
        .gap(8.0)
        .child(head)
        .child(list)
}

/// The quiet last row: a click adds a command at the end.
fn add_row(u: &Ui) -> El {
    row()
        .key("autoexec-add-row")
        .w_full()
        .h(COMMAND_H)
        .gap(8.0)
        .center_y()
        .hover_group()
        .focusable(true)
        .role("button")
        .label("Add a command")
        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| {
            st.autoexec.insert_after(None);
        })
        .child(
            icon(&icons::PLUS, 14.0)
                .color(u.faint)
                .group_color(u.muted)
                .a11y_hidden(),
        )
        .child(
            text("Add a command")
                .font(BODY)
                .color(u.faint)
                .group_color(u.muted)
                .a11y_hidden(),
        )
}

fn block_row(u: &Ui, block: &Block, index: usize, count: usize, drag: Option<(usize, usize)>) -> El {
    let id = block.id;
    let key = format!("autoexec-block-{id}");

    if block.kind == BlockKind::Blank {
        let mut gap = el().key(key).w_full().h(BLANK_H);
        if let Some((from, to)) = drag
            && to == index
            && from != to
        {
            gap = gap.child(drop_line(u, from < to));
        }
        return gap;
    }

    match block.marker() {
        Some(true) => {
            return row()
                .key(key)
                .w_full()
                .h(COMMENT_H)
                .pb(6.0)
                .gap(8.0)
                .items_end()
                .child(text("Crosshair").font(H2).color(u.muted))
                .child(
                    text("Set on the Crosshair page")
                        .font(SMALL)
                        .color(u.faint)
                        .pb(1.0),
                );
        }
        Some(false) => return el().key(key).w_full().h(0.0),
        None => {}
    }

    let comment = matches!(block.kind, BlockKind::Comment { .. });
    let field = rune::text_input(&block.edit, if comment { H2 } else { MONO })
        .key(format!("autoexec-input-{id}"))
        .single_line()
        .label(if comment { "Heading" } else { "Command" })
        .grow(1.0)
        .min_w(0.0)
        .color(if comment { u.muted } else { u.ink })
        .placeholder(
            if comment {
                "Heading"
            } else {
                "Type a command, like fps_max 0"
            },
            u.faint,
        )
        .caret(u.accent, u.accent.alpha(0.20))
        .on_edit(rune::edit_access(move |st: &mut State| {
            st.autoexec.edit_mut(id)
        }))
        .on_submit(move |st: &mut State, _tx: &mut Tx| {
            st.autoexec.settle(id);
            let at = st.autoexec.index_of(id);
            st.autoexec.insert_after(at);
        })
        .on_input_key(move |st: &mut State, ev: &KeyEvent, tx: &mut Tx| {
            block_key(st, id, ev, tx)
        })
        .on_input_paste(move |st: &mut State, content: &ClipboardContent, _tx: &mut Tx| {
            let pasted = content.text();
            if !pasted.contains('\n') {
                return false;
            }
            st.autoexec.paste_lines(id, pasted);
            true
        });

    let preset = if comment {
        None
    } else {
        core::preset_for_line(&block.edit.text)
    };

    let dragging = drag.map(|(from, _)| from) == Some(index);
    let mut r = row()
        .key(key)
        .w_full()
        .h(block.height())
        .gap(12.0)
        .hover_group()
        .fill(if dragging {
            u.selected
        } else {
            rune::Color::TRANSPARENT
        })
        .child(grip(u, index, comment, dragging).key(format!("autoexec-grip-{id}")))
        .child(field);
    r = if comment {
        r.items_end().pb(6.0)
    } else {
        r.center_y().border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
    };
    // The preset name sits flush right at rest; on hover the row's actions take its place.
    let mut trail = row().w(TRAIL_W).h(28.0).center_y().child(spacer());
    if let Some(preset) = preset {
        trail = trail.child(
            text(preset.name.clone())
                .font(SMALL)
                .color(u.faint)
                .truncate()
                .min_w(0.0)
                .group_opacity(1.0, 0.0)
                .a11y_hidden(),
        );
    }
    let actions_w = 4.0 * 28.0 + 3.0 * 2.0;
    trail = trail.child(
        block_actions(u, id, index, count).absolute(TRAIL_W - actions_w, 0.0),
    );
    r = r.child(trail);
    if let Some((from, to)) = drag
        && to == index
        && from != to
    {
        r = r.child(drop_line(u, from < to));
    }
    r
}

/// The insertion mark while a block is dragged: under the target when moving down, over it
/// when moving up.
fn drop_line(u: &Ui, below: bool) -> El {
    let line = el().w_full().h(2.0).fill(u.muted).a11y_hidden();
    let holder = el().absolute(0.0, 0.0).w_full().h_full().a11y_hidden();
    if below {
        holder.child(col().w_full().h_full().child(spacer()).child(line))
    } else {
        holder.child(line)
    }
}

/// The drag handle, hanging in the page margin to the left of the block.
fn grip(u: &Ui, index: usize, comment: bool, dragging: bool) -> El {
    let top = if comment {
        COMMENT_H - 6.0 - 20.0
    } else {
        (COMMAND_H - 20.0) / 2.0
    };
    el().absolute(-26.0, top)
        .size(20.0, 20.0)
        .center()
        .radius(4.0)
        .group_opacity(if dragging { 1.0 } else { 0.0 }, 1.0)
        .hover(u.hover)
        .label("Drag to reorder")
        .cursor(if dragging {
            rune::Cursor::Grabbing
        } else {
            rune::Cursor::Grab
        })
        .on_drag(move |st: &mut State, d: &mut Drag, _tx: &mut Tx| {
            drag_block(st, index, d);
        })
        .child(icon(&GRIP_VERTICAL, 14.0).color(u.faint).a11y_hidden())
}

fn drag_block(st: &mut State, index: usize, d: &mut Drag) {
    match d.phase {
        Phase::Down => st.autoexec.drag = Some((index, index)),
        Phase::Move => {
            let heights: Vec<f32> = st.autoexec.blocks.iter().map(Block::height).collect();
            let to = drop_index(&heights, index, d.dy);
            st.autoexec.drag = Some((index, to));
        }
        Phase::Up => {
            if let Some((from, to)) = st.autoexec.drag.take()
                && !d.cancelled()
            {
                st.autoexec.move_block(from, to);
            }
        }
    }
}

fn block_actions(u: &Ui, id: u64, index: usize, count: usize) -> El {
    row()
        .gap(2.0)
        .center_y()
        .group_opacity(0.0, 1.0)
        .child(
            small_button(u, &icons::PLUS, "Add a line below")
                .on_press(move |st: &mut State| {
                    let at = st.autoexec.index_of(id);
                    st.autoexec.insert_after(at);
                }),
        )
        .child(
            small_button(u, &icons::ARROW_UP, "Move up")
                .opacity(if index == 0 { 0.4 } else { 1.0 })
                .on_press(move |st: &mut State| {
                    if let Some(at) = st.autoexec.index_of(id)
                        && at > 0
                    {
                        st.autoexec.move_block(at, at - 1);
                    }
                }),
        )
        .child(
            small_button(u, &ARROW_DOWN, "Move down")
                .opacity(if index + 1 >= count { 0.4 } else { 1.0 })
                .on_press(move |st: &mut State| {
                    if let Some(at) = st.autoexec.index_of(id) {
                        st.autoexec.move_block(at, at + 1);
                    }
                }),
        )
        .child(
            small_button(u, &icons::TRASH_2, "Delete")
                .on_press(move |st: &mut State| st.autoexec.remove(id)),
        )
}

fn small_button(u: &Ui, glyph: &'static Icon, label: &str) -> El {
    el().size(28.0, 28.0)
        .center()
        .radius(crate::ui::r(6.0))
        .hover(u.hover)
        .press(rune::Color::WHITE.alpha(0.08))
        .focusable(true)
        .role("button")
        .label(label)
        .cursor(rune::Cursor::Pointer)
        .child(icon(glyph, 14.0).color(u.muted).a11y_hidden())
}

/// Keys inside a block, before the field: Backspace on an empty block deletes it, arrows move
/// between blocks, Alt+arrows move the block, Escape leaves the field.
fn block_key(st: &mut State, id: u64, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let v = &mut st.autoexec;
    let Some(index) = v.index_of(id) else {
        return false;
    };
    let neighbour = |v: &View, step: isize| -> Option<u64> {
        let mut at = index as isize + step;
        while at >= 0 && (at as usize) < v.blocks.len() {
            let block = &v.blocks[at as usize];
            if block.kind != BlockKind::Blank && block.marker().is_none() {
                return Some(block.id);
            }
            at += step;
        }
        None
    };
    match ev.key {
        Key::Backspace if v.blocks[index].edit.text.is_empty() && !ev.mods.cmd => {
            let previous = neighbour(v, -1);
            v.blocks.remove(index);
            if let Some(previous) = previous {
                v.focus(previous, true);
            } else {
                tx.blur();
            }
            true
        }
        Key::Up if ev.mods.alt => {
            if index > 0 {
                v.move_block(index, index - 1);
            }
            true
        }
        Key::Down if ev.mods.alt => {
            v.move_block(index, index + 1);
            true
        }
        Key::Up if !ev.mods.shift && !ev.mods.cmd => match neighbour(v, -1) {
            Some(previous) => {
                v.focus(previous, true);
                true
            }
            None => false,
        },
        Key::Down if !ev.mods.shift && !ev.mods.cmd => match neighbour(v, 1) {
            Some(next) => {
                v.focus(next, true);
                true
            }
            None => false,
        },
        Key::Escape => {
            v.settle(id);
            tx.blur();
            true
        }
        _ => false,
    }
}

// --- open ----------------------------------------------------------------------------------

/// Show autoexec.cfg in the file manager: selected in its folder when it exists, else the
/// folder it will be created in.
pub fn reveal_autoexec(st: &mut State) {
    let Some(game_root) = st.game_root.clone() else {
        st.status = "Set the game folder in Settings first.".to_string();
        return;
    };
    let target = core::autoexec_reveal_target(&game_root);
    let shown = match &target {
        RevealTarget::SelectFile(path) | RevealTarget::OpenFolder(path) => path.clone(),
    };
    if rune::is_headless() {
        st.status = format!("Headless run: did not open {}.", shown.display());
        return;
    }
    st.status = match open_in_file_manager(&target) {
        Ok(()) => String::new(),
        Err(error) => format!(
            "The file manager could not open {}: {error}",
            shown.display()
        ),
    };
}

#[cfg(target_os = "windows")]
fn open_in_file_manager(target: &RevealTarget) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    // `raw_arg`: explorer needs `/select,"C:\path with spaces"` exactly; `arg` would quote the
    // whole switch and explorer would fall back to Documents.
    let argument = match target {
        RevealTarget::SelectFile(path) => core::windows_select_argument(path),
        RevealTarget::OpenFolder(path) => core::windows_folder_argument(path),
    };
    std::process::Command::new("explorer")
        .raw_arg(argument)
        .spawn()
        .map(|_| ())
}

#[cfg(target_os = "macos")]
fn open_in_file_manager(target: &RevealTarget) -> std::io::Result<()> {
    let mut command = std::process::Command::new("open");
    match target {
        RevealTarget::SelectFile(path) => command.arg("-R").arg(path),
        RevealTarget::OpenFolder(path) => command.arg(path),
    };
    command.spawn().map(|_| ())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_in_file_manager(target: &RevealTarget) -> std::io::Result<()> {
    let folder: PathBuf = match target {
        RevealTarget::SelectFile(path) => path.parent().map(PathBuf::from).unwrap_or_default(),
        RevealTarget::OpenFolder(path) => path.clone(),
    };
    std::process::Command::new("xdg-open")
        .arg(folder)
        .spawn()
        .map(|_| ())
}

// --- save and clear ------------------------------------------------------------------------

/// The save button's approved write. Refuses when the file changed since it was opened and
/// shows what to do instead.
pub fn save(st: &mut State, overwrite: bool) {
    let Some(game_root) = st.game_root.clone() else {
        st.autoexec.last = Some("No Deadlock install was found. Nothing was written.".to_string());
        st.autoexec.confirm_save = false;
        return;
    };
    let text = st.autoexec.text();
    st.autoexec.busy = true;
    let outcome = if overwrite {
        core::write_autoexec(&game_root, &text, true).map(AutoexecSave::Written)
    } else {
        core::save_autoexec_if_unchanged(&game_root, &text, &st.autoexec_original, true)
    };
    st.autoexec.busy = false;
    match outcome {
        Ok(AutoexecSave::Written(written)) => {
            st.autoexec.confirm_save = false;
            st.autoexec.conflict = None;
            st.autoexec_original = core::normalize_trailing_newline(&text);
            st.autoexec.exists = true;
            st.autoexec.path = written.path.display().to_string();
            st.autoexec.last = Some(match &written.backup {
                Some(backup) => format!(
                    "Saved. The previous file is kept as {}.",
                    backup
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| backup.display().to_string())
                ),
                None => "Saved. The game runs it on the next start.".to_string(),
            });
            // Undo restores the copy this save took. A first save has none, so no Undo.
            let notice = crate::notify::Notice::success("Saved autoexec.cfg");
            let notice = match written.backup {
                Some(backup) => notice.undo(move |st: &mut State| undo_save(st, backup)),
                None => notice,
            };
            crate::notify::push(st, notice);
        }
        Ok(AutoexecSave::ChangedOnDisk { current }) => {
            st.autoexec.conflict = Some(current);
        }
        Err(error) => {
            st.autoexec.confirm_save = false;
            st.autoexec.last = Some(error.to_string());
        }
    }
}

/// GAME WRITE, from the save toast's Undo: put back the autoexec.cfg the save copied aside
/// (`backups::restore` copies the file it replaces aside too), then reread it.
fn undo_save(st: &mut State, backup: PathBuf) {
    use crate::notify::{self, Notice};
    match deadlock_core::backups::restore(&backup, true) {
        Ok(_) => {
            crate::data::load_autoexec_now(st);
            st.autoexec.last = Some("Put the previous file back.".to_string());
            notify::push(st, Notice::info("Put the previous autoexec.cfg back"));
        }
        Err(error) => notify::push(
            st,
            Notice::error("Could not undo the save").detail(error.to_string()),
        ),
    }
}

/// Drop the edits and load what the file holds now (after a conflict).
fn reload_from_conflict(st: &mut State) {
    if let Some(current) = st.autoexec.conflict.take() {
        st.autoexec.set_text(&current);
        st.autoexec_original = current;
        st.autoexec.exists = true;
    }
    st.autoexec.confirm_save = false;
}

fn save_layer(s: &State, u: &Ui) -> El {
    let close = |st: &mut State| {
        st.autoexec.confirm_save = false;
        st.autoexec.conflict = None;
    };
    let v = &s.autoexec;
    let conflict = v.conflict.is_some();
    let (title, body) = if conflict {
        (
            tr("autoexec.cfg changed"),
            tr("Something else wrote to it after you opened it. Use the file as it is now, or save yours over it."),
        )
    } else if v.exists {
        (
            tr("Save autoexec.cfg"),
            tr("The current file is backed up first. The game reads it on the next start."),
        )
    } else {
        (
            tr("Save autoexec.cfg"),
            tr("This creates the file. The game reads it on the next start."),
        )
    };

    let mut buttons = row()
        .w_full()
        .gap(8.0)
        .pt(8.0)
        .child(spacer())
        .child(
            u.action(tr("Cancel"), ActionKind::Ghost)
                .cursor(rune::Cursor::Pointer)
                .on_press(close),
        );
    buttons = if conflict {
        buttons
            .child(
                u.action(tr("Use the file"), ActionKind::Secondary)
                    .key("autoexec-load-theirs")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(reload_from_conflict),
            )
            .child(
                u.action(tr("Save mine"), ActionKind::Primary)
                    .key("autoexec-save-mine")
                    .cursor(rune::Cursor::Pointer)
                    .on_press(|st: &mut State| save(st, true)),
            )
    } else {
        buttons.child(
            u.action(tr("Save"), ActionKind::Primary)
                .key("autoexec-save-confirm")
                .cursor(rune::Cursor::Pointer)
                .on_press(|st: &mut State| save(st, false)),
        )
    };

    u.dialog_shell()
        .key("autoexec-save-dialog")
        .role("dialog")
        .label(title)
        .w(460.0)
        .padding(24.0)
        .gap(16.0)
        .modal(true, close)
        .on_key(move |st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                close(st);
                true
            } else {
                false
            }
        })
        .child(
            col()
                .w_full()
                .gap(6.0)
                .child(text(title).font(H1).color(u.ink))
                .child(text(body).font(BODY).color(u.muted).wrap()),
        )
        .child(
            col()
                .w_full()
                .child(u.kv(tr("Commands"), &v.command_count().to_string(), u.ink)),
        )
        .child(buttons)
}

fn clear_layer(s: &State, u: &Ui) -> El {
    let close = |st: &mut State| st.autoexec.confirm_clear = false;
    let lines = s.autoexec.line_count();
    u.dialog_shell()
        .key("autoexec-clear-dialog")
        .role("dialog")
        .label("Clear all commands")
        .w(440.0)
        .padding(24.0)
        .gap(16.0)
        .modal(true, close)
        .on_key(move |st: &mut State, ev: &KeyEvent, _| {
            if ev.key == Key::Escape {
                close(st);
                true
            } else {
                false
            }
        })
        .child(
            col()
                .w_full()
                .gap(6.0)
                .child(text(tr("Clear all commands?")).font(H1).color(u.ink))
                .child(
                    text(if lines == 1 {
                        tr("Removes the one line from the list. autoexec.cfg stays as it is until you save.").to_string()
                    } else {
                        fill(
                            tr("Removes all {} lines from the list. autoexec.cfg stays as it is until you save."),
                            &[&lines.to_string()],
                        )
                    })
                    .font(BODY)
                    .color(u.muted)
                    .wrap(),
                ),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
                .pt(8.0)
                .child(spacer())
                .child(
                    u.action(tr("Cancel"), ActionKind::Ghost)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(close),
                )
                .child(
                    u.action(tr("Clear all"), ActionKind::Danger)
                        .key("autoexec-clear-confirm")
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|st: &mut State| {
                            st.autoexec.blocks.clear();
                            st.autoexec.drag = None;
                            st.autoexec.confirm_clear = false;
                        }),
                ),
        )
}

// --- external editor -----------------------------------------------------------------------

/// Open autoexec.cfg in the editor chosen in Settings (the system default when none is).
fn open_in_editor(st: &mut State) {
    let Some(game_root) = st.game_root.clone() else {
        st.autoexec.last = Some(tr("Set the game folder in Settings first.").to_string());
        return;
    };
    let path = core::autoexec_path(&game_root);
    if !path.is_file() {
        st.autoexec.last = Some(
            tr("There is no autoexec.cfg yet. Save a command first, then open it in your editor.")
                .to_string(),
        );
        return;
    }
    crate::screens::settings::open_in_editor(st, &path);
}

// --- launch options ------------------------------------------------------------------------

fn simulated(st: &State) -> bool {
    st.demo || rune::is_headless()
}

/// Read the saved value and what Steam holds, off the UI thread (finding Steam runs a
/// process check).
fn start_launch_read(st: &mut State) {
    if st.autoexec.launch.read.is_running() {
        return;
    }
    if simulated(st) {
        st.autoexec.launch.loaded = true;
        return;
    }
    let root = st.game_root.clone();
    let wake = st.wake.clone();
    st.autoexec.launch.read.start(wake, move || {
        let saved = launch_options::load(&crate::data::data_dir());
        Ok((saved, launch_options::steam_state(root.as_deref())))
    });
}

/// Take finished launch option jobs and start the first read on the Autoexec page. The app
/// shell calls it every poll (through `crosshair::poll` until app.rs calls it directly).
pub fn poll(st: &mut State) -> bool {
    let mut changed = false;
    if st.screen == Screen::Autoexec && !st.autoexec.launch.loaded && !st.autoexec.launch.read.is_running() {
        start_launch_read(st);
        changed = true;
    }
    if let Some(result) = st.autoexec.launch.read.try_take() {
        let launch = &mut st.autoexec.launch;
        launch.loaded = true;
        match result {
            Ok((saved, steam)) => {
                if launch.field.text == launch.saved.value {
                    launch.field = TextEdit::new(&saved.value);
                }
                launch.saved = saved;
                launch.steam = Some(steam);
            }
            Err(error) => launch.line = Some((error, true)),
        }
        changed = true;
    }
    if let Some(result) = st.autoexec.launch.write.try_take() {
        st.autoexec.launch.line = Some(match result {
            Ok(line) => (line, false),
            Err(error) => (error, true),
        });
        st.autoexec.launch.loaded = false;
        changed = true;
    }
    changed
}

/// Save the field as the launch options Sigil writes at launch. App data only.
fn save_launch(st: &mut State) {
    let value = st.autoexec.launch.field.text.clone();
    if simulated(st) {
        let clean = value.trim().to_string();
        st.autoexec.launch.field = TextEdit::new(&clean);
        st.autoexec.launch.saved = Saved {
            value: clean,
            managed: true,
        };
        st.autoexec.launch.line = Some((tr("Saved. Sigil writes them into Steam the next time it launches the game.").to_string(), false));
        return;
    }
    match launch_options::save(&crate::data::data_dir(), &value) {
        Ok(saved) => {
            st.autoexec.launch.field = TextEdit::new(&saved.value);
            st.autoexec.launch.saved = saved;
            st.autoexec.launch.line = Some((
                tr("Saved. Sigil writes them into Steam the next time it launches the game.").to_string(),
                false,
            ));
            st.autoexec.launch.loaded = false;
        }
        Err(error) => st.autoexec.launch.line = Some((error.to_string(), true)),
    }
}

/// What a sync did, as one line for the page or a toast.
fn sync_line(outcome: &LaunchSync) -> (String, bool) {
    match outcome {
        LaunchSync::NotManaged => (tr("No launch options saved in Sigil yet.").to_string(), false),
        LaunchSync::InSync => (tr("Steam already has these launch options.").to_string(), false),
        LaunchSync::NoSteamConfig => (
            tr("Steam config not found. Start Deadlock from Steam once, then try again.").to_string(),
            true,
        ),
        LaunchSync::SteamRunning => (
            tr("Steam is running and would undo the change when it closes. Quit Steam, then try again.").to_string(),
            true,
        ),
        LaunchSync::Written { .. } => (
            tr("Written to Steam. The old localconfig.vdf is backed up next to it.").to_string(),
            false,
        ),
    }
}

/// The "Write now" button: save the field when it changed, then write it into Steam.
fn write_launch_now(st: &mut State) {
    if st.autoexec.launch.write.is_running() {
        return;
    }
    if st.autoexec.launch.field.text.trim() != st.autoexec.launch.saved.value {
        save_launch(st);
    }
    if simulated(st) {
        st.autoexec.launch.line = Some(sync_line(&LaunchSync::SteamRunning));
        return;
    }
    let root = st.game_root.clone();
    let wake = st.wake.clone();
    st.autoexec.launch.write.start(wake, move || {
        let outcome = launch_options::sync_at_launch(&crate::data::data_dir(), root.as_deref())
            .map_err(|error| error.to_string())?;
        let (line, bad) = sync_line(&outcome);
        if bad { Err(line) } else { Ok(line) }
    });
}

/// GAME-ADJACENT WRITE hook for the launch buttons: call it right before handing off to Steam.
/// Writes the saved launch options into Steam's localconfig.vdf when they differ, skips while
/// Steam runs, and never blocks the launch. Says what happened in a toast only when it matters.
pub fn sync_launch_options(st: &mut State) {
    if simulated(st) {
        return;
    }
    let root = st.game_root.clone();
    match launch_options::sync_at_launch(&crate::data::data_dir(), root.as_deref()) {
        Ok(LaunchSync::Written { .. }) => {
            st.autoexec.launch.loaded = false;
            crate::notify::push(
                st,
                crate::notify::Notice::info(tr("Launch options written to Steam")),
            );
        }
        Ok(LaunchSync::SteamRunning) => crate::notify::push(
            st,
            crate::notify::Notice::warning(tr("Launch options not written"))
                .detail(tr("Steam was already running. Quit Steam before launching from Sigil so they apply.")),
        ),
        Ok(_) => {}
        Err(error) => crate::notify::push(
            st,
            crate::notify::Notice::warning(tr("Launch options not written")).detail(error.to_string()),
        ),
    }
}

fn launch_field(st: &mut State) -> &mut TextEdit {
    &mut st.autoexec.launch.field
}

fn launch_section(s: &State, u: &Ui) -> El {
    let launch = &s.autoexec.launch;
    let draft = launch.field.text.trim();
    let dirty = draft != launch.saved.value;
    let writing = launch.write.is_running();

    let mut head = row()
        .key("autoexec-launch-head")
        .w_full()
        .h(32.0)
        .gap(8.0)
        .center_y()
        .child(u.section(tr("Launch options")))
        .child(spacer());
    let steam = launch.steam.as_ref();
    let differs = steam.is_some_and(|steam| {
        steam.path.is_some() && steam.current.as_deref().unwrap_or("") != launch.saved.value
    });
    if launch.saved.managed && (differs || dirty) {
        head = head.child(
            u.action(tr(if writing { "Writing" } else { "Write to Steam now" }), ActionKind::Ghost)
                .key("autoexec-launch-write")
                .disabled(writing)
                .cursor(rune::Cursor::Pointer)
                .on_press(write_launch_now),
        );
    }

    let field = row()
        .key("autoexec-launch-field")
        .grow(1.0)
        .min_w(0.0)
        .h(32.0)
        .px(10.0)
        .center_y()
        .radius(crate::ui::r(8.0))
        .fill(u.well)
        .border(1.0, u.hair)
        .child(
            rune::text_input(&launch.field, MONO)
                .key("autoexec-launch-input")
                .label(tr("Launch options"))
                .single_line()
                .grow(1.0)
                .min_w(0.0)
                .color(u.ink)
                .placeholder("-high -novid".to_string(), u.faint)
                .caret(u.accent, u.accent.alpha(0.20))
                .on_edit(launch_field)
                .on_submit(|st: &mut State, _| save_launch(st)),
        );
    let save = u
        .action(tr("Save"), if dirty { ActionKind::Primary } else { ActionKind::Secondary })
        .key("autoexec-launch-save")
        .disabled(!dirty)
        .opacity(if dirty { 1.0 } else { 0.45 })
        .cursor(rune::Cursor::Pointer)
        .on_press(|st: &mut State| {
            let launch = &st.autoexec.launch;
            if launch.field.text.trim() != launch.saved.value {
                save_launch(st);
            }
        });

    let mut out = col()
        .key("autoexec-launch")
        .w_full()
        .gap(8.0)
        .child(head)
        .child(
            text(tr("Passed to Deadlock when Steam starts it. Sigil writes them into Steam right before it launches the game."))
                .font(SMALL)
                .color(u.muted)
                .wrap(),
        )
        .child(row().w_full().gap(8.0).center_y().child(field).child(save));

    let note = |key: &str, line: String, warn: bool| {
        row()
            .key(key.to_string())
            .w_full()
            .gap(8.0)
            .items_start()
            .child(
                el().pt(2.0).child(
                    icon(if warn { &icons::TRIANGLE_ALERT } else { &icons::INFO }, 14.0)
                        .color(if warn { u.amber } else { u.faint })
                        .a11y_hidden(),
                ),
            )
            .child(text(line).font(SMALL).color(u.muted).grow(1.0).min_w(0.0).wrap())
    };
    if let Some(steam) = steam {
        if steam.path.is_none() {
            out = out.child(note(
                "autoexec-launch-missing",
                tr("Steam config not found. Start Deadlock from Steam once, then come back.").to_string(),
                true,
            ));
        } else {
            let current = steam.current.clone().unwrap_or_default();
            if current != launch.saved.value {
                out = out.child(
                    row()
                        .key("autoexec-launch-steam")
                        .w_full()
                        .h(36.0)
                        .gap(12.0)
                        .center_y()
                        .border_sides([0.0, 0.0, 1.0, 0.0], u.hair)
                        .child(text(tr("In Steam now")).font(BODY).color(u.muted))
                        .child(spacer())
                        .child(
                            text(if current.is_empty() { tr("Empty").to_string() } else { current })
                                .font(MONO)
                                .color(u.ink)
                                .max_w(520.0)
                                .truncate(),
                        ),
                );
                if launch.saved.managed {
                    out = out.child(note(
                        "autoexec-launch-overwrite",
                        tr("Your saved value replaces it the next time you launch from Sigil.").to_string(),
                        false,
                    ));
                }
            }
            if steam.steam_running && launch.saved.managed {
                out = out.child(note(
                    "autoexec-launch-running",
                    tr("Steam is running. Quit it before launching from Sigil so the change sticks.").to_string(),
                    true,
                ));
            }
        }
    }
    if let Some((line, bad)) = &launch.line {
        out = out.child(note("autoexec-launch-line", line.clone(), *bad));
    }
    out
}

// --- scenes --------------------------------------------------------------------------------

/// A populated file for snapshots: headings, presets, a bind, an unknown line, a managed block.
pub const SCENE_FILE: &str = "// Performance\r\nfps_max 144\r\nr_low_latency 2\r\n\r\n// Network\r\nrate 1000000\r\n\r\n// Binds\r\nbind \"F5\" \"toggleconsole\"\r\nalias +jumpthrow \"+jump;-attack\"\r\ncitadel_unit_status_use_new true\r\n\r\n// dah:crosshair begin\r\ncitadel_crosshair_color_r 255\r\n// dah:crosshair end\r\n";

fn scene_base(s: &mut State, text: &str, exists: bool) {
    s.screen = Screen::Autoexec;
    s.game_detected = true;
    let root = PathBuf::from("/nonexistent/Deadlock");
    s.autoexec.path = core::autoexec_path(&root).display().to_string();
    s.game_root = Some(root);
    s.status.clear();
    s.autoexec.set_text(text);
    s.autoexec_original = text.to_string();
    s.autoexec.loaded = true;
    s.autoexec.exists = exists;
    s.autoexec.error = None;
}

/// `autoexec-file`: a populated file, one preset edited so Save shows.
pub fn scene_file(s: &mut State) {
    scene_base(s, SCENE_FILE, true);
    s.autoexec.add_preset("fps_max 0");
}

/// `autoexec-presets`: no file yet, the matchmaking tab, one preset added.
pub fn scene_presets(s: &mut State) {
    scene_base(s, "", false);
    s.autoexec.category = 4;
    s.autoexec.add_preset("citadel_region_override 1");
}

/// `autoexec-empty`: a file with no lines yet.
pub fn scene_empty(s: &mut State) {
    scene_base(s, "", false);
}

/// `autoexec-clear`: the clear confirm over a populated file.
pub fn scene_clear(s: &mut State) {
    scene_base(s, SCENE_FILE, true);
    s.autoexec.confirm_clear = true;
}

/// `autoexec-conflict`: the save confirm after another writer changed the file.
pub fn scene_conflict(s: &mut State) {
    scene_file(s);
    s.autoexec.confirm_save = true;
    s.autoexec.conflict = Some(SCENE_FILE.to_string());
}

/// `autoexec-launch`: launch options saved, Steam holding an older value while it runs.
pub fn scene_launch(s: &mut State) {
    scene_base(s, "fps_max 144\r\n", true);
    let launch = &mut s.autoexec.launch;
    launch.loaded = true;
    launch.saved = Saved {
        value: "-high -novid -fullscreen".to_string(),
        managed: true,
    };
    launch.field = TextEdit::new("-high -novid -fullscreen +fps_max 240");
    launch.steam = Some(SteamState {
        path: Some(PathBuf::from("/Steam/userdata/1/config/localconfig.vdf")),
        current: Some("-novid".to_string()),
        steam_running: true,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_round_trip_the_file() {
        let mut view = View::default();
        for sample in [SCENE_FILE, "", "a\nb", "\u{feff}// x\n\n\nfps_max 0", "  //  spaced\n"] {
            view.set_text(sample);
            assert_eq!(view.text(), sample);
            assert!(!view.dirty(sample));
        }
    }

    #[test]
    fn presets_replace_the_same_variable() {
        let mut view = View::default();
        view.set_text(SCENE_FILE);
        view.add_preset("fps_max 0");
        assert!(view.text().contains("\r\nfps_max 0\r\n"));
        assert!(!view.text().contains("fps_max 144"));
        let before = view.text();
        view.add_preset("fps_max 0");
        assert_eq!(view.text(), before);
        view.add_preset("minimap_update_rate_hz 60");
        assert!(view.text().ends_with("// dah:crosshair end\r\nminimap_update_rate_hz 60\r\n"));
    }

    #[test]
    fn comments_edit_their_words_only() {
        let mut view = View::default();
        view.set_text("  //  Old\nfps_max 0\n");
        view.blocks[0].edit = TextEdit::new("New");
        assert_eq!(view.text(), "  //  New\nfps_max 0\n");
    }

    #[test]
    fn moving_and_pasting_keep_lines_whole() {
        let mut view = View::default();
        view.set_text("a\nb");
        view.move_block(1, 0);
        assert_eq!(view.text(), "b\na\n");
        let id = view.insert_after(Some(0));
        view.paste_lines(id, "x 1\n// y\nz 2");
        assert_eq!(view.text(), "b\nx 1\n// y\nz 2\na\n");
    }

    #[test]
    fn drop_index_walks_mixed_heights() {
        let heights = [COMMENT_H, COMMAND_H, BLANK_H, COMMAND_H];
        assert_eq!(drop_index(&heights, 1, 0.0), 1);
        assert_eq!(drop_index(&heights, 1, -30.0), 0);
        assert_eq!(drop_index(&heights, 1, 40.0), 3);
        assert_eq!(drop_index(&heights, 1, 999.0), 3);
        assert_eq!(drop_index(&heights, 3, -999.0), 0);
    }
}
