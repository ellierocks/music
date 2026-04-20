use std::{
    env,
    io::Write,
    path::{Path, PathBuf},
};

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

pub struct GraphicsRenderer {
    kitty_supported: bool,
    last_rect: Option<Rect>,
    last_path: Option<PathBuf>,
}

impl GraphicsRenderer {
    pub fn new() -> Self {
        Self {
            kitty_supported: detect_kitty_support(),
            last_rect: None,
            last_path: None,
        }
    }

    pub fn sync<W: Write>(&mut self, writer: &mut W, app: &App) -> anyhow::Result<()> {
        if !self.kitty_supported {
            return Ok(());
        }

        let Some(path) = app.cover_path() else {
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
        self.draw_image(writer, path, rect)?;
        self.last_rect = Some(rect);
        self.last_path = Some(path.to_path_buf());
        Ok(())
    }

    pub fn clear<W: Write>(&mut self, writer: &mut W) -> anyhow::Result<()> {
        if !self.kitty_supported {
            return Ok(());
        }
        self.delete_image(writer)?;
        self.last_rect = None;
        self.last_path = None;
        Ok(())
    }

    fn draw_image<W: Write>(&self, writer: &mut W, path: &Path, rect: Rect) -> anyhow::Result<()> {
        let encoded_path = STANDARD.encode(path.to_string_lossy().as_bytes());

        execute!(writer, SavePosition, MoveTo(rect.x, rect.y))
            .context("failed to position cursor for kitty image")?;

        let command = format!(
            "\x1b_Ga=T,i={IMAGE_ID},p={PLACEMENT_ID},q=2,C=1,t=f,f=100,c={},r={};{}\x1b\\",
            rect.width, rect.height, encoded_path
        );
        writer
            .write_all(command.as_bytes())
            .context("failed to write kitty image command")?;
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

fn detect_kitty_support() -> bool {
    env::var_os("KITTY_WINDOW_ID").is_some()
        || env::var("TERM")
            .map(|term| term.contains("kitty"))
            .unwrap_or(false)
}
