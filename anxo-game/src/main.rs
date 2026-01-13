mod commands;
mod level;
mod python;
mod ui;

use std::time::Duration;

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{Projection, ScalingMode, Viewport};
use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use bevy::window::WindowResolution;
use bevy_egui::{
    EguiContext, EguiGlobalSettings, EguiPlugin, EguiPrimaryContextPass, PrimaryEguiContext,
};
use crossbeam_channel::{Receiver, TryRecvError};

use commands::{Command, Direction};
use level::{
    DecorationKind, LevelAssets, LevelMap, TILE_SIZE, TileKind, grid_to_world, parse_level,
};
use ui::{EditorState, ResetRequest, RunRequest};

#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq, Default)]
enum GamePhase {
    #[default]
    Editing,
    Playing,
    Won,
}

#[derive(Component)]
struct Hero {
    grid_pos: IVec2,
}

#[derive(Component)]
struct Flag;

#[derive(Component)]
struct Moving {
    start: Vec3,
    end: Vec3,
    target_grid: IVec2,
    timer: Timer,
}

#[derive(Component)]
struct ResetAnim {
    total: Timer,
    frame: Timer,
    index: usize,
    base_pos: Vec3,
}

#[derive(Component)]
struct WinAnim {
    total: Timer,
    frame: Timer,
    index: usize,
    base_pos: Vec3,
}
#[derive(Component)]
struct FlagAnim {
    timer: Timer,
    index: usize,
}
type HeroQueryData = (
    Entity,
    &'static mut Hero,
    &'static Transform,
    Option<&'static Moving>,
    Option<&'static ResetAnim>,
);

#[derive(Component)]
struct WorldCamera;

#[derive(Resource, Default)]
struct CommandQueue {
    commands: Vec<Command>,
    index: usize,
}

#[derive(Resource)]
struct PlaybackTimer(Timer);

#[derive(Resource, Default)]
struct PythonTask {
    receiver: Option<Receiver<Result<Vec<Command>, String>>>,
    running: bool,
}

#[derive(Resource, Default)]
struct AutoRun {
    done: bool,
}

#[derive(Resource)]
pub struct UiLayout {
    editor_width: f32,
    editor_left: f32,
    pixels_per_point: f32,
}

#[derive(Resource, Clone, Copy)]
struct PlaceholderMode(bool);

#[derive(Resource, Default)]
struct RunState {
    has_run: bool,
}

fn resolve_asset_root() -> String {
    if let Ok(root) = std::env::var("ANXO_ASSET_ROOT") {
        return root;
    }
    if std::path::Path::new("assets").exists() {
        return "assets".to_string();
    }
    let manifest_assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    if manifest_assets.exists() {
        return manifest_assets.to_string_lossy().into_owned();
    }
    "assets".to_string()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--python-worker") {
        std::process::exit(python::run_worker());
    }

    let asset_root = resolve_asset_root();
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Anxo Game".to_string(),
                        resolution: WindowResolution::new(960, 540),
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .set(AssetPlugin {
                    file_path: asset_root,
                    ..Default::default()
                }),
        )
        .insert_resource(ClearColor(Color::srgb(0.08, 0.08, 0.1)))
        .insert_resource(EguiGlobalSettings {
            auto_create_primary_context: false,
            ..Default::default()
        })
        .add_plugins(EguiPlugin::default())
        .insert_resource(CommandQueue::default())
        .insert_resource(PlaybackTimer(Timer::from_seconds(
            0.2,
            TimerMode::Repeating,
        )))
        .insert_resource(PythonTask::default())
        .insert_resource(GamePhase::default())
        .insert_resource(UiLayout {
            editor_width: 360.0,
            editor_left: 600.0,
            pixels_per_point: 1.0,
        })
        .insert_resource(EditorState {
            code: initial_code(),
            error: None,
        })
        .insert_resource(AutoRun::default())
        .insert_resource(RunState::default())
        .add_message::<RunRequest>()
        .add_message::<ResetRequest>()
        .add_systems(Startup, setup)
        .add_systems(EguiPrimaryContextPass, ui::ui_system)
        .add_systems(
            PreUpdate,
            (
                auto_run_system,
                handle_run_requests,
                poll_python_results,
                update_camera_viewport,
                reset_animation_system,
                win_animation_system,
                flag_animation_system,
                playback_system,
                movement_system,
                win_system,
                reset_system,
            ),
        )
        .run();
}

