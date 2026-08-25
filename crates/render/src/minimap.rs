//! The minimap.
//!
//! A texture, rebuilt from the simulation a few times a second and drawn as one
//! image. Not a second camera: rendering the world twice would double the draw
//! calls and the triangle count for a picture two centimetres across, and the
//! budget in docs/04-rendering.md is a test rather than a wish.
//!
//! It shows exactly what the player is allowed to know — the same fog the
//! terrain mesh uses. A minimap that quietly revealed the whole map would be
//! the single most effective cheat in the game, and it is the sort of thing
//! that gets added for debugging and then forgotten.

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use redshift_sim::map::Cell;
use redshift_sim::{PlayerId, Sight};

use crate::session::Session;
use crate::world::TEAM_COLOURS;

/// Side of the minimap, in logical pixels.
const SIZE: f32 = 172.0;

/// Ticks between redraws.
///
/// The map does not change fast enough to be worth a rebuild every frame, and
/// a full-map scan sixty times a second is real work for a picture nobody is
/// staring at.
const REDRAW_EVERY: u32 = 6;

#[derive(Component)]
pub struct Minimap;

#[derive(Resource)]
pub struct MinimapState {
    pub image: Handle<Image>,
    pub width: u32,
    pub height: u32,
    last_drawn: u32,
}

pub fn build_minimap(images: &mut Assets<Image>, width: u32, height: u32) -> MinimapState {
    let image = Image::new_fill(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    MinimapState {
        image: images.add(image),
        width,
        height,
        last_drawn: u32::MAX,
    }
}

pub fn spawn_minimap(commands: &mut Commands, state: &MinimapState) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(6.0),
            right: Val::Px(6.0),
            width: Val::Px(SIZE),
            height: Val::Px(SIZE),
            ..default()
        },
        ImageNode::new(state.image.clone()),
        Minimap,
    ));
}

/// Repaints the texture from the world.
pub fn refresh_minimap(
    session: Res<Session>,
    mut state: ResMut<MinimapState>,
    mut images: ResMut<Assets<Image>>,
) {
    let tick = session.sim().tick_number();
    if state.last_drawn != u32::MAX && tick.saturating_sub(state.last_drawn) < REDRAW_EVERY {
        return;
    }
    state.last_drawn = tick;

    let handle = state.image.clone();
    let Some(mut image) = images.get_mut(&handle) else {
        return;
    };
    let Some(data) = image.data.as_mut() else {
        return;
    };

    let sim = session.sim();
    let viewer = session.local_player();
    let map = sim.map();
    let (w, h) = (state.width, state.height);

    for y in 0..h {
        for x in 0..w {
            // The texture is the map's own size, so this is one cell per pixel
            // and there is no sampling decision to get wrong.
            let cell = Cell::new(x as i32, y as i32);
            let colour = match sim.visibility().sight(viewer, cell) {
                Sight::Unseen => [6, 6, 8],
                sight => {
                    let base = if map.is_bridged(cell) {
                        [118, 92, 62]
                    } else {
                        match map.terrain(cell) {
                            redshift_sim::Terrain::Water => [28, 71, 117],
                            redshift_sim::Terrain::Rock => [87, 82, 77],
                            redshift_sim::Terrain::Ground if map.has_ore(cell) => [198, 153, 36],
                            redshift_sim::Terrain::Ground => [82, 102, 56],
                        }
                    };
                    if sight == Sight::Fogged {
                        [base[0] / 2, base[1] / 2, base[2] / 2]
                    } else {
                        base
                    }
                }
            };
            write_pixel(data, w, x, y, colour);
        }
    }

    // Units on top, and only the ones the player can see — the same rule the
    // rest of the interface obeys. A minimap that showed everything would be
    // the most effective cheat in the game.
    for (_, unit) in sim.view().units() {
        if !unit.is_alive() || unit.is_aboard() || !sim.can_see(viewer, unit) {
            continue;
        }
        let cell = unit.cell();
        if cell.x < 0 || cell.y < 0 || cell.x as u32 >= w || cell.y as u32 >= h {
            continue;
        }
        let (r, g, b) = team_colour(unit.owner);
        // A blob rather than a pixel: one pixel per unit is invisible at this
        // size, and an army has to read as an army.
        let stats = sim.stats().get(unit.owner, unit.kind);
        let reach = if stats.mobile { 0 } else { 1 };
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let (px, py) = (cell.x + dx, cell.y + dy);
                if px >= 0 && py >= 0 && (px as u32) < w && (py as u32) < h {
                    write_pixel(data, w, px as u32, py as u32, [r, g, b]);
                }
            }
        }
    }
}

fn write_pixel(data: &mut [u8], width: u32, x: u32, y: u32, colour: [u8; 3]) {
    let i = ((y * width + x) * 4) as usize;
    if let Some(slot) = data.get_mut(i..i + 4) {
        slot[0] = colour[0];
        slot[1] = colour[1];
        slot[2] = colour[2];
        slot[3] = 255;
    }
}

fn team_colour(player: PlayerId) -> (u8, u8, u8) {
    if player.is_neutral() {
        return (170, 170, 160);
    }
    TEAM_COLOURS[player.0 as usize % TEAM_COLOURS.len()]
}
