use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use bevy::prelude::*;

pub const TILE_SIZE: f32 = 32.0;

#[derive(Resource, Clone)]
pub struct LevelMap {
    pub width: i32,
    pub height: i32,
    pub walls: HashSet<IVec2>,
    pub flag: IVec2,
    pub hero_start: IVec2,
    pub key_pos: Option<IVec2>,
    pub lock_pos: Option<IVec2>,
    pub decorations: Vec<Decoration>,
    pub tiles: HashMap<IVec2, TileKind>,
}

impl LevelMap {
    pub fn is_wall(&self, pos: IVec2) -> bool {
        self.walls.contains(&pos)
    }
}

#[derive(Resource, Clone)]
pub struct LevelAssets {
    pub ground_main: Handle<Image>,
    pub ground_top: Handle<Image>,
    pub background_base: Handle<Image>,
    pub background_row0: Handle<Image>,
    pub background_row1: Vec<Handle<Image>>,
    pub flag_frames: Vec<Handle<Image>>,
    pub hero: Handle<Image>,
    pub hero_frames: Vec<Handle<Image>>,
    pub key: Handle<Image>,
    pub lock: Handle<Image>,
    pub decor_cloud: Handle<Image>,
    pub decor_plant: Handle<Image>,
}

#[derive(Clone, Copy)]
pub enum DecorationKind {
    Cloud,
    Plant,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum BgTileKind {
    Base,
    Row0,
    Row1A,
    Row1B,
    Row1C,
    Row1D,
}

#[derive(Clone, Copy)]
pub enum TileKind {
    GroundMain,
    GroundTop,
}

#[derive(Clone, Copy)]
pub struct Decoration {
    pub kind: DecorationKind,
    pub pos: IVec2,
}

#[derive(Clone)]
pub struct BackgroundMap {
    pub width: i32,
    pub height: i32,
    pub tiles: HashMap<IVec2, BgTileKind>,
    pub ground_tiles: HashMap<IVec2, TileKind>,
    pub walls: HashSet<IVec2>,
}

#[derive(Clone)]
pub struct LevelDefinition {
    pub name: String,
    pub background: BackgroundMap,
    pub foreground: LevelMap,
    pub template: String,
    pub evaluate: String,
}

#[derive(Resource)]
pub struct Levels {
    pub entries: Vec<LevelDefinition>,
    pub current: usize,
}

pub fn parse_level(text: &str) -> LevelMap {
    let walls = HashSet::new();
    let mut flag = IVec2::ZERO;
    let mut hero_start = IVec2::ZERO;
    let mut key_pos = None;
    let mut lock_pos = None;
    let mut decorations = Vec::new();
    let tiles = HashMap::new();
    let mut width = 0;

    let lines: Vec<&str> = text.lines().collect();
    let height = lines.len() as i32;
    for (row, line) in lines.iter().enumerate() {
        width = width.max(line.chars().count() as i32);
        for (col, ch) in line.chars().enumerate() {
            let pos = IVec2::new(col as i32, height - 1 - row as i32);
            match ch {
                'F' => {
                    flag = pos;
                }
                'H' => {
                    hero_start = pos;
                }
                'K' => {
                    key_pos = Some(pos);
                }
                'L' => {
                    lock_pos = Some(pos);
                }
                'C' => decorations.push(Decoration {
                    kind: DecorationKind::Cloud,
                    pos,
                }),
                'P' => decorations.push(Decoration {
                    kind: DecorationKind::Plant,
                    pos,
                }),
                _ => {}
            }
        }
    }

    LevelMap {
        width,
        height,
        walls,
        flag,
        hero_start,
        key_pos,
        lock_pos,
        decorations,
        tiles,
    }
}

pub fn parse_background(text: &str) -> BackgroundMap {
    let mut tiles = HashMap::new();
    let mut ground_tiles = HashMap::new();
    let mut walls = HashSet::new();
    let mut width = 0;
    let lines: Vec<&str> = text.lines().collect();
    let height = lines.len() as i32;
    for (row, line) in lines.iter().enumerate() {
        width = width.max(line.chars().count() as i32);
        for (col, ch) in line.chars().enumerate() {
            let pos = IVec2::new(col as i32, height - 1 - row as i32);
            match ch {
                '-' => {
                    ground_tiles.insert(pos, TileKind::GroundTop);
                    walls.insert(pos);
                }
                '#' => {
                    ground_tiles.insert(pos, TileKind::GroundMain);
                    walls.insert(pos);
                }
                _ => {
                    let kind = match ch {
                        '1' => BgTileKind::Row0,
                        '2' => BgTileKind::Row1A,
                        '3' => BgTileKind::Row1B,
                        '4' => BgTileKind::Row1C,
                        '5' => BgTileKind::Row1D,
                        _ => BgTileKind::Base,
                    };
                    tiles.insert(pos, kind);
                }
            }
        }
    }
    BackgroundMap {
        width,
        height,
        tiles,
        ground_tiles,
        walls,
    }
}

pub fn load_levels(asset_root: &Path) -> Result<Levels, String> {
    let levels_root = asset_root.join("levels");
    let mut entries = Vec::new();
    let mut dirs: Vec<PathBuf> = fs::read_dir(&levels_root)
        .map_err(|err| format!("Failed to read levels directory: {err}"))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.path())
        .collect();
    dirs.sort();

    for dir in dirs {
        let name = dir
            .file_name()
            .and_then(|os| os.to_str())
            .unwrap_or("level")
            .to_string();
        if name.starts_with('_') {
            continue;
        }
        let background_path = dir.join("background.txt");
        let foreground_path = dir.join("foreground.txt");
        let template_path = dir.join("template.py");
        let evaluate_path = dir.join("evaluate.py");

        if !background_path.exists()
            || !foreground_path.exists()
            || !template_path.exists()
            || !evaluate_path.exists()
        {
            continue;
        }

        let background_text = fs::read_to_string(&background_path)
            .map_err(|err| format!("Failed to read {background_path:?}: {err}"))?;
        let foreground_text = fs::read_to_string(&foreground_path)
            .map_err(|err| format!("Failed to read {foreground_path:?}: {err}"))?;
        let template = fs::read_to_string(&template_path)
            .map_err(|err| format!("Failed to read {template_path:?}: {err}"))?;
        let evaluate = fs::read_to_string(&evaluate_path)
            .map_err(|err| format!("Failed to read {evaluate_path:?}: {err}"))?;

        let background = parse_background(&background_text);
        let mut foreground = parse_level(&foreground_text);
        if background.width != foreground.width || background.height != foreground.height {
            return Err(format!(
                "Level {name} background/foreground size mismatch: {}x{} vs {}x{}",
                background.width, background.height, foreground.width, foreground.height
            ));
        }
        foreground.walls.extend(background.walls.iter().copied());
        foreground.tiles.extend(background.ground_tiles.iter().map(|(pos, tile)| (*pos, *tile)));

        entries.push(LevelDefinition {
            name,
            background,
            foreground,
            template,
            evaluate,
        });
    }

    if entries.is_empty() {
        return Err("No level folders found under assets/levels".to_string());
    }

    Ok(Levels {
        entries,
        current: 0,
    })
}

pub fn grid_to_world(pos: IVec2) -> Vec3 {
    Vec3::new(pos.x as f32 * TILE_SIZE, pos.y as f32 * TILE_SIZE, 0.0)
}
