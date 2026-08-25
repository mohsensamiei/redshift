//! Telling the player the match is over.
//!
//! The simulation has known how a match ends for a while and nothing said so
//! on screen. A game that quietly stops mattering, with both sides still
//! standing there, is worse than one with no victory condition at all: at
//! least the second is honestly unfinished.

use bevy::prelude::*;
use bevy::text::FontSize;
use redshift_sim::sim::Outcome;

use crate::session::Session;

/// The banner. One node, spawned once, hidden until there is something to say.
#[derive(Component)]
pub struct VerdictBanner;

pub fn spawn_banner(commands: &mut Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(38.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            // Hidden rather than absent, so nothing has to be spawned at the
            // moment a match ends — which is a tick where a good deal else is
            // already happening.
            Visibility::Hidden,
            VerdictBanner,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new(""),
                TextFont {
                    font_size: FontSize::Px(40.0),
                    ..default()
                },
                TextColor(Color::srgb(0.95, 0.94, 0.88)),
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.65)),
                Node {
                    padding: UiRect::axes(Val::Px(28.0), Val::Px(14.0)),
                    ..default()
                },
            ));
        });
}

pub fn update_banner(
    session: Res<Session>,
    mut banners: Query<(&mut Visibility, &Children), With<VerdictBanner>>,
    mut texts: Query<&mut Text>,
) {
    let Ok((mut visibility, children)) = banners.single_mut() else {
        return;
    };
    let Some(outcome) = session.sim().outcome() else {
        *visibility = Visibility::Hidden;
        return;
    };

    // Said from the local player's side. "Victory(1)" is a fact about the
    // match; "Defeat" is what the person watching needs to know.
    let line = match outcome {
        Outcome::Victory(winner) if winner == session.local_player() => "VICTORY",
        Outcome::Victory(_) => "DEFEAT",
        Outcome::Stalemate => "STALEMATE",
    };

    *visibility = Visibility::Visible;
    for child in children.iter() {
        if let Ok(mut text) = texts.get_mut(child)
            && text.0 != line
        {
            text.0 = line.to_string();
        }
    }
}
