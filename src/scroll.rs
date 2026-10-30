//! The bit that makes it `sl`: the picture enters from the right edge and
//! leaves by the left, animating as it goes, with the caption riding under it.

use crate::art::{Art, Mood};
use anyhow::Result;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute, queue,
    style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io::{self, Write};
use std::time::{Duration, Instant};

pub struct ScrollOpts {
    pub speed_ms: u64,
    pub frame_hold: u64,
    pub stubborn: bool,
    pub footer: String,
}

pub fn colour(mood: Mood) -> Color {
    match mood {
        Mood::Red => Color::Red,
        Mood::Green => Color::Green,
        Mood::Yellow => Color::Yellow,
        Mood::Blue => Color::Blue,
        Mood::Magenta => Color::Magenta,
        Mood::Cyan => Color::Cyan,
        Mood::White => Color::White,
        Mood::Orange => Color::Rgb { r: 255, g: 107, b: 53 },
    }
}

/// Raw mode plus the alternate screen, undone on every exit path.
struct Screen;

impl Screen {
    fn enter() -> Result<Screen> {
        terminal::enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, cursor::Hide, Clear(ClearType::All))?;
        Ok(Screen)
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), ResetColor, cursor::Show, LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

pub fn run(art: &Art, opts: &ScrollOpts) -> Result<()> {
    let _screen = Screen::enter()?;
    let mut out = io::BufWriter::new(io::stdout());

    let width = art.width() as i64;
    let height = art.height() as i64;
    let caption: Vec<char> = art.caption.chars().collect();
    let travel = width.max(caption.len() as i64);
    let frames: Vec<Vec<Vec<char>>> = art
        .frames
        .iter()
        .map(|f| f.iter().map(|l| l.chars().collect()).collect())
        .collect();
    let hold = opts.frame_hold.max(1) as i64;
    let speed = Duration::from_millis(opts.speed_ms.max(1));
    let ink = colour(art.mood);

    let (cols0, _) = terminal::size()?;
    let mut x: i64 = cols0 as i64;
    let mut tick: i64 = 0;

    loop {
        let (cols, rows) = terminal::size()?;
        let cols = cols as i64;
        let rows = rows as i64;
        let frame = &frames[((tick / hold) as usize) % frames.len()];
        let band = height + 2;
        let top = ((rows - 1 - band) / 2).max(0);

        queue!(out, SetForegroundColor(ink))?;
        for (i, line) in frame.iter().enumerate() {
            let row = top + i as i64;
            if row >= rows - 1 {
                break;
            }
            queue!(out, cursor::MoveTo(0, row as u16), Print(window(line, x, cols)))?;
        }
        let caption_row = top + height + 1;
        if caption_row < rows - 1 {
            queue!(
                out,
                SetAttribute(Attribute::Italic),
                cursor::MoveTo(0, caption_row as u16),
                Print(window(&caption, x, cols)),
                SetAttribute(Attribute::Reset)
            )?;
        }
        let footer: Vec<char> = opts.footer.chars().collect();
        queue!(
            out,
            ResetColor,
            SetAttribute(Attribute::Dim),
            cursor::MoveTo(0, (rows - 1) as u16),
            Print(window(&footer, 0, cols)),
            SetAttribute(Attribute::Reset)
        )?;
        out.flush()?;

        if x + travel < 0 {
            break;
        }

        let deadline = Instant::now() + speed;
        loop {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            if event::poll(deadline - now)? {
                match event::read()? {
                    Event::Key(k) if k.kind == KeyEventKind::Press => {
                        let quit = matches!(k.code, KeyCode::Char('q') | KeyCode::Esc)
                            || (k.code == KeyCode::Char('c')
                                && k.modifiers.contains(KeyModifiers::CONTROL));
                        if quit && !opts.stubborn {
                            return Ok(());
                        }
                    }
                    Event::Resize(_, _) => {
                        queue!(out, Clear(ClearType::All))?;
                    }
                    _ => {}
                }
            }
        }
        x -= 1;
        tick += 1;
    }
    Ok(())
}

/// The `cols`-wide strip of the terminal row, with `line` placed at column
/// `x` (which may be negative or beyond the right edge) and spaces elsewhere.
fn window(line: &[char], x: i64, cols: i64) -> String {
    let mut s = String::with_capacity(cols as usize);
    for col in 0..cols {
        let idx = col - x;
        if idx >= 0 && (idx as usize) < line.len() {
            s.push(line[idx as usize]);
        } else {
            s.push(' ');
        }
    }
    s
}

/// No animation: the first frame and the caption, for pipes and `print`.
pub fn print_plain(art: &Art, out: &mut dyn Write) -> Result<()> {
    if let Some(frame) = art.frames.first() {
        for line in frame {
            writeln!(out, "{}", line.trim_end())?;
        }
    }
    writeln!(out)?;
    writeln!(out, "  {}", art.caption)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_clips_both_edges() {
        let line: Vec<char> = "abcdef".chars().collect();
        assert_eq!(window(&line, 0, 4), "abcd");
        assert_eq!(window(&line, -2, 4), "cdef");
        assert_eq!(window(&line, 2, 4), "  ab");
        assert_eq!(window(&line, 10, 4), "    ");
        assert_eq!(window(&line, -10, 4), "    ");
    }
}
