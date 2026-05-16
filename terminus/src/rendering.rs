use std::{
    collections::HashSet,
    ops::{Index, IndexMut},
    sync::atomic::AtomicBool,
};

use unicode_segmentation::UnicodeSegmentation as _;

use crossterm::{
    cursor::{Hide, MoveTo, SetCursorStyle, Show},
    style::{Attribute, SetAttribute},
    terminal::{Clear, ClearType},
};

use crate::{
    charxel::{Charxel, ContinuationCell, Grapheme, IntoCharxels},
    BgColor, CursorPos, CursorStyle, Dimensions, FgColor, Style,
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

#[derive(Debug)]
pub struct OffscreenRenderBuffer {
    lines: Vec<OffscreenLine>,
    size: ScreenSize,
    cursor: CursorPos,
    cursor_priority: u8,
    cursor_style: CursorStyle,
    show_cursor: bool,
    do_bell: AtomicBool,
    force_full_render: AtomicBool,
}

impl Default for OffscreenRenderBuffer {
    fn default() -> Self {
        Self {
            lines: Vec::new(),
            size: ScreenSize::default(),
            cursor: CursorPos::default(),
            cursor_priority: 0,
            cursor_style: CursorStyle::SteadyBar,
            show_cursor: false,
            do_bell: AtomicBool::new(false),
            force_full_render: AtomicBool::new(false),
        }
    }
}

impl Clone for OffscreenRenderBuffer {
    fn clone(&self) -> Self {
        Self {
            lines: self.lines.clone(),
            size: self.size,
            cursor: self.cursor,
            cursor_priority: self.cursor_priority,
            cursor_style: self.cursor_style,
            show_cursor: self.show_cursor,
            do_bell: AtomicBool::new(self.do_bell.load(std::sync::atomic::Ordering::Relaxed)),
            force_full_render: AtomicBool::new(false),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.lines.clone_from(&source.lines);
        self.size = source.size;
        self.cursor = source.cursor;
        self.cursor_priority = source.cursor_priority;
        self.cursor_style = source.cursor_style;
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
        self.cursor_priority = 0;
    }

    pub fn set_cursor(&mut self, pos: CursorPos) {
        self.cursor = pos;
    }

    pub fn set_cursor_with_priority(&mut self, pos: CursorPos, priority: u8) {
        if priority >= self.cursor_priority {
            self.cursor = pos;
            self.cursor_priority = priority;
        }
    }

    pub fn set_cursor_style(&mut self, style: CursorStyle) {
        self.cursor_style = style;
    }

    pub fn set_cursor_visible(&mut self, visible: bool) {
        self.show_cursor = visible;
    }

    pub fn request_full_render(&self) {
        self.force_full_render
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn dump_text(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        for (row, line) in self.lines.iter().enumerate() {
            let _ = write!(out, "ROW {row:03}:");
            for charxel in &line.charxels {
                let grapheme = &charxel.grapheme;
                let w = charxel.display_width();
                let _ = write!(out, " [{grapheme} w={w}]");
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
                        #[allow(clippy::cast_possible_truncation)]
                        top: (i / (self.size.width as usize)) as u16,
                        #[allow(clippy::cast_possible_truncation)]
                        left: (i % (self.size.width as usize)) as u16,
                    },
                    charxels: vec![charxel.clone()],
                };
                current_diff = Some(diff);
            }
        }

        if let Some(diff) = current_diff {
            diffs.push(diff);
        }

        diffs
    }

    #[allow(clippy::unused_self)]
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

    #[allow(clippy::cast_possible_truncation)]
    fn apply_diff(&mut self, diffs: Vec<ContinuousDiff>) {
        let width = self.size.width as usize;
        let height = self.size.height as usize;
        for diff in diffs {
            let mut col_offset = 0usize;
            for charxel in diff.charxels {
                let left = diff.pos.left as usize + col_offset;
                let w = charxel.display_width() as usize;
                let row = diff.pos.top as usize + left / width;
                let col = left % width;
                if row < height && col < width {
                    self[row as u16][col as u16] = charxel.clone();
                }
                for extra in 1..w {
                    let right_left = left + extra;
                    let right_row = diff.pos.top as usize + right_left / width;
                    let right_col = right_left % width;
                    if right_row < height && right_col < width {
                        self[right_row as u16][right_col as u16] = Charxel::from(ContinuationCell);
                    }
                }
                col_offset += w;
            }
        }
    }

    #[allow(clippy::similar_names)]
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
                    let _ = write!(screen, "{style}");
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

    #[allow(clippy::similar_names)]
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
                    let _ = write!(screen, "{style}");
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
        let _ = write!(screen, "{Hide}");
        let _ = write!(screen, "{}", MoveTo(0, 0));
        let _ = write!(screen, "{}", Clear(ClearType::All));

        for line in &self.lines {
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
            SetCursorStyle::from(self.cursor_style),
            MoveTo(self.cursor.left, self.cursor.top),
            Show
        );
    }

    pub fn render<W>(&self, screen: &mut W, reference_screen: &mut Self)
    where
        W: std::io::Write,
    {
        let mut cursor_moved = false;
        let force = self
            .force_full_render
            .swap(false, std::sync::atomic::Ordering::Relaxed);
        if force || self.size != reference_screen.size {
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
            || self.cursor_style != reference_screen.cursor_style
        {
            if self.show_cursor {
                self.render_cursor(screen);
            } else {
                let _ = write!(screen, "{Hide}");
            }
            reference_screen.cursor = self.cursor;
            reference_screen.cursor_style = self.cursor_style;
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

    pub fn set_styles(&mut self, styles: &[Style]) {
        for i in self.dimensions.top..self.dimensions.top + self.dimensions.height {
            for j in self.dimensions.left..self.dimensions.left + self.dimensions.width {
                self.offscreen[i][j].set_styles(styles);
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
                    continuation: false,
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
                    self.offscreen[self.dimensions.top + self.cursor.top][col] =
                        Charxel::from(ContinuationCell);
                }
            }
            self.cursor.left += w;
        }
    }

    pub fn highlight_text(&mut self, query: &str, fg: FgColor, bg: BgColor) {
        let query_graphemes: Vec<String> = query.graphemes(true).map(str::to_lowercase).collect();
        let qlen = query_graphemes.len();
        if qlen == 0 {
            return;
        }

        for row in self.dimensions.top..self.dimensions.top + self.dimensions.height {
            let start = self.dimensions.left;
            let end = self.dimensions.left + self.dimensions.width;
            let row_len = (end - start) as usize;
            if row_len < qlen {
                continue;
            }
            let row_graphemes: Vec<String> = (start..end)
                .map(|col| self.offscreen[row][col].grapheme.as_str().to_lowercase())
                .collect();
            for i in 0..=(row_len - qlen) {
                if (0..qlen).all(|j| row_graphemes[i + j] == query_graphemes[j]) {
                    for j in 0..qlen {
                        #[allow(clippy::cast_possible_truncation)]
                        let col = start + (i + j) as u16;
                        self.offscreen[row][col].set_foreground(fg);
                        self.offscreen[row][col].set_background(bg);
                    }
                }
            }
        }
    }

    pub fn write_at<CP>(&mut self, at: CP, str: impl IntoCharxels)
    where
        CP: Into<CursorPos>,
    {
        self.cursor = at.into();
        self.write(str.into_charxels());
    }

    pub fn set_cursor(&mut self, position: CursorPos) {
        self.offscreen.set_cursor(position);
    }

    pub fn set_cursor_with_priority(&mut self, position: CursorPos, priority: u8) {
        self.offscreen.set_cursor_with_priority(position, priority);
    }

    pub fn set_cursor_style(&mut self, style: CursorStyle) {
        self.offscreen.set_cursor_style(style);
    }

    pub fn set_cursor_visible(&mut self, visible: bool) {
        self.offscreen.set_cursor_visible(visible);
    }

    #[must_use]
    pub fn width(&self) -> u16 {
        self.dimensions.width
    }

    #[must_use]
    pub fn height(&self) -> u16 {
        self.dimensions.height
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::charxel::ContinuationCell;

    /// ContinuationCell must produce a Charxel distinct from the default blank.
    #[test]
    fn continuation_cell_is_distinct_from_default() {
        assert_ne!(Charxel::from(ContinuationCell), Charxel::default());
    }

    /// When a wide char is replaced by default-coloured blanks, the position
    /// previously covered by its right half must appear in the diff so the
    /// terminal can clear it explicitly.
    #[test]
    fn compute_diff_clears_continuation_replaced_by_default() {
        // Reference: 🔒 at col 0, continuation sentinel at col 1.
        let mut reference = OffscreenRenderBuffer::default();
        reference.set_size((4u16, 1u16).into());
        reference[0][0] = Charxel::new(Grapheme::from("🔒"));
        reference[0][1] = Charxel::from(ContinuationCell);

        // New render: all Charxel::default() (🔒 was removed, no explicit colour).
        let mut current = OffscreenRenderBuffer::default();
        current.set_size((4u16, 1u16).into());

        let diffs = current.compute_diff(&reference.lines);

        let diffed_cols: Vec<u16> = diffs
            .iter()
            .flat_map(|d| {
                let mut off = 0u16;
                d.charxels.iter().map(move |cx| {
                    let col = d.pos.left + off;
                    off += cx.display_width().max(1);
                    col
                })
            })
            .collect();

        assert!(
            diffed_cols.contains(&1),
            "continuation col 1 not in diff; diffs: {:?}",
            diffs
        );
    }

    #[test]
    fn compute_diff_pushes_last_open_diff() {
        // 2 rows × 3 cols; reference has all 'A', current has all 'A'
        // except the very last cell which is 'B'.
        // The loop in compute_diff ends with an open diff on that cell but
        // never pushes it — so without the fix this test fails.
        let mut reference = OffscreenRenderBuffer::default();
        reference.set_size((3u16, 2u16).into());
        for row in 0..2u16 {
            for col in 0..3u16 {
                reference[row][col].grapheme = Grapheme::from("A");
            }
        }

        let mut current = OffscreenRenderBuffer::default();
        current.set_size((3u16, 2u16).into());
        for row in 0..2u16 {
            for col in 0..3u16 {
                current[row][col].grapheme = Grapheme::from("A");
            }
        }
        // Only the last cell differs.
        current[1][2].grapheme = Grapheme::from("B");

        let diffs = current.compute_diff(&reference.lines);

        assert_eq!(diffs.len(), 1, "last open diff must be pushed");
        assert_eq!(diffs[0].pos.top, 1);
        assert_eq!(diffs[0].pos.left, 2);
        assert_eq!(diffs[0].charxels.len(), 1);
        assert_eq!(diffs[0].charxels[0].grapheme, Grapheme::from("B"));
    }
}