fn setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    let level_text = include_str!("../assets/levels/level1.txt");
    let level = parse_level(level_text);
    let use_placeholders = std::env::var("ANXO_PLACEHOLDER").ok().as_deref() == Some("1");
    let world_layer = RenderLayers::layer(0);
    let assets = LevelAssets {
        background_base: asset_server.load("kenney_pixel_platformer/Tiles/Backgrounds/tile_0000.png"),
        background_row0: asset_server.load("kenney_pixel_platformer/Tiles/Backgrounds/tile_0016.png"),
        background_row1: vec![
            asset_server.load("kenney_pixel_platformer/Tiles/Backgrounds/tile_0008.png"),
            asset_server.load("kenney_pixel_platformer/Tiles/Backgrounds/tile_0009.png"),
            asset_server.load("kenney_pixel_platformer/Tiles/Backgrounds/tile_0010.png"),
            asset_server.load("kenney_pixel_platformer/Tiles/Backgrounds/tile_0011.png"),
        ],
        ground_main: asset_server.load("kenney_pixel_platformer/Tiles/tile_0104.png"),
        ground_top: asset_server.load("kenney_pixel_platformer/Tiles/tile_0022.png"),
        flag_frames: vec![
            asset_server.load("kenney_pixel_platformer/Tiles/tile_0111.png"),
            asset_server.load("kenney_pixel_platformer/Tiles/tile_0112.png"),
        ],
        hero: asset_server.load("kenney_pixel_platformer/Tiles/Characters/tile_0000.png"),
        hero_frames: vec![
            asset_server.load("kenney_pixel_platformer/Tiles/Characters/tile_0000.png"),
            asset_server.load("kenney_pixel_platformer/Tiles/Characters/tile_0001.png"),
            asset_server.load("kenney_pixel_platformer/Tiles/Characters/tile_0002.png"),
            asset_server.load("kenney_pixel_platformer/Tiles/Characters/tile_0003.png"),
        ],
        decor_cloud: asset_server.load("kenney_pixel_platformer/Tiles/tile_0125.png"),
        decor_plant: asset_server.load("kenney_pixel_platformer/Tiles/tile_0103.png"),
    };

    let camera_center = Vec3::new(
        (level.width as f32 - 1.0) * TILE_SIZE * 0.5,
        (level.height as f32 - 1.0) * TILE_SIZE * 0.5,
        999.0,
    );
    commands.spawn((
        Camera2d,
        Camera {
            order: 0,
            ..Default::default()
        },
        Transform::from_translation(camera_center),
        WorldCamera,
        world_layer.clone(),
    ));
    commands.spawn((
        Camera2d,
        Camera {
            order: 1,
            clear_color: ClearColorConfig::None,
            ..Default::default()
        },
        RenderLayers::layer(1),
        EguiContext::default(),
        PrimaryEguiContext,
    ));

    let max_wall_y = level.walls.iter().map(|pos| pos.y).max().unwrap_or(-1);
    let lowest_empty_row = max_wall_y.saturating_add(1).clamp(0, level.height - 1);
    let second_empty_row = (lowest_empty_row + 1).clamp(0, level.height - 1);

    for y in 0..level.height {
        for x in 0..level.width {
            let pos = IVec2::new(x, y);
            let world_pos = grid_to_world(pos);
            let background_image = if y == lowest_empty_row {
                assets.background_row0.clone()
            } else if y == second_empty_row {
                let index = (x as usize) % assets.background_row1.len().max(1);
                assets
                    .background_row1
                    .get(index)
                    .cloned()
                    .unwrap_or_else(|| assets.background_base.clone())
            } else {
                assets.background_base.clone()
            };
            commands.spawn((
                if use_placeholders {
                    Sprite {
                        color: Color::srgb(0.72, 0.86, 0.9),
                        custom_size: Some(Vec2::splat(TILE_SIZE)),
                        ..Default::default()
                    }
                } else {
                    Sprite {
                        image: background_image,
                        custom_size: Some(Vec2::splat(TILE_SIZE)),
                        ..Default::default()
                    }
                },
                Transform::from_translation(world_pos),
                world_layer.clone(),
            ));
        }
    }

    for (pos, kind) in &level.tiles {
        let (color, image) = match kind {
            TileKind::GroundMain => (Color::srgb(0.32, 0.24, 0.16), assets.ground_main.clone()),
            TileKind::GroundTop => (Color::srgb(0.5, 0.4, 0.25), assets.ground_top.clone()),
        };
        commands.spawn((
            if use_placeholders {
                Sprite {
                    color,
                    custom_size: Some(Vec2::splat(TILE_SIZE)),
                    ..Default::default()
                }
            } else {
                Sprite {
                    image,
                    custom_size: Some(Vec2::splat(TILE_SIZE)),
                    ..Default::default()
                }
            },
            Transform::from_translation(grid_to_world(*pos) + Vec3::new(0.0, 0.0, 1.0)),
            world_layer.clone(),
        ));
    }

    for decoration in &level.decorations {
        let (color, image, z) = match decoration.kind {
            DecorationKind::Cloud => (Color::srgb(0.95, 0.98, 1.0), assets.decor_cloud.clone(), 0.6),
            DecorationKind::Plant => (Color::srgb(0.2, 0.6, 0.25), assets.decor_plant.clone(), 1.2),
        };
        commands.spawn((
            if use_placeholders {
                Sprite {
                    color,
                    custom_size: Some(Vec2::splat(TILE_SIZE)),
                    ..Default::default()
                }
            } else {
                Sprite {
                    image,
                    custom_size: Some(Vec2::splat(TILE_SIZE)),
                    ..Default::default()
                }
            },
            Transform::from_translation(grid_to_world(decoration.pos) + Vec3::new(0.0, 0.0, z)),
            world_layer.clone(),
        ));
    }

    commands.spawn((
        if use_placeholders {
            Sprite {
                color: Color::srgb(0.85, 0.2, 0.2),
                custom_size: Some(Vec2::splat(TILE_SIZE)),
                ..Default::default()
            }
        } else {
            Sprite {
                image: assets
                    .flag_frames
                    .first()
                    .cloned()
                    .unwrap_or_else(|| assets.ground_main.clone()),
                custom_size: Some(Vec2::splat(TILE_SIZE)),
                ..Default::default()
            }
        },
        Transform::from_translation(grid_to_world(level.flag) + Vec3::new(0.0, 0.0, 2.0)),
        world_layer.clone(),
        Flag,
        FlagAnim {
            timer: Timer::from_seconds(0.35, TimerMode::Repeating),
            index: 0,
        },
    ));

    commands.spawn((
        if use_placeholders {
            Sprite {
                color: Color::srgb(0.9, 0.75, 0.2),
                custom_size: Some(Vec2::splat(TILE_SIZE)),
                ..Default::default()
            }
        } else {
            Sprite {
                image: assets.hero.clone(),
                custom_size: Some(Vec2::splat(TILE_SIZE)),
                ..Default::default()
            }
        },
        Transform::from_translation(grid_to_world(level.hero_start) + Vec3::new(0.0, 0.0, 3.0)),
        world_layer,
        Hero {
            grid_pos: level.hero_start,
        },
    ));

    commands.insert_resource(level);
    commands.insert_resource(assets);
    commands.insert_resource(PlaceholderMode(use_placeholders));
}

