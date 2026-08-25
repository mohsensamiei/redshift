//! The right-hand panel: what you can build, and what you are building.
//!
//! This replaces four function keys that queued four hard-coded structures. It
//! is not a decoration on top of that — it is what makes the build list come
//! from the *rules* rather than from a constant in the renderer, which is the
//! difference between adding a unit by editing a RON file and adding one by
//! editing Rust.
//!
//! It also gives the last two commands somewhere to live. Selling a building
//! and cancelling a queued item are the two things a player does that have no
//! sensible keystroke and no sensible right-click: they are buttons, they were
//! always going to be buttons, and until there was a panel they simply could
//! not be issued at all.
//!
//! Art is Phase 4. Everything here is a coloured rectangle with a label.

use bevy::prelude::*;
use bevy::text::FontSize;
use redshift_sim::EntityKind;
use redshift_sim::command::CommandKind;
use redshift_sim::{EntityId, PlayerId};

use crate::session::Session;

/// Width of the panel, in logical pixels.
const PANEL_WIDTH: f32 = 184.0;
const ROW_HEIGHT: f32 = 26.0;

/// Marks the panel root.
#[derive(Component)]
pub struct Sidebar;

/// One buildable thing's row.
#[derive(Component, Clone, Copy)]
pub struct BuildRow {
    pub kind: EntityKind,
}

/// One item in the queue, so right-clicking it can cancel it.
#[derive(Component, Clone, Copy)]
pub struct QueueRow {
    pub building: EntityId,
    pub index: u8,
}

/// The line describing what is selected.
#[derive(Component)]
pub struct SelectionInfo;

/// A charged superweapon, ready to be aimed.
#[derive(Component, Clone, Copy)]
pub struct PowerRow {
    pub building: EntityId,
}

/// Which superweapon is waiting for a target, if any.
///
/// A mode, like selling, and for the same reason: firing one is irreversible
/// and worth two deliberate acts rather than one. It also has to be a mode
/// because a superweapon is aimed at *ground*, and there is nothing else in the
/// interface that takes a place as an argument.
#[derive(Resource, Default)]
pub struct AimingPower {
    pub building: Option<EntityId>,
}

/// The sell toggle.
#[derive(Component)]
pub struct SellToggle;

/// Where the rows go, so they can be rebuilt without touching the chrome.
#[derive(Component)]
pub struct BuildList;

#[derive(Component)]
pub struct QueueList;

#[derive(Component)]
pub struct PowerList;

#[derive(Component)]
pub struct CreditsLabel;

/// Whether the next left-click on one of your buildings sells it.
///
/// A mode rather than a modifier key, as the original had it. Selling is
/// irreversible and worth making deliberate — a player should have to arm it,
/// see that it is armed, and then choose a building.
#[derive(Resource, Default)]
pub struct SellMode {
    pub armed: bool,
}

pub fn spawn_sidebar(commands: &mut Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                right: Val::Px(0.0),
                bottom: Val::Px(0.0),
                width: Val::Px(PANEL_WIDTH),
                padding: UiRect::all(Val::Px(6.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(4.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.06, 0.07, 0.09, 0.88)),
            Sidebar,
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new(""),
                TextFont {
                    font_size: FontSize::Px(12.0),
                    ..default()
                },
                TextColor(Color::srgb(0.78, 0.82, 0.86)),
                SelectionInfo,
            ));
            panel.spawn((
                Text::new("0"),
                TextFont {
                    font_size: FontSize::Px(15.0),
                    ..default()
                },
                TextColor(Color::srgb(0.92, 0.86, 0.42)),
                CreditsLabel,
            ));

            panel.spawn((
                Button,
                Node {
                    height: Val::Px(ROW_HEIGHT),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(Color::srgb(0.22, 0.10, 0.10)),
                SellToggle,
                children![(
                    Text::new("SELL"),
                    TextFont {
                        font_size: FontSize::Px(13.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.9, 0.7, 0.7)),
                )],
            ));

            // What is being built, above what could be built: a player checks
            // progress far more often than they start something new.
            // Above everything: a charged superweapon is the most important
            // thing on the panel by a distance.
            panel.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(2.0),
                    ..default()
                },
                PowerList,
            ));
            panel.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(2.0),
                    ..default()
                },
                QueueList,
            ));
            panel.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(2.0),
                    ..default()
                },
                BuildList,
            ));
        });
}

