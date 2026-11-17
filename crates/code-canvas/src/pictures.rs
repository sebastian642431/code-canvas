use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};

const SIDE: u32 = 1024;
const KEPT: usize = 96;
const AT_ONCE: usize = 3;
const MAX_BYTES: u64 = 64 << 20;

type Picture = Result<(egui::ColorImage, [u32; 2]), &'static str>;
type Read = (PathBuf, Picture);

enum Slot {
    Reading,
    Shown { texture: egui::TextureHandle, size: [u32; 2], used: u64 },
    Failed(&'static str),
}

pub struct Pictures {
    slots: HashMap<PathBuf, Slot>,
    tx: Sender<Read>,
    rx: Receiver<Read>,
    frame: u64,
}

fn read(path: &Path) -> Picture {
    match std::fs::metadata(path).map(|file| file.len()) {
        Ok(0) => return Err("empty"),
        Ok(bytes) if bytes > MAX_BYTES => return Err("too big to show here"),
        Ok(_) => {}
        Err(_) => return Err("cannot be read"),
    }
    let whole = image::open(path).map_err(|why| match why {
        image::ImageError::Unsupported(_) => "a kind of picture that cannot be shown here",
        image::ImageError::IoError(_) => "cannot be read",
        _ => "not a picture, or a damaged one",
    })?;
    let size = [whole.width(), whole.height()];
    let shown = if size[0].max(size[1]) > SIDE { whole.thumbnail(SIDE, SIDE) } else { whole }.to_rgba8();
    let pixels = [shown.width() as usize, shown.height() as usize];
    Ok((egui::ColorImage::from_rgba_unmultiplied(pixels, shown.as_raw()), size))
}

impl Pictures {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Pictures { slots: HashMap::new(), tx, rx, frame: 0 }
    }

    pub fn begin_frame(&mut self, ctx: &egui::Context) {
        self.frame += 1;
        while let Ok((path, read)) = self.rx.try_recv() {
            let slot = match read {
                Ok((image, size)) => {
                    let name = path.to_string_lossy().into_owned();
                    let texture = ctx.load_texture(name, image, egui::TextureOptions::LINEAR);
                    Slot::Shown { texture, size, used: self.frame }
                }
                Err(why) => Slot::Failed(why),
            };
            self.slots.insert(path, slot);
        }
        let shown = self.slots.values().filter(|slot| matches!(slot, Slot::Shown { .. })).count();
        if shown > KEPT {
            let mut ages: Vec<u64> = self.slots.values().filter_map(|slot| slot.used()).collect();
            ages.sort_unstable();
            let cut = ages[shown - KEPT];
            self.slots.retain(|_, slot| slot.used().is_none_or(|used| used > cut));
        }
    }

    pub fn get(&mut self, path: &Path) -> Option<(egui::TextureId, [u32; 2])> {
        let frame = self.frame;
        match self.slots.get_mut(path) {
            Some(Slot::Shown { texture, size, used }) => {
                *used = frame;
                return Some((texture.id(), *size));
            }
            Some(_) => return None,
            None => {}
        }
        let reading = self.slots.values().filter(|slot| matches!(slot, Slot::Reading)).count();
        if reading < AT_ONCE {
            self.slots.insert(path.to_path_buf(), Slot::Reading);
            let (tx, path) = (self.tx.clone(), path.to_path_buf());
            std::thread::spawn(move || {
                let picture = read(&path);
                let _ = tx.send((path, picture));
                crate::ext::wake_ui();
            });
        }
        None
    }

    pub fn problem(&self, path: &Path) -> Option<&'static str> {
        match self.slots.get(path) {
            Some(Slot::Failed(why)) => Some(why),
            _ => None,
        }
    }

    pub fn forget(&mut self, path: &Path) {
        self.slots.remove(path);
    }
}

impl Slot {
    fn used(&self) -> Option<u64> {
        match self {
            Slot::Shown { used, .. } => Some(*used),
            _ => None,
        }
    }
}