#[allow(clippy::too_many_arguments)]
fn handle_run_requests(
    mut events: MessageReader<RunRequest>,
    mut python_task: ResMut<PythonTask>,
    mut editor: ResMut<EditorState>,
    level: Res<LevelMap>,
    mut command_queue: ResMut<CommandQueue>,
    mut phase: ResMut<GamePhase>,
    mut hero_query: Query<(Entity, &mut Hero, &mut Transform, Option<&Moving>)>,
    mut commands: Commands,
    run_state: ResMut<RunState>,
) {
    if python_task.running {
        events.clear();
        return;
    }

    if let Some(event) = events.read().last() {
        if run_state.has_run {
            reset_game_state(
                &level,
                &mut command_queue,
                &mut phase,
                &mut editor,
                &mut hero_query,
                &mut commands,
                true,
            );
        } else {
            reset_game_state(
                &level,
                &mut command_queue,
                &mut phase,
                &mut editor,
                &mut hero_query,
                &mut commands,
                false,
            );
        }
        let code = event.0.clone();
        let (tx, rx) = crossbeam_channel::unbounded();
        std::thread::spawn(move || {
            let result = python::run_code_via_worker(code, Duration::from_secs(1));
            let _ = tx.send(result);
        });
        python_task.receiver = Some(rx);
        python_task.running = true;
    }
}

