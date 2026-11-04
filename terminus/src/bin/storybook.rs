/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

//! Interactive storybook for terminus components.
//!
//! Navigation:
//!   n / Enter / Right arrow  – next story
//!   p / Left arrow           – previous story
//!   q / Esc / Ctrl-C         – quit
//!
//! Usage:
//!   cargo run --bin storybook                   # interactive browser
//!   cargo run --bin storybook -- --list          # print story names and exit
//!   cargo run --bin storybook -- --story label   # jump straight to a story

use std::io::{stdout, Write};

use crossterm::{
    cursor::Hide,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use terminus::{
    label::Label,
    rendering::{OffscreenRenderBuffer, ScreenSize},
    stories::{all_stories, render_view_into, Story},
    Dimensions, View,
};

/// Render a story into a full-screen buffer with a header and footer bar.
fn render_frame(
    story: &Story,
    current: usize,
    total: usize,
    width: u16,
    height: u16,
) -> OffscreenRenderBuffer {
    let header = format!(
        " [{}/{}] {} — {}",
        current + 1,
        total,
        story.name,
        story.description
    );

    let mut buf = OffscreenRenderBuffer::default();
    buf.set_size(ScreenSize::from((width, height)));

    render_label(&mut buf, &header, 0, width);

    let content_height = height.saturating_sub(2);
    if content_height > 0 {
        let content_dims = Dimensions {
            top: 1,
            left: 0,
            width,
            height: content_height,
        };
        let mut view = story.build();
        render_view_into(&mut buf, view.as_mut(), &content_dims);
    }

    if height > 1 {
        render_label(
            &mut buf,
            " n/→: next   p/←: prev   q/Esc: quit",
            height - 1,
            width,
        );
    }

    buf
}

fn render_label(buf: &mut OffscreenRenderBuffer, text: &str, row: u16, width: u16) {
    let dims = Dimensions {
        top: row,
        left: 0,
        width,
        height: 1,
    };
    let mut label: Box<dyn View<(), ()>> = Box::new(Label::new(text));
    render_view_into(buf, label.as_mut(), &dims);
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let stories = all_stories();

    // --list: print all story names and exit
    if args.iter().any(|a| a == "--list") {
        for (i, s) in stories.iter().enumerate() {
            println!("{}: {} — {}", i + 1, s.name, s.description);
        }
        return Ok(());
    }

    // --story <name>: jump to a specific story
    let initial = if let Some(pos) = args.iter().position(|a| a == "--story") {
        let name = args.get(pos + 1).map(String::as_str).unwrap_or("");
        stories
            .iter()
            .position(|s| s.name == name)
            .unwrap_or_else(|| {
                eprintln!("unknown story {:?}; available:", name);
                for s in &stories {
                    eprintln!("  {}", s.name);
                }
                std::process::exit(1);
            })
    } else {
        0
    };

    if stories.is_empty() {
        eprintln!("no stories defined");
        return Ok(());
    }

    terminal::enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen, Hide)?;

    let mut current = initial;
    let mut reference = OffscreenRenderBuffer::default();

    let result = run_loop(&stories, &mut current, &mut reference, &mut out);

    execute!(out, LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;

    result
}

fn run_loop(
    stories: &[Story],
    current: &mut usize,
    reference: &mut OffscreenRenderBuffer,
    out: &mut impl Write,
) -> anyhow::Result<()> {
    loop {
        let (cols, rows) = terminal::size()?;
        let buf = render_frame(&stories[*current], *current, stories.len(), cols, rows);
        buf.render(out, reference);

        match event::read()? {
            Event::Key(KeyEvent {
                code: KeyCode::Char('q'),
                ..
            })
            | Event::Key(KeyEvent {
                code: KeyCode::Esc, ..
            })
            | Event::Key(KeyEvent {
                code: KeyCode::Char('c'),
                modifiers: KeyModifiers::CONTROL,
                ..
            }) => break,

            Event::Key(KeyEvent {
                code: KeyCode::Char('n') | KeyCode::Enter | KeyCode::Right,
                ..
            }) => {
                *current = (*current + 1) % stories.len();
                *reference = OffscreenRenderBuffer::default();
            }

            Event::Key(KeyEvent {
                code: KeyCode::Char('p') | KeyCode::Left,
                ..
            }) => {
                *current = if *current == 0 {
                    stories.len() - 1
                } else {
                    *current - 1
                };
                *reference = OffscreenRenderBuffer::default();
            }

            Event::Resize(_, _) => {
                *reference = OffscreenRenderBuffer::default();
            }

            _ => {}
        }
    }
    Ok(())
}