/// Everything the local player could start building right now.
///
/// Read from the rules every time rather than cached. It changes whenever a
/// building is finished, captured, sold or destroyed, and a cache would be one
/// missed invalidation away from offering something the player cannot have.
fn buildable_now(session: &Session) -> Vec<(EntityKind, String, u32)> {
    let sim = session.sim();
    let player = session.local_player();
    let mut out = Vec::new();
    for (kind, def) in sim.rules().entities() {
        let stats = sim.stats().get(player, kind);
        if stats.cost == 0 {
            continue;
        }
        if sim.producer_for(player, kind).is_none() {
            continue;
        }
        if !sim.prerequisites_met(player, kind) || !sim.within_build_limit(player, kind) {
            continue;
        }
        out.push((kind, def.id.clone(), stats.cost));
    }
    out
}

/// Rebuilds the two lists when what they should say has changed.
///
/// Compared against what is on screen rather than rebuilt every frame: a panel
/// respawned sixty times a second would allocate more than the rest of the
/// renderer put together, and would eat every click as the node under the
/// cursor vanished between press and release.
pub fn refresh_sidebar(
    mut commands: Commands,
    session: Res<Session>,
    build_list: Query<(Entity, Option<&Children>), With<BuildList>>,
    queue_list: Query<(Entity, Option<&Children>), With<QueueList>>,
    rows: Query<&BuildRow>,
    queued: Query<&QueueRow>,
    mut credits: Query<&mut Text, With<CreditsLabel>>,
) {
    let player = session.local_player();

    if let Ok(mut text) = credits.single_mut() {
        let line = format!("${}", session.sim().treasury().credits(player));
        if text.0 != line {
            text.0 = line;
        }
    }

    let wanted = buildable_now(&session);
    if let Ok((list, children)) = build_list.single() {
        let current: Vec<EntityKind> = children
            .map(|c| {
                c.iter()
                    .filter_map(|e| rows.get(e).ok())
                    .map(|r| r.kind)
                    .collect()
            })
            .unwrap_or_default();
        let same: Vec<EntityKind> = wanted.iter().map(|(k, _, _)| *k).collect();
        if current != same {
            commands.entity(list).despawn_related::<Children>();
            for (kind, name, cost) in &wanted {
                let row = spawn_row(&mut commands, &format!("{name}  ${cost}"), 0.16, 0.19, 0.24);
                commands.entity(row).insert(BuildRow { kind: *kind });
                commands.entity(list).add_child(row);
            }
        }
    }

    // The queue is described by (what, how far along), and the label carries
    // the progress, so the comparison has to include it or a bar that only
    // advances would never redraw.
    let mut queue: Vec<(EntityId, u8, String)> = Vec::new();
    for (id, unit) in session.sim().view().units() {
        if unit.owner != player {
            continue;
        }
        let Some(q) = unit.production.as_ref() else {
            continue;
        };
        for (i, item) in q.items().iter().enumerate() {
            let name = &session.sim().rules().entity(item.kind).id;
            // A zero-duration item is finished the moment it starts, which is
            // legal in the rules and would otherwise divide by zero here.
            let percent = (item.progress * 100)
                .checked_div(item.duration)
                .unwrap_or(100);
            queue.push((id, i as u8, format!("{name}  {percent}%")));
        }
    }
    if let Ok((list, children)) = queue_list.single() {
        let current = children.map_or(0, |c| c.iter().count());
        let labels_changed = current != queue.len()
            || children.is_some_and(|c| {
                c.iter()
                    .filter_map(|e| queued.get(e).ok())
                    .zip(&queue)
                    .any(|(row, (id, i, _))| row.building != *id || row.index != *i)
            });
        if labels_changed || !queue.is_empty() {
            commands.entity(list).despawn_related::<Children>();
            for (building, index, label) in &queue {
                let row = spawn_row(&mut commands, label, 0.12, 0.22, 0.14);
                commands.entity(row).insert(QueueRow {
                    building: *building,
                    index: *index,
                });
                commands.entity(list).add_child(row);
            }
        }
    }
}