fn auto_run_system(
    mut autorun: ResMut<AutoRun>,
    editor: Res<EditorState>,
    mut run_events: MessageWriter<RunRequest>,
) {
    if autorun.done {
        return;
    }
    if std::env::var("ANXO_AUTORUN").ok().as_deref() != Some("1") {
        return;
    }
    run_events.write(RunRequest(editor.code.clone()));
    autorun.done = true;
}

#[allow(clippy::too_many_arguments)]
fn poll_python_results(
    mut python_task: ResMut<PythonTask>,
    mut command_queue: ResMut<CommandQueue>,
    mut phase: ResMut<GamePhase>,
    mut editor: ResMut<EditorState>,
    _level: Res<LevelMap>,
    _hero_query: Query<(Entity, &mut Hero, &mut Transform, Option<&Moving>)>,
    _commands: Commands,
    mut run_state: ResMut<RunState>,
) {
    let Some(receiver) = python_task.receiver.as_ref() else {
        return;
    };

    match receiver.try_recv() {
        Ok(result) => {
            python_task.running = false;
            python_task.receiver = None;
            match result {
                Ok(parsed_commands) => {
                    if parsed_commands.iter().any(|command| {
                        matches!(command, Command::Move(Direction::Up | Direction::Down))
                    }) {
                        command_queue.commands.clear();
                        command_queue.index = 0;
                        editor.error = Some(
                            "Only move_left() and move_right() are allowed in level 1."
                                .to_string(),
                        );
                        *phase = GamePhase::Editing;
                        return;
                    }
                    command_queue.commands = parsed_commands;
                    command_queue.index = 0;
                    *phase = GamePhase::Playing;
                    run_state.has_run = true;
                }
                Err(error) => {
                    editor.error = Some(error);
                    *phase = GamePhase::Editing;
                }
            }
        }
        Err(TryRecvError::Empty) => {}
        Err(TryRecvError::Disconnected) => {
            python_task.running = false;
            python_task.receiver = None;
            editor.error = Some("Python worker disconnected".to_string());
        }
    }
}

fn initial_code() -> String {
    if let Ok(code) = std::env::var("ANXO_START_CODE") {
        return code;
    }
    "from game import hero\n".to_string()
}

