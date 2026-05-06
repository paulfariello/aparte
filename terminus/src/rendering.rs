use std::{
    collections::HashSet,
    ops::{Index, IndexMut},
    sync::atomic::AtomicBool,
};

use crossterm::{
    cursor::{Hide, MoveTo, SetCursorStyle, Show},
    style::{Attribute, SetAttribute},
    terminal::{Clear, ClearType},
};

use crate::{
    charxel::{Charxel, Grapheme, IntoCharxels},
    BgColor, CursorPos, Dimensions, FgColor, Style,
};

#[derive(Default, Copy, Clone, Eq, PartialEq, Debug)]
pub struct ScreenSize {
    width: u16,
    height: u16,
}

impl From<&Dimensions> for ScreenSize {
    fn from(dimensions: &Dimensions) -> Self {
        ScreenSize {
            width: dimensions.width,
            height: dimensions.height,
        }
    }
}

impl From<(u16, u16)> for ScreenSize {
    fn from((width, height): (u16, u16)) -> Self {
        ScreenSize { width, height }
    }
}

#[derive(Default, Debug)]
pub struct OffscreenRenderBuffer {
    lines: Vec<OffscreenLine>,
    size: ScreenSize,
    cursor: CursorPos,
    show_cursor: bool,
    do_bell: AtomicBool,
}

