//! The camera: a fixed dimetric view, as the original had.
//!
//! # Why the angle is not negotiable
//!
//! The original used a 2:1 dimetric projection — one cell is twice as wide on
//! screen as it is tall. That ratio is what the whole visual language depends
//! on: unit silhouettes, building footprints and terrain tiles were all
//! designed to read at that angle.
//!
//! It corresponds to an elevation of `atan(1/2)`, about 26.57°, viewed at 45°
//! of yaw, under an **orthographic** projection. Perspective would make cells
//! at the top of the screen smaller than cells at the bottom, which breaks the
//! constant-size grid the player reads positions from.
//!
//! Rotation is deliberately not offered. Flat, angle-specific art stops reading
//! correctly the moment the camera turns, and the whole art direction assumes a
//! single viewpoint. See `docs/adr/0002-realtime-3d-under-a-budget.md`.

use bevy::prelude::*;

/// Elevation above the ground plane, in degrees: `atan(1/2)`, the 2:1 dimetric
/// angle.
pub const CAMERA_PITCH_DEGREES: f32 = 26.565_05;

/// Rotation about the vertical axis, in degrees.
pub const CAMERA_YAW_DEGREES: f32 = 45.0;

/// How many cells tall the viewport is at default zoom.
///
/// Chosen to show roughly as much ground as the original did at its native
/// resolution — close enough that build spacing and engagement ranges feel the
/// same.
pub const DEFAULT_VIEW_HEIGHT_CELLS: f32 = 24.0;

/// Zoom limits, as multipliers of the default view height.
///
/// A narrow range on purpose: models and UI are authored for this scale, and a
/// wide zoom would expose how little detail the low-poly art carries up close
/// while making units unreadably small far out.
pub const MIN_ZOOM: f32 = 0.75;
pub const MAX_ZOOM: f32 = 1.5;

/// Cells per second the camera pans at default zoom.
const PAN_SPEED: f32 = 18.0;

/// How close to the window edge the pointer must be to pan, in pixels.
const EDGE_PAN_MARGIN: f32 = 12.0;

/// Camera state that is not derivable from the transform.
#[derive(Resource)]
pub struct CameraRig {
    /// The ground point the camera is centred on.
    pub focus: Vec2,
    /// Multiplier on [`DEFAULT_VIEW_HEIGHT_CELLS`].
    pub zoom: f32,
    /// Bounds of the map, so panning cannot wander off into empty space.
    pub bounds: Vec2,
    /// Whether the pointer may pan by touching the window edge. Off while a
    /// selection box is being dragged, or the camera would run away.
    pub edge_pan_enabled: bool,
}

impl CameraRig {
    pub fn new(map_width: f32, map_height: f32) -> CameraRig {
        CameraRig {
            focus: Vec2::new(map_width / 2.0, map_height / 2.0),
            zoom: 1.0,
            bounds: Vec2::new(map_width, map_height),
            edge_pan_enabled: true,
        }
    }

    /// Half-height of the visible area, in cells.
    pub fn half_height(&self) -> f32 {
        DEFAULT_VIEW_HEIGHT_CELLS * self.zoom / 2.0
    }
}

/// Marks the game camera.
#[derive(Component)]
pub struct GameCamera;

/// The camera's position in world space for a given focus point.
///
/// The distance back along the view direction is arbitrary under an
/// orthographic projection — it changes nothing about what is drawn, only what
/// falls inside the near and far planes.
fn camera_transform(focus: Vec2) -> Transform {
    let pitch = CAMERA_PITCH_DEGREES.to_radians();
    let yaw = CAMERA_YAW_DEGREES.to_radians();

    // Simulation (x, y) maps to world (x, z); world y is height.
    let target = Vec3::new(focus.x, 0.0, focus.y);
    let distance = 200.0;
    let offset = Vec3::new(
        yaw.sin() * pitch.cos(),
        pitch.sin(),
        yaw.cos() * pitch.cos(),
    ) * distance;

    Transform::from_translation(target + offset).looking_at(target, Vec3::Y)
}

pub fn spawn_camera(commands: &mut Commands, rig: &CameraRig) {
    commands.spawn((
        Camera3d::default(),
        // No tonemapping. The default curve needs lookup textures we do not
        // ship, and a failed tonemapping pass takes the whole 3D pass with it —
        // every surface comes out the fallback magenta.
        //
        // Turning it off is also the correct choice on its own terms: tone
        // mapping exists to compress high-dynamic-range lighting into display
        // range, and this scene is deliberately flat and low-dynamic-range.
        // Passing colours through unchanged means what the art defines is what
        // reaches the screen. See docs/04-rendering.md.
        bevy::core_pipeline::tonemapping::Tonemapping::None,
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                viewport_height: DEFAULT_VIEW_HEIGHT_CELLS * rig.zoom,
            },
            // A generous depth range: the camera sits far back so that nothing
            // on a large map clips, and orthographic depth precision does not
            // suffer the way perspective does.
            near: -500.0,
            far: 1000.0,
            ..OrthographicProjection::default_3d()
        }),
        camera_transform(rig.focus),
        GameCamera,
    ));
}

