use std::{
    collections::HashSet,
    ops::{Index, IndexMut},
};

use unicode_segmentation::UnicodeSegmentation;

use crate::{
    is_clean_str, term_string_visible_len, BgColor, CursorPos, Dimensions, FgColor, Style,
};

#[derive(Debug, Clone, PartialEq)]
struct Grapheme(String);

impl std::fmt::Display for Grapheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Default for Grapheme {
    fn default() -> Self {
        Grapheme(String::from(" "))
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Charxel {
    grapheme: Grapheme,
    foreground: FgColor,
    background: BgColor,
    styles: HashSet<Style>,
}

impl Charxel {
    pub fn set_background(&mut self, color: BgColor) {
        self.background = color;
    }

    pub fn set_foreground(&mut self, color: FgColor) {
        self.foreground = color;
    }

    pub fn set_styles(&mut self, styles: Vec<Style>) {
        self.styles = HashSet::from_iter(styles.into_iter());
    }

    pub fn set_grapheme(&mut self, grapheme: String) {
        self.grapheme = Grapheme(grapheme);
    }
}

#[derive(Default, Copy, Clone, Eq, PartialEq)]
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

#[derive(Default, Clone)]
pub struct OffscreenRenderBuffer {
    lines: Vec<OffscreenLine>,
    size: ScreenSize,
    cursor: CursorPos,
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

#[derive(Clone, PartialEq)]
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

    fn compute_diff(&self, reference_lines: &Vec<OffscreenLine>) -> Vec<ContinuousDiff> {
        let mut diffs: Vec<ContinuousDiff> = vec![];
        let mut current_diff: Option<ContinuousDiff> = None;

        for (i, (ref_charxel, charxel)) in reference_lines
            .iter()
            .map(|line| line.charxels.iter())
            .flatten()
            .zip(self.lines.iter().map(|line| line.charxels.iter()).flatten())
            .enumerate()
        {
            if charxel == ref_charxel {
                if let Some(diff) = current_diff.take() {
                    diffs.push(diff);
                }
            } else {
                if let Some(diff) = current_diff.as_mut() {
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
        }

        diffs
    }

    fn render_diff<W>(&self, screen: &mut W, diffs: &Vec<ContinuousDiff>)
    where
        W: std::io::Write,
    {
        log::trace!("Diff render: {:?}", diffs);
        for diff in diffs {
            let _ = write!(
                screen,
                "{}",
                termion::cursor::Goto(diff.pos.left + 1, diff.pos.top + 1)
            );
            Self::render_chunk(screen, &diff.charxels);
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

    pub fn render<W>(&self, screen: &mut W, reference_screen: Option<&Self>)
    where
        W: std::io::Write,
    {
        if let Some(reference_screen) = reference_screen {
            let mut cursor_moved = false;
            if self.size != reference_screen.size {
                self.full_render(screen);
                cursor_moved = true;
            } else if self.lines != reference_screen.lines {
                let diff = self.compute_diff(&reference_screen.lines);
                self.render_diff(screen, &diff);
                cursor_moved = true;
            }

            if cursor_moved || self.cursor != reference_screen.cursor {
                self.render_cursor(screen);
            }
        } else {
            self.full_render(screen);
            self.render_cursor(screen);
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
        for i in self.dimensions.top..self.dimensions.height {
            for j in self.dimensions.left..self.dimensions.width {
                self.offscreen[i][j].set_background(color);
            }
        }
    }

    pub fn set_foreground(&mut self, color: FgColor) {
        for i in self.dimensions.top..self.dimensions.height {
            for j in self.dimensions.left..self.dimensions.width {
                self.offscreen[i][j].set_foreground(color);
            }
        }
    }

    pub fn set_styles(&mut self, styles: Vec<Style>) {
        for i in self.dimensions.top..self.dimensions.height {
            for j in self.dimensions.left..self.dimensions.width {
                self.offscreen[i][j].set_styles(styles.clone());
            }
        }
    }

    fn full_write(
        &mut self,
        str: &str,
        styles: Option<Vec<Style>>,
        background: Option<BgColor>,
        foreground: Option<FgColor>,
    ) {
        assert!(is_clean_str(str));
        for grapheme in str.graphemes(true) {
            let mut charxel = &mut self.offscreen[self.dimensions.top + self.cursor.top]
                [self.dimensions.left + self.cursor.left];

            charxel.set_grapheme(grapheme.to_string());
            if let Some(styles) = styles.as_ref() {
                charxel.set_styles(styles.clone());
            }
            if let Some(background) = background {
                charxel.set_background(background);
            }
            if let Some(foreground) = foreground {
                charxel.set_foreground(foreground);
            }
            self.cursor.left += term_string_visible_len(grapheme) as u16;
        }
    }

    pub fn write<S>(&mut self, str: S)
    where
        S: AsRef<str>,
    {
        self.full_write(str.as_ref(), None, None, None)
    }

    pub fn write_at<CP>(&mut self, at: CP, str: &str)
    where
        CP: Into<CursorPos>,
    {
        self.cursor = at.into();
        log::debug!("write `{}' at {:?}", str, self.cursor);
        self.full_write(str, None, None, None)
    }

    pub fn write_with_style<S>(&mut self, str: S, style: Style)
    where
        S: AsRef<str>,
    {
        self.full_write(str.as_ref(), Some(vec![style]), None, None)
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
