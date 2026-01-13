use std::collections::{HashMap, HashSet};

use bevy::prelude::*;

pub const TILE_SIZE: f32 = 32.0;

#[derive(Resource, Clone)]
pub struct LevelMap {
    pub width: i32,
    pub height: i32,
    pub walls: HashSet<IVec2>,
    pub flag: IVec2,
    pub hero_start: IVec2,
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
    pub decor_cloud: Handle<Image>,
    pub decor_plant: Handle<Image>,
}

#[derive(Clone, Copy)]
pub enum DecorationKind {
    Cloud,
    Plant,
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

pub fn parse_level(text: &str) -> LevelMap {
    let mut walls = HashSet::new();
    let mut flag = IVec2::ZERO;
    let mut hero_start = IVec2::ZERO;
    let mut decorations = Vec::new();
    let mut tiles = HashMap::new();
    let mut width = 0;

    let lines: Vec<&str> = text.lines().collect();
    let height = lines.len() as i32;
    for (row, line) in lines.iter().enumerate() {
        width = width.max(line.chars().count() as i32);
        for (col, ch) in line.chars().enumerate() {
            let pos = IVec2::new(col as i32, height - 1 - row as i32);
            match ch {
                '#' => {
                    walls.insert(pos);
                    tiles.insert(pos, TileKind::GroundMain);
                }
                '-' => {
                    walls.insert(pos);
                    tiles.insert(pos, TileKind::GroundTop);
                }
                'F' => {
                    flag = pos;
                }
                'H' => {
                    hero_start = pos;
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
        decorations,
        tiles,
    }
}

pub fn grid_to_world(pos: IVec2) -> Vec3 {
    Vec3::new(pos.x as f32 * TILE_SIZE, pos.y as f32 * TILE_SIZE, 0.0)
}
