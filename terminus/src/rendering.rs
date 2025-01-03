use std::{
    ops::{Index, IndexMut},
    sync::atomic::AtomicBool,
};

use crate::{
    charxel::{Charxel, IntoCharxels},
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
    do_bell: AtomicBool,
}

impl Clone for OffscreenRenderBuffer {
    fn clone(&self) -> Self {
        Self {
            lines: self.lines.clone(),
            size: self.size,
            cursor: self.cursor,
            do_bell: AtomicBool::new(self.do_bell.load(std::sync::atomic::Ordering::Relaxed)),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.lines.clone_from(&source.lines);
        self.size = source.size;
        self.cursor = source.cursor;
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

            skip = charxel.display_width() - 1;
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
            let _ = write!(
                screen,
                "{}",
                termion::cursor::Goto(diff.pos.left + 1, diff.pos.top + 1)
            );
            Self::render_chunk(screen, &diff.charxels);
        }
    }

    fn apply_diff(&mut self, diffs: Vec<ContinuousDiff>) {
        for diff in diffs {
            for (i, charxel) in diff.charxels.into_iter().enumerate() {
                let width = self.size.width as usize;
                let left = diff.pos.left as usize + i;
                self[diff.pos.top + ((left / width) as u16)][(left % width) as u16] = charxel;
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
        for charxel in chunk {
            if Some(charxel.background) != current_bg {
                let _ = write!(screen, "{}", charxel.background);
                current_bg = Some(charxel.background);
            }
            if Some(charxel.foreground) != current_fg {
                let _ = write!(screen, "{}", charxel.foreground);
                current_fg = Some(charxel.foreground);
            }
            // TODO style
            let _ = write!(screen, "{}", charxel.grapheme);
        }
    }

    fn full_render<W>(&self, screen: &mut W)
    where
        W: std::io::Write,
    {
        log::trace!("Full render");
        let _ = write!(screen, "{}", termion::cursor::Hide,);
        let _ = write!(screen, "{}", termion::cursor::Goto(1, 1));
        let _ = write!(screen, "{}", termion::clear::All);

        for line in self.lines.iter() {
            Self::render_chunk(screen, &line.charxels);
        }
    }

    fn render_cursor<W>(&self, screen: &mut W)
    where
        W: std::io::Write,
    {
        log::trace!("Render cursor");
        let _ = write!(
            screen,
            "{}{}",
            termion::cursor::Goto(self.cursor.left + 1, self.cursor.top + 1),
            termion::cursor::Show
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

        if cursor_moved || self.cursor != reference_screen.cursor {
            self.render_cursor(screen);
            reference_screen.cursor = self.cursor;
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
            self.offscreen[self.dimensions.top + self.cursor.top]
                [self.dimensions.left + self.cursor.left] = charxel;

            self.cursor.left += 1;
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

    pub fn width(&self) -> u16 {
        self.dimensions.width
    }

    pub fn height(&self) -> u16 {
        self.dimensions.height
    }
}