#[allow(clippy::too_many_arguments)]
fn playback_system(
    time: Res<Time>,
    mut timer: ResMut<PlaybackTimer>,
    level: Res<LevelMap>,
    mut command_queue: ResMut<CommandQueue>,
    mut phase: ResMut<GamePhase>,
    mut editor: ResMut<EditorState>,
    mut hero_query: Query<HeroQueryData>,
    mut commands: Commands,
) {
    if *phase != GamePhase::Playing {
        return;
    }
    timer.0.tick(time.delta());
    if !timer.0.is_finished() {
        return;
    }

    let Ok((hero_entity, hero, transform, moving, reset_anim)) = hero_query.single_mut() else {
        return;
    };

    if moving.is_some() {
        return;
    }
    if reset_anim.is_some() {
        return;
    }

    let Some(command) = command_queue.commands.get(command_queue.index) else {
        return;
    };

    let target = match command {
        Command::Move(Direction::Left) => find_horizontal_target(&level, hero.grid_pos, -1),
        Command::Move(Direction::Right) => find_horizontal_target(&level, hero.grid_pos, 1),
        Command::Move(Direction::Up | Direction::Down) => {
            editor.error =
                Some("Only move_left() and move_right() are allowed in level 1.".to_string());
            *phase = GamePhase::Editing;
            return;
        }
    };

    let Some(target) = target else {
        editor.error = Some("You can't move there.".to_string());
        return;
    };

    let start = transform.translation;
    let end = grid_to_world(target) + Vec3::new(0.0, 0.0, 3.0);
    commands.entity(hero_entity).insert(Moving {
        start,
        end,
        target_grid: target,
        timer: Timer::from_seconds(0.15, TimerMode::Once),
    });

    command_queue.index += 1;
}

fn movement_system(
    time: Res<Time>,
    mut commands: Commands,
    mut hero_query: Query<(Entity, &mut Hero, &mut Transform, &mut Moving)>,
) {
    let Ok((entity, mut hero, mut transform, mut moving)) = hero_query.single_mut() else {
        return;
    };

    moving.timer.tick(time.delta());
    let duration = moving.timer.duration().as_secs_f32().max(f32::EPSILON);
    let t = (moving.timer.elapsed_secs() / duration).clamp(0.0, 1.0);
    transform.translation = moving.start.lerp(moving.end, t);

    if moving.timer.is_finished() {
        hero.grid_pos = moving.target_grid;
        commands.entity(entity).remove::<Moving>();
    }
}