/// Keyboard, edge and wheel camera control.
pub fn update_camera(
    keys: Res<ButtonInput<KeyCode>>,
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
    windows: Query<&Window>,
    time: Res<Time>,
    mut rig: ResMut<CameraRig>,
    mut camera: Query<(&mut Transform, &mut Projection), With<GameCamera>>,
) {
    let mut pan = Vec2::ZERO;

    if keys.any_pressed([KeyCode::ArrowUp, KeyCode::KeyW]) {
        pan.y -= 1.0;
    }
    if keys.any_pressed([KeyCode::ArrowDown, KeyCode::KeyS]) {
        pan.y += 1.0;
    }
    if keys.any_pressed([KeyCode::ArrowLeft, KeyCode::KeyA]) {
        pan.x -= 1.0;
    }
    if keys.any_pressed([KeyCode::ArrowRight, KeyCode::KeyD]) {
        pan.x += 1.0;
    }

    // Edge panning, as the original had. Only when the window is focused, or
    // the camera would drift while the player is in another application.
    if rig.edge_pan_enabled
        && pan == Vec2::ZERO
        && let Ok(window) = windows.single()
        && window.focused
        && let Some(cursor) = window.cursor_position()
    {
        let size = Vec2::new(window.width(), window.height());
        if cursor.x < EDGE_PAN_MARGIN {
            pan.x -= 1.0;
        } else if cursor.x > size.x - EDGE_PAN_MARGIN {
            pan.x += 1.0;
        }
        if cursor.y < EDGE_PAN_MARGIN {
            pan.y -= 1.0;
        } else if cursor.y > size.y - EDGE_PAN_MARGIN {
            pan.y += 1.0;
        }
    }

    if pan != Vec2::ZERO {
        // Screen-space pan converted to world space. The camera is yawed 45°,
        // so "right on screen" is not "+x in the world"; without this rotation
        // the controls would feel diagonal.
        let yaw = CAMERA_YAW_DEGREES.to_radians();
        let (sin, cos) = yaw.sin_cos();
        let dir = pan.normalize();
        let world = Vec2::new(dir.x * cos - dir.y * sin, dir.x * sin + dir.y * cos);
        let step = world * PAN_SPEED * rig.zoom * time.delta_secs();
        rig.focus += step;
    }

    let mut zoom_delta = 0.0;
    for event in wheel.read() {
        zoom_delta -= event.y * 0.1;
    }
    if zoom_delta != 0.0 {
        rig.zoom = (rig.zoom + zoom_delta).clamp(MIN_ZOOM, MAX_ZOOM);
    }
    if keys.just_pressed(KeyCode::Home) {
        rig.zoom = 1.0;
    }

    // Keep the focus over the map. Half a viewport of overhang is allowed so
    // the map edge can be centred rather than pinned to the screen edge.
    let margin = rig.half_height();
    let bounds = rig.bounds;
    rig.focus.x = rig.focus.x.clamp(-margin, bounds.x + margin);
    rig.focus.y = rig.focus.y.clamp(-margin, bounds.y + margin);

    if let Ok((mut transform, mut projection)) = camera.single_mut() {
        *transform = camera_transform(rig.focus);
        if let Projection::Orthographic(ortho) = &mut *projection {
            ortho.scaling_mode = bevy::camera::ScalingMode::FixedVertical {
                viewport_height: DEFAULT_VIEW_HEIGHT_CELLS * rig.zoom,
            };
        }
    }
}

/// Converts a screen position to a point on the ground plane.
///
/// Returns `None` when the ray does not meet the ground, which under this fixed
/// camera only happens if the projection is misconfigured — but the caller must
/// still handle it rather than assume a hit.
pub fn screen_to_ground(
    camera: &Camera,
    transform: &GlobalTransform,
    screen: Vec2,
) -> Option<Vec2> {
    let ray = camera.viewport_to_world(transform, screen).ok()?;
    // Intersect with the plane y = 0.
    let denominator = ray.direction.y;
    if denominator.abs() < f32::EPSILON {
        return None;
    }
    let distance = -ray.origin.y / denominator;
    if distance < 0.0 {
        return None;
    }
    let hit = ray.origin + ray.direction * distance;
    Some(Vec2::new(hit.x, hit.z))
}
