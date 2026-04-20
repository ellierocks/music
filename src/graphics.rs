use std::{env, io::Write, path::PathBuf};

use anyhow::Context;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use crossterm::{
    cursor::{MoveTo, RestorePosition, SavePosition},
    execute,
};
use ratatui::layout::Rect;

use crate::app::App;

const IMAGE_ID: u32 = 7;
const PLACEMENT_ID: u32 = 1;
const MAX_CHUNK_SIZE: usize = 3072;

pub struct GraphicsRenderer {
    graphics_supported: bool,
    last_rect: Option<Rect>,
    last_path: Option<PathBuf>,
}

impl GraphicsRenderer {
    pub fn new() -> Self {
        Self {
            graphics_supported: detect_graphics_support(),
            last_rect: None,
            last_path: None,
        }
    }

    pub fn sync<W: Write>(&mut self, writer: &mut W, app: &App) -> anyhow::Result<()> {
        if !self.graphics_supported {
            return Ok(());
        }

        let Some(path) = app.cover_path() else {
            self.clear(writer)?;
            return Ok(());
        };
        let Some(png_data) = app.cover_png_data() else {
            self.clear(writer)?;
            return Ok(());
        };
        let Some(rect) = app.cover_rect else {
            return Ok(());
        };
        if rect.width == 0 || rect.height == 0 {
            return Ok(());
        }

        let should_redraw = self.last_rect != Some(rect) || self.last_path.as_deref() != Some(path);
        if !should_redraw {
            return Ok(());
        }

        self.delete_image(writer)?;
        self.draw_image(writer, png_data, rect)?;
        self.last_rect = Some(rect);
        self.last_path = Some(path.to_path_buf());
        Ok(())
    }

    pub fn clear<W: Write>(&mut self, writer: &mut W) -> anyhow::Result<()> {
        if !self.graphics_supported {
            return Ok(());
        }
        self.delete_image(writer)?;
        self.last_rect = None;
        self.last_path = None;
        Ok(())
    }

    fn draw_image<W: Write>(
        &self,
        writer: &mut W,
        png_data: &[u8],
        rect: Rect,
    ) -> anyhow::Result<()> {
        execute!(writer, SavePosition, MoveTo(rect.x, rect.y))
            .context("failed to position cursor for kitty image")?;

        for (index, chunk) in png_data.chunks(MAX_CHUNK_SIZE).enumerate() {
            let encoded_chunk = STANDARD.encode(chunk);
            let more = usize::from((index + 1) * MAX_CHUNK_SIZE < png_data.len());
            let command = if index == 0 {
                format!(
                    "\x1b_Ga=T,i={IMAGE_ID},p={PLACEMENT_ID},q=2,C=1,f=100,c={},r={},m={};{}\x1b\\",
                    rect.width, rect.height, more, encoded_chunk
                )
            } else {
                format!("\x1b_Gm={};{}\x1b\\", more, encoded_chunk)
            };

            writer
                .write_all(command.as_bytes())
                .context("failed to write kitty image command")?;
        }
        execute!(writer, RestorePosition).context("failed to restore cursor after kitty image")?;
        writer
            .flush()
            .context("failed to flush kitty image command")?;
        Ok(())
    }

    fn delete_image<W: Write>(&self, writer: &mut W) -> anyhow::Result<()> {
        let command = format!("\x1b_Ga=d,d=I,i={IMAGE_ID},p={PLACEMENT_ID},q=2\x1b\\");
        writer
            .write_all(command.as_bytes())
            .context("failed to write kitty image delete command")?;
        writer
            .flush()
            .context("failed to flush kitty image delete command")?;
        Ok(())
    }
}

fn detect_graphics_support() -> bool {
    if env_flag("MUSIC_DISABLE_GRAPHICS") {
        return false;
    }
    if env_flag("MUSIC_FORCE_GRAPHICS") {
        return true;
    }

    env::var_os("KITTY_WINDOW_ID").is_some()
        || env::var("TERM")
            .map(|term| term.contains("kitty") || term.contains("xterm-kitty"))
            .unwrap_or(false)
        || env::var("TERM_PROGRAM")
            .map(|program| {
                matches!(
                    program.to_ascii_lowercase().as_str(),
                    "wezterm" | "ghostty" | "kitty"
                )
            })
            .unwrap_or(false)
        || env::var_os("WEZTERM_EXECUTABLE").is_some()
        || env::var_os("GHOSTTY_BIN_DIR").is_some()
}

fn env_flag(key: &str) -> bool {
    env::var(key)
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}