/// Rebuilds the superweapon list.
///
/// Separate from `refresh_sidebar` because it changes on a different rhythm —
/// a charge bar moves every tick and the build list changes when a building
/// finishes. Rebuilding the whole panel at the faster rate would eat clicks.
pub fn refresh_powers(
    mut commands: Commands,
    session: Res<Session>,
    list: Query<(Entity, Option<&Children>), With<PowerList>>,
    rows: Query<&PowerRow>,
) {
    let player = session.local_player();
    let mut powers: Vec<(EntityId, String)> = Vec::new();
    for (id, unit) in session.sim().view().units() {
        if unit.owner != player || !unit.is_alive() {
            continue;
        }
        let Some(progress) = session.sim().power_progress(id) else {
            continue;
        };
        let name = &session.sim().rules().entity(unit.kind).id;
        let label = if progress >= 100 {
            format!("{name}  READY")
        } else {
            format!("{name}  {progress}%")
        };
        powers.push((id, label));
    }

    let Ok((list, children)) = list.single() else {
        return;
    };
    let current: Vec<EntityId> = children
        .map(|c| {
            c.iter()
                .filter_map(|e| rows.get(e).ok())
                .map(|r| r.building)
                .collect()
        })
        .unwrap_or_default();
    let same: Vec<EntityId> = powers.iter().map(|(id, _)| *id).collect();
    // The set is compared but the labels are always redrawn, because a charge
    // percentage changes constantly and the set almost never does.
    if current != same {
        commands.entity(list).despawn_related::<Children>();
        for (building, label) in &powers {
            let row = spawn_row(&mut commands, label, 0.26, 0.20, 0.08);
            commands.entity(row).insert(PowerRow {
                building: *building,
            });
            commands.entity(list).add_child(row);
        }
        return;
    }
    if let Some(children) = children {
        for (child, (_, label)) in children.iter().zip(&powers) {
            commands.entity(child).despawn_related::<Children>();
            let text = commands
                .spawn((
                    Text::new(label.clone()),
                    TextFont {
                        font_size: FontSize::Px(12.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.92, 0.86, 0.62)),
                ))
                .id();
            commands.entity(child).add_child(text);
        }
    }
}

fn spawn_row(commands: &mut Commands, label: &str, r: f32, g: f32, b: f32) -> Entity {
    commands
        .spawn((
            Button,
            Node {
                height: Val::Px(ROW_HEIGHT),
                padding: UiRect::horizontal(Val::Px(6.0)),
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgb(r, g, b)),
            children![(
                Text::new(label.to_string()),
                TextFont {
                    font_size: FontSize::Px(12.0),
                    ..default()
                },
                TextColor(Color::srgb(0.86, 0.88, 0.9)),
            )],
        ))
        .id()
}

