use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    style::Print,
    terminal::{self, ClearType},
};

use crossterm::style::{Color, ResetColor, SetForegroundColor};

use syntect::{
    easy::HighlightLines,
    highlighting::ThemeSet,
    parsing::SyntaxSet,
};

use std::io::{stdout, Write};
use std::path::PathBuf;



#[derive(PartialEq)]
enum Mode {
    Normal,
    Insert,
    Command,
    SavePrompt,
    OpenPrompt,
    StatusMsg,
}

/// Returned by handle_command so the main loop knows whether to quit.
enum CommandResult {
    Quit,
    Stay,
}

fn main() -> std::io::Result<()> {
    terminal::enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, terminal::EnterAlternateScreen)?;

    let result = run(&mut stdout);

    execute!(stdout, terminal::LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;

    result
}

fn buffer_to_string(lines: &[Vec<char>]) -> String {
    lines
        .iter()
        .map(|l| l.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn native_save_dialog(suggested: Option<&str>) -> Option<PathBuf> {
    #[cfg(feature = "native-dialog")]
    {
        let mut dialog = rfd::FileDialog::new()
            .add_filter("Text files", &["txt", "rs", "md"])
            .add_filter("All files", &["*"]);
        if let Some(name) = suggested {
            dialog = dialog.set_file_name(name);
        }
        return dialog.save_file();
    }
    #[cfg(not(feature = "native-dialog"))]
    {
        let _ = suggested;
        None
    }
}

fn write_file(path: &PathBuf, content: &str) -> std::io::Result<()> {
    std::fs::write(path, content)
}

/// Shared save logic. Returns true if the file was successfully written
/// (so :wq knows it can quit).
fn do_save(
    mode: &mut Mode,
    dirty: &mut bool,
    file_path: &mut Option<PathBuf>,
    save_prompt_buf: &mut String,
    status_msg: &mut String,
    lines: &[Vec<char>],
) -> bool {
    let content = buffer_to_string(lines);

    if let Some(path) = file_path.clone() {
        match write_file(&path, &content) {
            Ok(_) => {
                *dirty = false;
                *status_msg = format!("Saved: {}", path.display());
                *mode = Mode::StatusMsg;
                true
            }
            Err(e) => {
                *status_msg = format!("Error saving: {}", e);
                *mode = Mode::StatusMsg;
                false
            }
        }
    } else if let Some(path) = native_save_dialog(None) {
        match write_file(&path, &content) {
            Ok(_) => {
                *dirty = false;
                *status_msg = format!("Saved: {}", path.display());
                *file_path = Some(path);
                *mode = Mode::StatusMsg;
                true
            }
            Err(e) => {
                *status_msg = format!("Error saving: {}", e);
                *mode = Mode::StatusMsg;
                false
            }
        }
    } else {
        // Fall back to in-terminal filename prompt.
        save_prompt_buf.clear();
        *mode = Mode::SavePrompt;
        false
    }
}

/// Handle a completed command string (the text after `:` + Enter).
fn handle_command(
    cmd: &str,
    mode: &mut Mode,
    dirty: &mut bool,
    file_path: &mut Option<PathBuf>,
    save_prompt_buf: &mut String,
    open_prompt_buf: &mut String,
    status_msg: &mut String,
    lines: &[Vec<char>],
) -> CommandResult {
    match cmd {
        "q" => {
            if *dirty {
                *status_msg =
                    "Unsaved changes! Use :w to save, or :q! to force quit.".to_string();
                *mode = Mode::StatusMsg;
                CommandResult::Stay
            } else {
                CommandResult::Quit
            }
        }
        "q!" => CommandResult::Quit,
        "w" => {
            do_save(mode, dirty, file_path, save_prompt_buf, status_msg, lines);
            CommandResult::Stay
        }
        "wq" => {
            let saved = do_save(mode, dirty, file_path, save_prompt_buf, status_msg, lines);
            if saved {
                CommandResult::Quit
            } else {
                CommandResult::Stay
            }
        }
        "op" => {
            open_prompt_buf.clear();
            *mode = Mode::OpenPrompt;
            CommandResult::Stay
        }
        _ => {
            *mode = Mode::Normal;
            CommandResult::Stay
        }
    }
}

fn run(stdout: &mut impl Write) -> std::io::Result<()> {

    let ps = SyntaxSet::load_defaults_newlines();
    let ts = ThemeSet::load_defaults();

    let theme = &ts.themes["base16-ocean.dark"];

    let mut mode = Mode::Normal;
    let mut command_buffer = String::new();

    let mut lines: Vec<Vec<char>> = vec![Vec::new()];
    let mut row_offset: usize = 0;
    let mut col_offset: usize = 0;
    let mut cursor_x: usize = 0;
    let mut cursor_y: usize = 0;


    let mut dirty = false;
    let mut file_path: Option<PathBuf> = None;


    let mut save_prompt_buf = String::new();
    let mut open_prompt_buf = String::new();
    let mut status_msg = String::new();

    'editor: loop {
        let (_cols, rows) = terminal::size()?;

        let scroll_margin = 5;

        if cursor_y < row_offset + scroll_margin {
            row_offset = cursor_y.saturating_sub(scroll_margin);
        } else if cursor_y >= row_offset + (rows as usize - 1 - scroll_margin) {
            row_offset = cursor_y.saturating_sub(rows as usize - 1 - scroll_margin);
        }

        let horizontal_margin = 8;

        if cursor_x < col_offset + horizontal_margin {
            col_offset = cursor_x.saturating_sub(horizontal_margin);
        } else if cursor_x >= col_offset + _cols as usize - horizontal_margin {
            col_offset = cursor_x.saturating_sub(_cols as usize - horizontal_margin);
        }

        // ── Render ───────────────────────────────────────────────────────────
        execute!(stdout, cursor::Hide)?;


        for (screen_y, line) in lines
            .iter()
            .skip(row_offset)
            .take((rows - 1) as usize)
            .enumerate() {
                execute!(
                    stdout,
                    cursor::MoveTo(0, screen_y as u16),
                    terminal::Clear(ClearType::CurrentLine)
                )?;
            if screen_y as u16 >= rows - 1 {
                break;
            }
            let full_text: String = line.iter().collect();

            let visible_text: String = full_text
                .chars()
                .skip(col_offset)
                .take(_cols as usize)
                .collect();

            let syntax = if let Some(path) = &file_path {
                path.extension()
                    .and_then(|s| s.to_str())
                    .and_then(|ext| ps.find_syntax_by_extension(ext))
                    .unwrap_or_else(|| ps.find_syntax_plain_text())
            } else {
                ps.find_syntax_plain_text()
            };

            let mut h = HighlightLines::new(syntax, theme);

            let ranges = h.highlight_line(&visible_text, &ps).unwrap();

            execute!(stdout, cursor::MoveTo(0, screen_y as u16))?;

            for(style, piece) in ranges {
                let color = Color::Rgb {
                    r: style.foreground.r,
                    g: style.foreground.g,
                    b: style.foreground.b,
                };

                execute!(
                    stdout,
                    SetForegroundColor(color),
                    Print(piece)
                )?;
            }
            execute!(stdout, cursor::Show)?;

            stdout.flush();

            execute!(stdout, ResetColor)?;
        }



        // Status bar
        execute!(stdout, cursor::MoveTo(0, rows - 1))?;
        match mode {
            Mode::Normal => {
                let fname = file_path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "[No Name]".to_string());
                let modified = if dirty { " [+]" } else { "" };
                execute!(stdout, Print(format!("-- NORMAL -- {}{}", fname, modified)))?;
            }
            Mode::Insert     => execute!(stdout, Print("-- INSERT --"))?,
            Mode::Command    => execute!(stdout, Print(format!(":{}", command_buffer)))?,
            Mode::SavePrompt => execute!(stdout, Print(format!("Save as: {}", save_prompt_buf)))?,
            Mode::OpenPrompt => execute!(stdout, Print(format!("Open: {}", open_prompt_buf)))?,
            Mode::StatusMsg  => execute!(stdout, Print(&status_msg))?,
        }

        // Cursor
        match mode {
            Mode::Command => {
                execute!(
                    stdout,
                    cursor::MoveTo(command_buffer.len() as u16 + 1, rows - 1)
                )?;
            }
            Mode::SavePrompt => {
                let col = "Save as: ".len() as u16 + save_prompt_buf.len() as u16;
                execute!(stdout, cursor::MoveTo(col, rows - 1))?;
            }
            Mode::OpenPrompt => {
                let col = "Open: ".len() as u16 + open_prompt_buf.len() as u16;
                execute!(stdout, cursor::MoveTo(col, rows - 1))?;
            }
            _ => {
                execute!(stdout,
                    cursor::MoveTo(
                        (cursor_x - col_offset) as u16,
                        (cursor_y - row_offset) as u16,
                    )
                )?;
            }
        }

        stdout.flush()?;

        // ── Input ────────────────────────────────────────────────────────────
        if let Event::Key(KeyEvent { code, modifiers, kind, .. }) = event::read()? {
            if kind != KeyEventKind::Press {
                continue;
            }

            if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
                break 'editor;
            }

            if mode == Mode::StatusMsg {
                mode = Mode::Normal;
                status_msg.clear();
                continue;
            }

            match mode {
                // ── NORMAL ───────────────────────────────────────────────────
                Mode::Normal => match code {
                    KeyCode::Char('i') => mode = Mode::Insert,
                    KeyCode::Char('a') => {
                        if cursor_x < lines[cursor_y].len() {
                            cursor_x += 1;
                        }
                        mode = Mode::Insert;
                    }
                    KeyCode::Char('o') => {
                        cursor_y += 1;
                        lines.insert(cursor_y, Vec::new());
                        cursor_x = 0;
                        mode = Mode::Insert;
                    }
                    KeyCode::Char(':') => {
                        mode = Mode::Command;
                        command_buffer.clear();
                    }
                    KeyCode::Char('h') | KeyCode::Left => {
                        cursor_x = cursor_x.saturating_sub(1);
                    }
                    KeyCode::Char('l') | KeyCode::Right => {
                        let len = lines[cursor_y].len();
                        if len > 0 && cursor_x + 1 < len {
                            cursor_x += 1;
                        }
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        if cursor_y > 0 {
                            cursor_y -= 1;
                            cursor_x = cursor_x.min(lines[cursor_y].len().saturating_sub(1));
                        }
                    }
                    KeyCode::Char('j') | KeyCode::Down => {
                        if cursor_y + 1 < lines.len() {
                            cursor_y += 1;
                            cursor_x = cursor_x.min(lines[cursor_y].len().saturating_sub(1));
                        }
                    }
                    KeyCode::Char('x') => {
                        let line = &mut lines[cursor_y];
                        if cursor_x < line.len() {
                            line.remove(cursor_x);
                            if cursor_x > 0 && cursor_x >= line.len() {
                                cursor_x = line.len().saturating_sub(1);
                            }
                        }
                    }
                    KeyCode::Char('d') if modifiers.contains(KeyModifiers::CONTROL) => {
                        let jump = (rows as usize) / 2;

                        cursor_y = (cursor_y + jump).min(lines.len().saturating_sub(1));

                        row_offset = (row_offset + jump)
                            .min(lines.len().saturating_sub(rows as usize));
                    }
                    KeyCode::Char('u') if modifiers.contains(KeyModifiers::CONTROL) => {
                        let jump = (rows as usize) / 2;

                        cursor_y = cursor_y.saturating_sub(jump);
                        row_offset = row_offset.saturating_sub(jump);
                    }
                    KeyCode::PageDown => {
                        let jump = rows as usize - 2;

                        cursor_y = (cursor_y + jump).min(lines.len().saturating_sub(1));

                        row_offset = (row_offset + jump
                            .min(lines.len()).saturating_sub(rows as usize));
                    }
                    KeyCode::PageUp => {
                        let jump = rows as usize - 2;

                        cursor_y = cursor_y.saturating_sub(jump);
                        row_offset = row_offset.saturating_sub(jump);
                    }
                    KeyCode::Char('z') => {
                        row_offset = cursor_y.saturating_sub((rows as usize) / 2);
                    }
                    _ => {}
                },

                // ── INSERT ───────────────────────────────────────────────────
                Mode::Insert => match code {
                    KeyCode::Esc => {
                        cursor_x = cursor_x.saturating_sub(1);
                        mode = Mode::Normal;
                    }
                    KeyCode::Enter => {
                        let tail: Vec<char> = lines[cursor_y].drain(cursor_x..).collect();
                        cursor_y += 1;
                        lines.insert(cursor_y, tail);
                        cursor_x = 0;
                        dirty = true;
                    }
                    KeyCode::Backspace => {
                        if cursor_x > 0 {
                            cursor_x -= 1;
                            lines[cursor_y].remove(cursor_x);
                            dirty = true;
                        } else if cursor_y > 0 {
                            let current = lines.remove(cursor_y);
                            cursor_y -= 1;
                            cursor_x = lines[cursor_y].len();
                            lines[cursor_y].extend(current);
                            dirty = true;
                        }
                    }
                    KeyCode::Left  => { cursor_x = cursor_x.saturating_sub(1); }
                    KeyCode::Right => {
                        if cursor_x < lines[cursor_y].len() { cursor_x += 1; }
                    }
                    KeyCode::Up => {
                        if cursor_y > 0 {
                            cursor_y -= 1;
                            cursor_x = cursor_x.min(lines[cursor_y].len());
                        }
                    }
                    KeyCode::Down => {
                        if cursor_y + 1 < lines.len() {
                            cursor_y += 1;
                            cursor_x = cursor_x.min(lines[cursor_y].len());
                        }
                    }
                    KeyCode::Char(c) => {
                        lines[cursor_y].insert(cursor_x, c);
                        cursor_x += 1;
                        dirty = true;
                    }
                    _ => {}
                },

                // ── COMMAND ──────────────────────────────────────────────────
                // Kept deliberately thin — only buffer input here.
                // All command logic lives in handle_command().
                Mode::Command => match code {
                    KeyCode::Esc => {
                        mode = Mode::Normal;
                        command_buffer.clear();
                    }
                    KeyCode::Backspace => { command_buffer.pop(); }
                    KeyCode::Char(c)   => { command_buffer.push(c); }
                    KeyCode::Enter => {
                        let cmd = command_buffer.trim().to_string();
                        command_buffer.clear();
                        let result = handle_command(
                            &cmd,
                            &mut mode,
                            &mut dirty,
                            &mut file_path,
                            &mut save_prompt_buf,
                            &mut open_prompt_buf,
                            &mut status_msg,
                            &lines,
                        );
                        if let CommandResult::Quit = result {
                            break 'editor;
                        }
                    }
                    _ => {}
                },

                // ── SAVE PROMPT ───────────────────────────────────────────────
                Mode::SavePrompt => match code {
                    KeyCode::Esc => {
                        status_msg = "Save cancelled.".to_string();
                        mode = Mode::StatusMsg;
                        save_prompt_buf.clear();
                    }
                    KeyCode::Backspace => { save_prompt_buf.pop(); }
                    KeyCode::Char(c)   => { save_prompt_buf.push(c); }
                    KeyCode::Enter => {
                        let name = save_prompt_buf.trim().to_string();
                        save_prompt_buf.clear();
                        if name.is_empty() {
                            status_msg = "Save cancelled (no filename given).".to_string();
                            mode = Mode::StatusMsg;
                        } else {
                            let path = PathBuf::from(&name);
                            match write_file(&path, &buffer_to_string(&lines)) {
                                Ok(_) => {
                                    dirty = false;
                                    file_path = Some(path.clone());
                                    status_msg = format!("Saved: {}", path.display());
                                }
                                Err(e) => {
                                    status_msg = format!("Error saving: {}", e);
                                }
                            }
                            mode = Mode::StatusMsg;
                        }
                    }
                    _ => {}
                },

                // ── OPEN PROMPT ───────────────────────────────────────────────
                Mode::OpenPrompt => match code {
                    KeyCode::Esc => {
                        status_msg = "Open cancelled.".to_string();
                        mode = Mode::StatusMsg;
                        open_prompt_buf.clear();
                    }
                    KeyCode::Backspace => { open_prompt_buf.pop(); }
                    KeyCode::Char(c)   => { open_prompt_buf.push(c); }
                    KeyCode::Enter => {
                        let name = open_prompt_buf.trim().to_string();
                        open_prompt_buf.clear();
                        if name.is_empty() {
                            status_msg = "Open cancelled (no path given).".to_string();
                            mode = Mode::StatusMsg;
                        } else {
                            let path = PathBuf::from(&name);
                            match std::fs::read_to_string(&path) {
                                Ok(contents) => {
                                    // Replace the entire buffer with the file's contents.
                                    lines = contents
                                        .lines()
                                        .map(|l| l.chars().collect())
                                        .collect();
                                    // Ensure there is always at least one line.
                                    if lines.is_empty() {
                                        lines.push(Vec::new());
                                    }
                                    cursor_x = 0;
                                    cursor_y = 0;
                                    dirty = false;
                                    file_path = Some(path.clone());
                                    status_msg = format!("Opened: {}", path.display());
                                }
                                Err(e) => {
                                    status_msg = format!("Error opening '{}': {}", path.display(), e);
                                }
                            }
                            mode = Mode::StatusMsg;
                        }
                    }
                    _ => {}
                },

                Mode::StatusMsg => unreachable!(),
            }
        }
    }

    Ok(())
}