impl Clone for OffscreenRenderBuffer {
    fn clone(&self) -> Self {
        Self {
            lines: self.lines.clone(),
            size: self.size,
            cursor: self.cursor,
            show_cursor: self.show_cursor,
            do_bell: AtomicBool::new(self.do_bell.load(std::sync::atomic::Ordering::Relaxed)),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.lines.clone_from(&source.lines);
        self.size = source.size;
        self.cursor = source.cursor;
        self.show_cursor = source.show_cursor;
        self.do_bell.swap(
            source.do_bell.load(std::sync::atomic::Ordering::Relaxed),
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}

impl Index<u16> for OffscreenRenderBuffer {
    type Output = OffscreenLine;

    fn index(&self, index: u16) -> &Self::Output {
        &self.lines[index as usize]
    }
}

impl IndexMut<u16> for OffscreenRenderBuffer {
    fn index_mut(&mut self, index: u16) -> &mut Self::Output {
        &mut self.lines[index as usize]
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct OffscreenLine {
    charxels: Vec<Charxel>,
}

impl Index<u16> for OffscreenLine {
    type Output = Charxel;

    fn index(&self, index: u16) -> &Self::Output {
        &self.charxels[index as usize]
    }
}

impl IndexMut<u16> for OffscreenLine {
    fn index_mut(&mut self, index: u16) -> &mut Self::Output {
        &mut self.charxels[index as usize]
    }
}

#[derive(Debug)]
struct ContinuousDiff {
    pos: CursorPos,
    charxels: Vec<Charxel>,
}

impl OffscreenRenderBuffer {
    pub fn bell(&self) {
        self.do_bell
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn set_size(&mut self, screen_size: ScreenSize) {
        self.size = screen_size;
        self.lines = vec![
            OffscreenLine {
                charxels: vec![Charxel::default(); self.size.width.into()]
            };
            self.size.height.into()
        ];
    }

    pub fn clear(&mut self) {
        for i in 0..self.size.height {
            for j in 0..self.size.width {
                self[i][j] = Charxel::default();
            }
        }
    }

    pub fn set_cursor(&mut self, pos: CursorPos) {
        self.cursor = pos;
    }

    pub fn set_cursor_visible(&mut self, visible: bool) {
        self.show_cursor = visible;
    }

    pub fn dump_text(&self) -> String {
        let mut out = String::new();
        for (row, line) in self.lines.iter().enumerate() {
            out.push_str(&format!("ROW {:03}:", row));
            for charxel in &line.charxels {
                out.push_str(&format!(
                    " [{} w={}]",
                    charxel.grapheme,
                    charxel.display_width()
                ));
            }
            out.push('\n');
        }
        out
    }

    fn compute_diff(&self, reference_lines: &[OffscreenLine]) -> Vec<ContinuousDiff> {
        let mut diffs: Vec<ContinuousDiff> = vec![];
        let mut current_diff: Option<ContinuousDiff> = None;

        let mut skip = 0;

        for (i, (ref_charxel, charxel)) in reference_lines
            .iter()
            .flat_map(|line| line.charxels.iter())
            .zip(self.lines.iter().flat_map(|line| line.charxels.iter()))
            .enumerate()
        {
            if skip > 0 {
                skip -= 1;
                continue;
            }

            skip = charxel.display_width().saturating_sub(1);
            if charxel == ref_charxel {
                if let Some(diff) = current_diff.take() {
                    diffs.push(diff);
                }
            } else if let Some(diff) = current_diff.as_mut() {
                diff.charxels.push(charxel.clone());
            } else {
                let diff = ContinuousDiff {
                    pos: CursorPos {
                        top: (i / (self.size.width as usize)) as u16,
                        left: (i % (self.size.width as usize)) as u16,
                    },
                    charxels: vec![charxel.clone()],
                };
                current_diff = Some(diff);
            }
        }

        diffs
    }

    fn render_diff<W>(&self, screen: &mut W, diffs: &Vec<ContinuousDiff>)
    where
        W: std::io::Write,
    {
        log::trace!("Diff render");
        for diff in diffs {
            let _ = write!(screen, "{}", MoveTo(diff.pos.left, diff.pos.top));
            Self::render_chunk(screen, &diff.charxels);
        }
    }

    fn apply_diff(&mut self, diffs: Vec<ContinuousDiff>) {
        let width = self.size.width as usize;
        let height = self.size.height as usize;
        for diff in diffs {
            let mut col_offset = 0usize;
            for charxel in diff.charxels.into_iter() {
                let left = diff.pos.left as usize + col_offset;
                let w = charxel.display_width() as usize;
                let row = diff.pos.top as usize + left / width;
                let col = left % width;
                if row < height && col < width {
                    self[row as u16][col as u16] = charxel.clone();
                }
                // For wide chars, mark right-half cells with the same charxel so
                // that when this char is later replaced by a narrower one, the
                // right-half cell is known to differ from any normal buffer content
                // and gets explicitly rewritten (clearing the terminal's right half).
                for extra in 1..w {
                    let right_left = left + extra;
                    let right_row = diff.pos.top as usize + right_left / width;
                    let right_col = right_left % width;
                    if right_row < height && right_col < width {
                        self[right_row as u16][right_col as u16] = charxel.clone();
                    }
                }
                col_offset += w;
            }
        }
    }

    fn render_chunk<W>(screen: &mut W, chunk: &Vec<Charxel>)
    where
        W: std::io::Write,
    {
        // TODO try to be smart and avoid setting and resetting style and color
        let mut current_bg = None;
        let mut current_fg = None;
        let mut current_styles: Option<HashSet<Style>> = None;
        for charxel in chunk {
            if current_styles.as_ref() != Some(&charxel.styles) {
                let _ = write!(screen, "{}", SetAttribute(Attribute::Reset));
                current_bg = None;
                current_fg = None;
                for style in &charxel.styles {
                    let _ = write!(screen, "{}", style);
                }
                current_styles = Some(charxel.styles.clone());
            }
            if Some(charxel.background) != current_bg {
                let _ = write!(screen, "{}", charxel.background);
                current_bg = Some(charxel.background);
            }
            if Some(charxel.foreground) != current_fg {
                let _ = write!(screen, "{}", charxel.foreground);
                current_fg = Some(charxel.foreground);
            }
            let _ = write!(screen, "{}", charxel.grapheme);
        }
    }

    fn render_line<W>(screen: &mut W, line: &[Charxel])
    where
        W: std::io::Write,
    {
        // Like render_chunk but skips continuation placeholder cells that were written
        // by ScreenFrame::write() for wide (multi-column) characters.
        let mut current_bg = None;
        let mut current_fg = None;
        let mut current_styles: Option<HashSet<Style>> = None;
        let mut skip = 0u16;

        for charxel in line {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let w = charxel.display_width();
            skip = w.saturating_sub(1);
            if current_styles.as_ref() != Some(&charxel.styles) {
                let _ = write!(screen, "{}", SetAttribute(Attribute::Reset));
                current_bg = None;
                current_fg = None;
                for style in &charxel.styles {
                    let _ = write!(screen, "{}", style);
                }
                current_styles = Some(charxel.styles.clone());
            }
            if Some(charxel.background) != current_bg {
                let _ = write!(screen, "{}", charxel.background);
                current_bg = Some(charxel.background);
            }
            if Some(charxel.foreground) != current_fg {
                let _ = write!(screen, "{}", charxel.foreground);
                current_fg = Some(charxel.foreground);
            }
            let _ = write!(screen, "{}", charxel.grapheme);
        }
    }

    fn full_render<W>(&self, screen: &mut W)
    where
        W: std::io::Write,
    {
        log::trace!("Full render");
        let _ = write!(screen, "{}", Hide);
        let _ = write!(screen, "{}", MoveTo(0, 0));
        let _ = write!(screen, "{}", Clear(ClearType::All));

        for line in self.lines.iter() {
            Self::render_line(screen, &line.charxels);
        }
    }

    fn render_cursor<W>(&self, screen: &mut W)
    where
        W: std::io::Write,
    {
        log::trace!("Render cursor");
        let _ = write!(
            screen,
            "{}{}{}",
            SetCursorStyle::SteadyBar,
            MoveTo(self.cursor.left, self.cursor.top),
            Show
        );
    }

    pub fn render<W>(&self, screen: &mut W, reference_screen: &mut Self)
    where
        W: std::io::Write,
    {
        let mut cursor_moved = false;
        if self.size != reference_screen.size {
            self.full_render(screen);
            reference_screen.clone_from(self);
            cursor_moved = true;
        } else if self.lines != reference_screen.lines {
            let diff = self.compute_diff(&reference_screen.lines);
            self.render_diff(screen, &diff);
            reference_screen.apply_diff(diff);
            cursor_moved = true;
        }

        if cursor_moved
            || self.cursor != reference_screen.cursor
            || self.show_cursor != reference_screen.show_cursor
        {
            if self.show_cursor {
                self.render_cursor(screen);
            } else {
                let _ = write!(screen, "{}", Hide);
            }
            reference_screen.cursor = self.cursor;
            reference_screen.show_cursor = self.show_cursor;
        }

        if self
            .do_bell
            .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            let _ = write!(screen, "\x07");
        }

        let _ = screen.flush();
    }
}

pub struct ScreenFrame<'a> {
    pub offscreen: &'a mut OffscreenRenderBuffer,
    pub dimensions: &'a Dimensions,
    cursor: CursorPos,
}

impl<'a> ScreenFrame<'a> {
    pub fn new(offscreen: &'a mut OffscreenRenderBuffer, dimensions: &'a Dimensions) -> Self {
        Self {
            offscreen,
            dimensions,
            cursor: CursorPos::default(),
        }
    }

    pub fn set_background(&mut self, color: BgColor) {
        for i in self.dimensions.top..self.dimensions.top + self.dimensions.height {
            for j in self.dimensions.left..self.dimensions.left + self.dimensions.width {
                self.offscreen[i][j].set_background(color);
            }
        }
    }

    pub fn set_foreground(&mut self, color: FgColor) {
        for i in self.dimensions.top..self.dimensions.top + self.dimensions.height {
            for j in self.dimensions.left..self.dimensions.left + self.dimensions.width {
                self.offscreen[i][j].set_foreground(color);
            }
        }
    }

    pub fn set_styles(&mut self, styles: Vec<Style>) {
        for i in self.dimensions.top..self.dimensions.top + self.dimensions.height {
            for j in self.dimensions.left..self.dimensions.left + self.dimensions.width {
                self.offscreen[i][j].set_styles(&styles);
            }
        }
    }

    pub fn write(&mut self, charxels: impl IntoCharxels) {
        for charxel in charxels.into_charxels() {
            if self.cursor.top >= self.dimensions.height
                || self.cursor.left >= self.dimensions.width
            {
                break;
            }

            // Expand tab to spaces up to the next 8-column tab stop so the
            // offscreen buffer stays consistent with terminal rendering.
            if charxel.grapheme.as_str() == "\t" {
                let next_stop = (self.cursor.left / 8 + 1) * 8;
                let spaces = next_stop.min(self.dimensions.width) - self.cursor.left;
                let space = Charxel {
                    grapheme: Grapheme::from(" "),
                    foreground: charxel.foreground,
                    background: charxel.background,
                    styles: charxel.styles.clone(),
                };
                for _ in 0..spaces {
                    if self.cursor.left >= self.dimensions.width {
                        break;
                    }
                    self.offscreen[self.dimensions.top + self.cursor.top]
                        [self.dimensions.left + self.cursor.left] = space.clone();
                    self.cursor.left += 1;
                }
                continue;
            }

            let w = charxel.display_width();
            // Refuse to place a wide char that wouldn't fit in the remaining
            // columns — emitting it would overflow the screen edge.
            if w > 1 && self.cursor.left + w > self.dimensions.width {
                break;
            }
            self.offscreen[self.dimensions.top + self.cursor.top]
                [self.dimensions.left + self.cursor.left] = charxel;
            // Write blank placeholders for the continuation cells of wide characters so
            // that full_render's skip logic and compute_diff's skip logic stay consistent.
            for k in 1..w {
                let col = self.dimensions.left + self.cursor.left + k;
                if col < self.dimensions.left + self.dimensions.width {
                    self.offscreen[self.dimensions.top + self.cursor.top][col] = Charxel::default();
                }
            }
            self.cursor.left += w;
        }
    }

    pub fn write_at<CP>(&mut self, at: CP, str: impl IntoCharxels)
    where
        CP: Into<CursorPos>,
    {
        self.cursor = at.into();
        self.write(str.into_charxels())
    }

    pub fn set_cursor(&mut self, position: CursorPos) {
        self.offscreen.set_cursor(position)
    }

    pub fn set_cursor_visible(&mut self, visible: bool) {
        self.offscreen.set_cursor_visible(visible)
    }

    pub fn width(&self) -> u16 {
        self.dimensions.width
    }

    pub fn height(&self) -> u16 {
        self.dimensions.height
    }
}