fn win_system(
    mut hero_query: Query<(Entity, &Hero, Option<&Moving>, Option<&WinAnim>)>,
    level: Res<LevelMap>,
    mut phase: ResMut<GamePhase>,
    mut commands: Commands,
) {
    if *phase != GamePhase::Playing {
        return;
    }
    let Ok((entity, hero, moving, win_anim)) = hero_query.single() else {
        return;
    };

    if moving.is_none() && hero.grid_pos == level.flag {
        *phase = GamePhase::Won;
        if win_anim.is_none() {
            commands.entity(entity).insert(WinAnim {
                total: Timer::from_seconds(0.6, TimerMode::Once),
                frame: Timer::from_seconds(0.08, TimerMode::Repeating),
                index: 0,
                base_pos: grid_to_world(hero.grid_pos) + Vec3::new(0.0, 0.0, 3.0),
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn reset_system(
    mut events: MessageReader<ResetRequest>,
    level: Res<LevelMap>,
    mut command_queue: ResMut<CommandQueue>,
    mut phase: ResMut<GamePhase>,
    mut editor: ResMut<EditorState>,
    mut hero_query: Query<(Entity, &mut Hero, &mut Transform, Option<&Moving>)>,
    mut commands: Commands,
    mut run_state: ResMut<RunState>,
) {
    if events.is_empty() {
        return;
    }
    events.clear();

    reset_game_state(
        &level,
        &mut command_queue,
        &mut phase,
        &mut editor,
        &mut hero_query,
        &mut commands,
        true,
    );
    run_state.has_run = false;
}

fn reset_game_state(
    level: &LevelMap,
    command_queue: &mut CommandQueue,
    phase: &mut GamePhase,
    editor: &mut EditorState,
    hero_query: &mut Query<(Entity, &mut Hero, &mut Transform, Option<&Moving>)>,
    commands: &mut Commands,
    animate: bool,
) {
    command_queue.commands.clear();
    command_queue.index = 0;
    editor.error = None;
    *phase = GamePhase::Editing;

    if let Ok((entity, mut hero, mut transform, _)) = hero_query.single_mut() {
        hero.grid_pos = level.hero_start;
        transform.translation = grid_to_world(level.hero_start) + Vec3::new(0.0, 0.0, 3.0);
        commands.entity(entity).remove::<Moving>();
        if animate {
            trigger_reset_animation(level, hero_query, commands);
        }
    }
}

fn trigger_reset_animation(
    level: &LevelMap,
    hero_query: &mut Query<(Entity, &mut Hero, &mut Transform, Option<&Moving>)>,
    commands: &mut Commands,
) {
    if let Ok((entity, mut hero, mut transform, _)) = hero_query.single_mut() {
        hero.grid_pos = level.hero_start;
        transform.translation = grid_to_world(level.hero_start) + Vec3::new(0.0, 0.0, 3.0);
        commands.entity(entity).remove::<Moving>();
        commands.entity(entity).insert(ResetAnim {
            total: Timer::from_seconds(0.8, TimerMode::Once),
            frame: Timer::from_seconds(0.08, TimerMode::Repeating),
            index: 0,
            base_pos: transform.translation,
        });
    }
}

fn reset_animation_system(
    time: Res<Time>,
    assets: Res<LevelAssets>,
    placeholder: Res<PlaceholderMode>,
    mut commands: Commands,
    mut hero_query: Query<(Entity, &mut Sprite, &mut Transform, &mut ResetAnim)>,
) {
    for (entity, mut sprite, mut transform, mut anim) in &mut hero_query {
        anim.total.tick(time.delta());
        anim.frame.tick(time.delta());
        let t = anim.total.elapsed_secs() / anim.total.duration().as_secs_f32().max(f32::EPSILON);
        let jump = (t * std::f32::consts::PI).sin() * 16.0;
        transform.translation = anim.base_pos + Vec3::new(0.0, jump, 0.0);
        if anim.frame.just_finished() {
            anim.index = anim.index.saturating_add(1);
            if placeholder.0 {
                let pulse = if anim.index % 2 == 0 { 1.0 } else { 0.5 };
                sprite.color = Color::srgb(pulse, pulse * 0.75, 0.15);
            } else if !assets.hero_frames.is_empty() {
                let frame_index = anim.index % assets.hero_frames.len();
                sprite.image = assets.hero_frames[frame_index].clone();
                sprite.custom_size = Some(Vec2::splat(TILE_SIZE));
            }
        }

        if anim.total.is_finished() {
            if !placeholder.0 {
                sprite.image = assets.hero.clone();
                sprite.custom_size = Some(Vec2::splat(TILE_SIZE));
            }
            transform.translation = anim.base_pos;
            commands.entity(entity).remove::<ResetAnim>();
        }
    }
}

fn win_animation_system(
    time: Res<Time>,
    assets: Res<LevelAssets>,
    placeholder: Res<PlaceholderMode>,
    mut commands: Commands,
    mut hero_query: Query<(Entity, &mut Sprite, &mut Transform, &mut WinAnim)>,
) {
    for (entity, mut sprite, mut transform, mut anim) in &mut hero_query {
        anim.total.tick(time.delta());
        anim.frame.tick(time.delta());
        let t = anim.total.elapsed_secs() / anim.total.duration().as_secs_f32().max(f32::EPSILON);
        let hop = (t * std::f32::consts::TAU).sin() * 10.0;
        transform.translation = anim.base_pos + Vec3::new(0.0, hop, 0.0);
        if anim.frame.just_finished() {
            anim.index = anim.index.saturating_add(1);
            if placeholder.0 {
                let pulse = if anim.index % 2 == 0 { 0.9 } else { 0.6 };
                sprite.color = Color::srgb(pulse, pulse, 0.2);
            } else if !assets.hero_frames.is_empty() {
                let frame_index = anim.index % assets.hero_frames.len();
                sprite.image = assets.hero_frames[frame_index].clone();
                sprite.custom_size = Some(Vec2::splat(TILE_SIZE));
            }
        }

        if anim.total.is_finished() {
            if !placeholder.0 {
                sprite.image = assets.hero.clone();
                sprite.custom_size = Some(Vec2::splat(TILE_SIZE));
            }
            transform.translation = anim.base_pos;
            commands.entity(entity).remove::<WinAnim>();
        }
    }
}

fn flag_animation_system(
    time: Res<Time>,
    assets: Res<LevelAssets>,
    placeholder: Res<PlaceholderMode>,
    mut flag_query: Query<(&mut Sprite, &mut FlagAnim)>,
) {
    for (mut sprite, mut anim) in &mut flag_query {
        anim.timer.tick(time.delta());
        if !anim.timer.just_finished() {
            continue;
        }
        anim.index = anim.index.saturating_add(1);
        if placeholder.0 {
            let pulse = if anim.index % 2 == 0 { 0.9 } else { 0.6 };
            sprite.color = Color::srgb(0.9, pulse, pulse);
        } else if !assets.flag_frames.is_empty() {
            let frame_index = anim.index % assets.flag_frames.len();
            sprite.image = assets.flag_frames[frame_index].clone();
            sprite.custom_size = Some(Vec2::splat(TILE_SIZE));
        }
    }
}

fn find_horizontal_target(level: &LevelMap, current: IVec2, dx: i32) -> Option<IVec2> {
    let candidates = [0, 1, -1];
    for dy in candidates {
        let target = IVec2::new(current.x + dx, current.y + dy);
        if !in_bounds(level, target) {
            continue;
        }
        if level.is_wall(target) {
            continue;
        }
        let below = IVec2::new(target.x, target.y - 1);
        if !in_bounds(level, below) {
            continue;
        }
        if !level.is_wall(below) {
            continue;
        }
        return Some(target);
    }
    None
}

fn in_bounds(level: &LevelMap, pos: IVec2) -> bool {
    pos.x >= 0 && pos.y >= 0 && pos.x < level.width && pos.y < level.height
}

fn update_camera_viewport(
    windows: Query<&Window>,
    layout: Res<UiLayout>,
    level: Res<LevelMap>,
    mut camera_query: Query<(&mut Camera, &mut Projection, &mut Transform), With<WorldCamera>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let Ok((mut camera, mut projection, mut transform)) = camera_query.single_mut() else {
        return;
    };

    let scale = layout.pixels_per_point.max(0.1);
    let editor_px = (layout.editor_width * scale).round() as u32;
    let window_size = window.physical_size();
    let viewport_width = window_size.x.saturating_sub(editor_px);

    if viewport_width == 0 || window_size.y == 0 {
        camera.viewport = None;
        return;
    }
    camera.viewport = Some(Viewport {
        physical_position: UVec2::new(0, 0),
        physical_size: UVec2::new(viewport_width, window_size.y),
        ..Default::default()
    });

    let Projection::Orthographic(ref mut ortho) = *projection else {
        return;
    };
    let viewport_aspect = viewport_width as f32 / window_size.y as f32;
    let level_w = level.width as f32 * TILE_SIZE;
    let level_h = level.height as f32 * TILE_SIZE;
    let level_aspect = level_w / level_h.max(f32::EPSILON);
    let viewport_height = if viewport_aspect > level_aspect {
        level_w / viewport_aspect
    } else {
        level_h
    };
    ortho.scaling_mode = ScalingMode::FixedVertical { viewport_height };
    ortho.scale = 1.0;

    transform.translation = Vec3::new(
        (level.width as f32 - 1.0) * TILE_SIZE * 0.5,
        (level.height as f32 - 1.0) * TILE_SIZE * 0.5,
        999.0,
    );
}