/// Clicking a build row queues it; clicking a queue row cancels it.
pub fn handle_sidebar_clicks(
    mut session: ResMut<Session>,
    mut sell: ResMut<SellMode>,
    build_rows: Query<(&Interaction, &BuildRow), Changed<Interaction>>,
    queue_rows: Query<(&Interaction, &QueueRow), Changed<Interaction>>,
    sell_button: Query<&Interaction, (Changed<Interaction>, With<SellToggle>)>,
    power_rows: Query<(&Interaction, &PowerRow), Changed<Interaction>>,
    mut aiming: ResMut<AimingPower>,
) {
    for (interaction, row) in &build_rows {
        if *interaction == Interaction::Pressed {
            let player = session.local_player();
            if let Some(building) = session.sim().producer_for(player, row.kind) {
                session.issue(CommandKind::Produce {
                    building,
                    kind: row.kind,
                });
            }
        }
    }
    for (interaction, row) in &queue_rows {
        if *interaction == Interaction::Pressed {
            session.issue(CommandKind::CancelProduction {
                building: row.building,
                index: row.index,
            });
        }
    }
    for (interaction, row) in &power_rows {
        if *interaction == Interaction::Pressed && session.sim().power_ready(row.building) {
            aiming.building = Some(row.building);
        }
    }
    for interaction in &sell_button {
        if *interaction == Interaction::Pressed {
            sell.armed = !sell.armed;
        }
    }
}

/// Shows whether selling is armed.
pub fn paint_sell_toggle(
    sell: Res<SellMode>,
    mut buttons: Query<&mut BackgroundColor, With<SellToggle>>,
) {
    let Ok(mut colour) = buttons.single_mut() else {
        return;
    };
    let wanted = if sell.armed {
        Color::srgb(0.72, 0.18, 0.18)
    } else {
        Color::srgb(0.22, 0.10, 0.10)
    };
    if colour.0 != wanted {
        colour.0 = wanted;
    }
}

/// Whether a click on the world should be read as "sell that".
///
/// Disarmed after one sale, so a player cannot demolish their base by leaving
/// the mode on and clicking about.
pub fn take_sell_click(sell: &mut SellMode) -> bool {
    let armed = sell.armed;
    sell.armed = false;
    armed
}

/// Whether the pointer is over the panel.
///
/// The world's click handlers have to ask, or clicking a build row would also
/// select whatever unit happens to be behind the panel.
pub fn pointer_over_sidebar(window_width: f32, cursor_x: f32) -> bool {
    cursor_x >= window_width - PANEL_WIDTH
}

/// Every player id the panel might have to colour for. Unused for now; kept so
/// the signature does not change when team colours arrive.
pub fn _player_hint(_: PlayerId) {}

/// Describes the selection.
///
/// A line rather than a portrait panel — the portrait is Phase 4 art, and what
/// a player needs before then is the information: what this is, how hurt it is,
/// what rank it has reached, and what is inside it.
///
/// Rank and cargo are the two worth the trouble. A veteran tank hits harder and
/// resists more and there is nothing else on screen that says so; and a
/// transport's contents are invisible by design, which is right in the world
/// and unhelpful in the interface.
pub fn refresh_selection_info(
    session: Res<Session>,
    selection: Res<crate::input::Selection>,
    mut label: Query<&mut Text, With<SelectionInfo>>,
) {
    let Ok(mut text) = label.single_mut() else {
        return;
    };
    let sim = session.sim();

    let line = match selection.units.as_slice() {
        [] => String::new(),
        [one] => match sim.unit(*one) {
            None => String::new(),
            Some(unit) => {
                let stats = sim.stats().get(unit.owner, unit.kind);
                let name = &sim.rules().entity(unit.kind).id;
                let health = (unit.health * 100)
                    .checked_div(stats.max_health)
                    .unwrap_or(100);
                let mut line = format!("{name}  {health}%");
                match sim.rank_of(unit) {
                    redshift_sim::Rank::Rookie => {}
                    rank => line.push_str(&format!("  {rank:?}")),
                }
                if !unit.cargo.is_empty() {
                    line.push_str(&format!("  [{}]", unit.cargo.len()));
                }
                if unit.infestation.is_some() {
                    line.push_str("  INFESTED");
                }
                line
            }
        },
        many => format!("{} selected", many.len()),
    };

    if text.0 != line {
        text.0 = line;
    }
}
