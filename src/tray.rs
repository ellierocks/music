use std::{
    env, fs,
    path::Path,
    process::{Command, Stdio},
    sync::OnceLock,
};

use anyhow::anyhow;
use image::{Rgba, RgbaImage};
use ksni::{Handle, TrayMethods, menu::StandardItem};
use tokio::sync::mpsc;

use crate::ipc;

#[derive(Clone, Debug)]
pub enum TrayCommand {
    ShowPlayer,
    TogglePause,
    Next,
    Previous,
    Stop,
    Quit,
}

pub async fn spawn(
    tx: mpsc::UnboundedSender<TrayCommand>,
) -> anyhow::Result<Handle<MyTray>> {
    MyTray { tx }
        .spawn()
        .await
        .map_err(|error| anyhow!(error.to_string()))
}

pub struct MyTray {
    tx: mpsc::UnboundedSender<TrayCommand>,
}

impl ksni::Tray for MyTray {
    fn id(&self) -> String {
        "music".into()
    }

    fn title(&self) -> String {
        "Music".into()
    }

    fn icon_name(&self) -> String {
        String::new()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![tray_icon().clone()]
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.tx.send(TrayCommand::ShowPlayer).ok();
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        vec![
            StandardItem {
                label: "Show Player".into(),
                activate: Box::new(|tray: &mut Self| {
                    tray.tx.send(TrayCommand::ShowPlayer).ok();
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Play / Pause".into(),
                activate: Box::new(|tray: &mut Self| {
                    tray.tx.send(TrayCommand::TogglePause).ok();
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Previous".into(),
                activate: Box::new(|tray: &mut Self| {
                    tray.tx.send(TrayCommand::Previous).ok();
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Next".into(),
                activate: Box::new(|tray: &mut Self| {
                    tray.tx.send(TrayCommand::Next).ok();
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Stop".into(),
                activate: Box::new(|tray: &mut Self| {
                    tray.tx.send(TrayCommand::Stop).ok();
                }),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|tray: &mut Self| {
                    tray.tx.send(TrayCommand::Quit).ok();
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

fn tray_icon() -> &'static ksni::Icon {
    static ICON: OnceLock<ksni::Icon> = OnceLock::new();
    ICON.get_or_init(build_tray_icon)
}

fn build_tray_icon() -> ksni::Icon {
    let mut image = RgbaImage::from_pixel(32, 32, Rgba([0, 0, 0, 0]));
    let edge = [255, 255, 255, 144];
    let white = [255, 255, 255, 255];

    fill_circle(&mut image, 10, 23, 6, edge);
    fill_circle(&mut image, 23, 18, 6, edge);
    fill_rect(&mut image, 13, 7, 4, 16, edge);
    fill_rect(&mut image, 26, 4, 4, 14, edge);
    fill_rect(&mut image, 13, 5, 17, 3, edge);
    fill_rect(&mut image, 16, 3, 14, 3, edge);

    fill_circle(&mut image, 10, 23, 5, white);
    fill_circle(&mut image, 23, 18, 5, white);
    fill_rect(&mut image, 14, 7, 2, 15, white);
    fill_rect(&mut image, 27, 4, 2, 13, white);
    fill_rect(&mut image, 14, 5, 15, 2, white);
    fill_rect(&mut image, 17, 3, 12, 2, white);

    let (width, height) = image.dimensions();
    let mut data = image.into_vec();
    for pixel in data.chunks_exact_mut(4) {
        pixel.rotate_right(1);
    }

    ksni::Icon {
        width: width as i32,
        height: height as i32,
        data,
    }
}

fn fill_rect(image: &mut RgbaImage, x: u32, y: u32, width: u32, height: u32, color: [u8; 4]) {
    for dy in 0..height {
        for dx in 0..width {
            image.put_pixel(x + dx, y + dy, Rgba(color));
        }
    }
}

fn fill_circle(image: &mut RgbaImage, cx: i32, cy: i32, radius: i32, color: [u8; 4]) {
    let radius_sq = radius * radius;
    for y in (cy - radius)..=(cy + radius) {
        for x in (cx - radius)..=(cx + radius) {
            let dx = x - cx;
            let dy = y - cy;
            if dx * dx + dy * dy > radius_sq || x < 0 || y < 0 {
                continue;
            }

            let x = x as u32;
            let y = y as u32;
            if x < image.width() && y < image.height() {
                image.put_pixel(x, y, Rgba(color));
            }
        }
    }
}

pub fn reopen_terminal(current_exe: &Path) -> anyhow::Result<()> {
    if ipc::client_is_showing() {
        return Ok(());
    }

    for candidate in terminal_candidates(current_exe) {
        if !command_exists(&candidate.program) {
            continue;
        }

        if Command::new(&candidate.program)
            .args(&candidate.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
        {
            return Ok(());
        }
    }

    Err(anyhow!("failed to open a terminal for the music client"))
}

struct TerminalCommand {
    program: String,
    args: Vec<String>,
}

fn terminal_candidates(current_exe: &Path) -> Vec<TerminalCommand> {
    let exe = current_exe.display().to_string();
    vec![
        TerminalCommand {
            program: "kitty".into(),
            args: vec!["-e".into(), exe.clone(), "--client".into()],
        },
        TerminalCommand {
            program: "wezterm".into(),
            args: vec!["start".into(), "--".into(), exe.clone(), "--client".into()],
        },
        TerminalCommand {
            program: "ghostty".into(),
            args: vec!["-e".into(), exe.clone(), "--client".into()],
        },
        TerminalCommand {
            program: "x-terminal-emulator".into(),
            args: vec!["-e".into(), exe.clone(), "--client".into()],
        },
        TerminalCommand {
            program: "gnome-terminal".into(),
            args: vec!["--".into(), exe.clone(), "--client".into()],
        },
        TerminalCommand {
            program: "konsole".into(),
            args: vec!["-e".into(), exe.clone(), "--client".into()],
        },
        TerminalCommand {
            program: "alacritty".into(),
            args: vec!["-e".into(), exe, "--client".into()],
        },
    ]
}

fn command_exists(program: &str) -> bool {
    let Some(paths) = env::var_os("PATH") else {
        return false;
    };

    env::split_paths(&paths).any(|path| fs::metadata(path.join(program)).is_ok())
}
