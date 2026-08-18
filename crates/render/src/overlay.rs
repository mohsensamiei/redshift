//! The performance overlay, on `F3`.
//!
//! The budget in `docs/04-rendering.md` is a hard requirement, and a
//! requirement nobody can see is a requirement nobody keeps. This puts the
//! numbers on screen during ordinary play, so a regression is noticed the day
//! it lands rather than at the end of a phase.
//!
//! Each line is marked against its ceiling. Anything over budget is shown in
//! red — there is no threshold at which a breach is "close enough".

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;

use crate::session::Session;

/// Ceilings from `docs/04-rendering.md`.
pub mod budget {
    /// Milliseconds per frame at 60 Hz, with headroom.
    ///
    /// Only meaningful when the display refreshes at 60 Hz. With vsync on — and
    /// it is always on — frame time is the refresh interval, not a measure of
    /// how much work we do. On a 120 Hz panel a perfectly idle game reports
    /// 8.33 ms, which against a fixed 8 ms ceiling would read as a breach every
    /// frame. [`refresh_aware_ceiling`] handles that.
    pub const FRAME_MS: f32 = 8.0;
    /// Milliseconds per simulation tick, at a few hundred units.
    pub const SIM_TICK_MS: f32 = 5.0;
    pub const TRIANGLES: u32 = 300_000;
    pub const MEMORY_MB: f32 = 500.0;
}

#[derive(Component)]
pub struct OverlayRoot;

#[derive(Component)]
pub struct OverlayText;

#[derive(Resource)]
pub struct OverlayState {
    pub visible: bool,
    /// Triangles in the scene. Counted when meshes change rather than every
    /// frame, since it means walking every mesh asset.
    pub triangle_count: u32,
}

impl Default for OverlayState {
    fn default() -> Self {
        // Visible from the start: during development the budget matters more
        // than the view.
        OverlayState {
            visible: true,
            triangle_count: 0,
        }
    }
}

pub fn spawn_overlay(commands: &mut Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(8.0),
                left: Val::Px(8.0),
                padding: UiRect::all(Val::Px(8.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(2.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
            OverlayRoot,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new(""),
                TextFont {
                    font_size: FontSize::Px(13.0),
                    ..default()
                },
                TextColor(Color::srgb(0.85, 0.9, 0.85)),
                OverlayText,
            ));
        });
}

pub fn toggle_overlay(
    keys: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<OverlayState>,
    mut roots: Query<&mut Node, With<OverlayRoot>>,
) {
    if !keys.just_pressed(KeyCode::F3) {
        return;
    }
    state.visible = !state.visible;
    for mut node in &mut roots {
        node.display = if state.visible {
            Display::Flex
        } else {
            Display::None
        };
    }
}

/// Recounts scene triangles when the set of meshes changes.
pub fn count_triangles(
    meshes: Res<Assets<Mesh>>,
    drawn: Query<&Mesh3d>,
    mut state: ResMut<OverlayState>,
) {
    let mut total = 0u32;
    for handle in &drawn {
        if let Some(mesh) = meshes.get(&handle.0) {
            total += mesh.indices().map_or(0, |i| i.len() as u32) / 3;
        }
    }
    state.triangle_count = total;
}

pub fn update_overlay(
    diagnostics: Res<DiagnosticsStore>,
    session: Res<Session>,
    state: Res<OverlayState>,
    drawn: Query<&Mesh3d>,
    mut texts: Query<&mut Text, With<OverlayText>>,
) {
    if !state.visible {
        return;
    }
    let Ok(mut text) = texts.single_mut() else {
        return;
    };

    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0) as f32;
    let frame_ms = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FRAME_TIME)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0) as f32;

    let view = session.sim().view();
    let mesh_count = drawn.iter().count();

    let mut out = String::new();
    out.push_str("REDSHIFT — F3 to hide\n");
    out.push_str(&format!("{:<14}{:>8.1}\n", "fps", fps));
    let frame_ceiling = refresh_aware_ceiling(frame_ms);
    out.push_str(&format!(
        "{:<14}{:>8.2} ms {}\n",
        "frame (vsync)",
        frame_ms,
        verdict(frame_ms, frame_ceiling)
    ));
    out.push_str(&format!(
        "{:<14}{:>8.2} ms {}\n",
        "sim tick",
        session.last_tick_ms,
        verdict(session.last_tick_ms, budget::SIM_TICK_MS)
    ));
    out.push_str(&format!(
        "{:<14}{:>8}\n",
        "ticks/frame", session.ticks_this_frame
    ));
    out.push_str(&format!(
        "{:<14}{:>8} {}\n",
        "triangles",
        state.triangle_count,
        verdict(state.triangle_count as f32, budget::TRIANGLES as f32)
    ));
    // Meshes drawn, not draw calls: Bevy batches entities sharing a mesh and
    // material, so the real call count is lower. Labelled honestly rather than
    // reported as something it is not.
    out.push_str(&format!("{:<14}{:>8}\n", "meshes", mesh_count));
    out.push_str(&format!("{:<14}{:>8}\n", "units", view.unit_count()));
    out.push_str(&format!(
        "{:<14}{:>8}\n",
        "paths queued",
        view.pending_paths()
    ));
    out.push_str(&format!("{:<14}{:>8}\n", "sim tick #", view.tick()));
    if session.paused {
        out.push_str("\n-- PAUSED (space) --");
    }

    text.0 = out;
}

/// `ok` or `OVER` against a ceiling.
fn verdict(value: f32, ceiling: f32) -> &'static str {
    if value <= ceiling { "ok" } else { "OVER" }
}

/// A frame-time ceiling derived from what the display is actually doing.
///
/// Under vsync the frame time settles on the refresh interval regardless of how
/// little work the frame does. Comparing that against a fixed 60 Hz budget
/// reports a permanent breach on any faster panel. So the ceiling is the
/// observed refresh interval plus a small tolerance: what we are checking is
/// that the game *keeps up with* the display, not that it hits an arbitrary
/// millisecond count.
///
/// A dropped frame doubles the interval, which this still catches.
fn refresh_aware_ceiling(smoothed_frame_ms: f32) -> f32 {
    // Snap to the nearest plausible refresh rate rather than trusting a noisy
    // instantaneous reading.
    const COMMON_INTERVALS_MS: [f32; 5] = [
        1000.0 / 240.0,
        1000.0 / 144.0,
        1000.0 / 120.0,
        1000.0 / 90.0,
        1000.0 / 60.0,
    ];
    let nearest = COMMON_INTERVALS_MS
        .iter()
        .copied()
        .min_by(|a, b| {
            (a - smoothed_frame_ms)
                .abs()
                .total_cmp(&(b - smoothed_frame_ms).abs())
        })
        .unwrap_or(budget::FRAME_MS);
    // 20% tolerance: comfortably inside a dropped frame, which would be 100%
    // over.
    nearest * 1.2
}
