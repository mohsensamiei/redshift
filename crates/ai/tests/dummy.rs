//! What a dummy opponent does, and — more to the point — what it will not do.
//!
//! The claim worth testing hardest is the negative one. "It never attacks" is
//! easy to write and easy to get subtly wrong: an opponent that chases a
//! retreating scout across the map is attacking, whatever the code says it is
//! doing.

use redshift_ai::{Commander, Difficulty};
use redshift_data::rules::Rules;
use redshift_sim::command::{Command, CommandKind, PlayerId};
use redshift_sim::map::{Cell, Map};
use redshift_sim::sim::{MatchSetup, PlayerSetup, Sim, Spawn};
use redshift_sim::{EntityId, Tick};

fn shipped_rules() -> Rules {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rules");
    Rules::load_from(&root).expect("the shipped rules should load")
}

/// A map with ore near both bases, so an economy is possible at all.
fn field() -> Map {
    let mut map = Map::new(64, 64);
    map.add_ore_field(Cell::new(14, 14), 4, 400);
    map.add_ore_field(Cell::new(50, 50), 4, 400);
    map
}

/// A match with the computer at slot 1 and whatever the caller wants at 0.
fn scenario(spawns: Vec<(u8, &str, i32, i32)>) -> Sim {
    let rules = shipped_rules();
    let spawns = spawns
        .into_iter()
        .map(|(owner, id, x, y)| Spawn {
            owner: PlayerId(owner),
            kind: rules
                .kind_of(id)
                .unwrap_or_else(|| panic!("no entity {id:?}")),
            pos: Cell::new(x, y).centre(),
        })
        .collect();
    Sim::new(MatchSetup {
        seed: 0x_A1,
        map: field(),
        players: vec![
            PlayerSetup {
                id: PlayerId(0),
                faction: None,
            },
            PlayerSetup {
                id: PlayerId(1),
                faction: None,
            },
        ],
        spawns,
        rules,
    })
}

/// Runs the match with the computer thinking, and nobody else doing anything.
fn run(sim: &mut Sim, ai: &mut Commander, ticks: Tick) {
    for _ in 0..ticks {
        let orders = ai.think(sim);
        let commands: Vec<Command> = orders
            .into_iter()
            .enumerate()
            .map(|(i, kind)| Command::new(ai.player(), i as u16, kind))
            .collect();
        sim.tick(&commands);
    }
}

fn owned(sim: &Sim, player: u8) -> Vec<(EntityId, String)> {
    sim.view()
        .units()
        .filter(|(_, u)| u.owner == PlayerId(player) && u.is_alive())
        .map(|(id, u)| (id, sim.rules().entity(u.kind).id.clone()))
        .collect()
}

fn count(sim: &Sim, player: u8, id: &str) -> usize {
    owned(sim, player).iter().filter(|(_, n)| n == id).count()
}

// -- It builds --------------------------------------------------------------

#[test]
fn it_builds_something() {
    // The floor. An opponent that sits on its starting money is not an
    // opponent, whatever its difficulty says.
    let mut sim = scenario(vec![(1, "soviet_construction_yard", 50, 50)]);
    let mut ai = Commander::new(PlayerId(1), Difficulty::Dummy);
    let before = owned(&sim, 1).len();

    run(&mut sim, &mut ai, 2_000);

    assert!(
        owned(&sim, 1).len() > before,
        "it built nothing at all in a hundred seconds"
    );
}

#[test]
fn it_builds_power_before_anything_that_needs_it() {
    // An opponent that queued a barracks it could not power would look exactly
    // like a bug, and the player would be right to think so.
    let mut sim = scenario(vec![(1, "soviet_construction_yard", 50, 50)]);
    let mut ai = Commander::new(PlayerId(1), Difficulty::Dummy);
    run(&mut sim, &mut ai, 3_000);

    assert!(
        count(&sim, 1, "tesla_reactor") > 0,
        "it never built a power plant: {:?}",
        owned(&sim, 1)
    );
}

#[test]
fn it_builds_an_economy() {
    let mut sim = scenario(vec![(1, "soviet_construction_yard", 50, 50)]);
    let mut ai = Commander::new(PlayerId(1), Difficulty::Dummy);
    run(&mut sim, &mut ai, 12_000);

    let refineries = count(&sim, 1, "soviet_refinery");
    let miners = count(&sim, 1, "war_miner");
    assert!(refineries > 0, "no refinery: {:?}", owned(&sim, 1));
    assert!(miners > 0, "no miners: {:?}", owned(&sim, 1));
}

#[test]
fn its_buildings_do_not_pile_up_on_one_spot() {
    // The placement search has to actually work outwards. A version that always
    // returned the first legal cell would put every building in a line against
    // the same edge of the yard.
    let mut sim = scenario(vec![(1, "soviet_construction_yard", 50, 50)]);
    let mut ai = Commander::new(PlayerId(1), Difficulty::Dummy);
    run(&mut sim, &mut ai, 12_000);

    let places: std::collections::BTreeSet<(i32, i32)> = sim
        .view()
        .units()
        .filter(|(_, u)| u.owner == PlayerId(1) && u.is_alive())
        .filter(|(_, u)| !sim.stats().get(u.owner, u.kind).mobile)
        .map(|(_, u)| (u.cell().x, u.cell().y))
        .collect();
    assert!(
        places.len() >= 3,
        "only {} distinct building sites",
        places.len()
    );
}

// -- It does not attack -----------------------------------------------------

#[test]
fn it_never_leaves_its_base() {
    // The claim the whole difficulty rests on. An enemy sitting quietly on the
    // far side of the map should be left entirely alone.
    let mut sim = scenario(vec![
        (1, "soviet_construction_yard", 50, 50),
        (0, "grizzly_tank", 8, 8),
        (0, "construction_yard", 10, 10),
    ]);
    let mut ai = Commander::new(PlayerId(1), Difficulty::Dummy);
    let victim = sim.units().ids()[1];
    let before = sim.unit(victim).unwrap().health;

    run(&mut sim, &mut ai, 12_000);

    assert_eq!(
        sim.unit(victim).map(|u| u.health),
        Some(before),
        "it came across the map and attacked"
    );
    // And nothing of its own wandered over either.
    for (_, unit) in sim.view().units() {
        if unit.owner != PlayerId(1) {
            continue;
        }
        assert!(
            unit.cell().chebyshev_to(Cell::new(50, 50)) < 30,
            "one of its units is at {:?}, a long way from home",
            unit.cell()
        );
    }
}

#[test]
fn it_defends_itself() {
    // Not a punching bag. Walk into its base and it fights back — which is the
    // half of "dummy" that makes it worth playing against at all.
    let mut sim = scenario(vec![
        (1, "soviet_construction_yard", 50, 50),
        (1, "rhino_tank", 52, 50),
        (1, "rhino_tank", 50, 52),
        (0, "grizzly_tank", 48, 48),
    ]);
    let mut ai = Commander::new(PlayerId(1), Difficulty::Dummy);
    let intruder = sim.units().ids()[3];
    let before = sim.unit(intruder).unwrap().health;

    run(&mut sim, &mut ai, 600);

    assert!(
        sim.unit(intruder).is_none_or(|u| u.health < before),
        "an enemy stood in its base and was ignored"
    );
}

#[test]
fn it_stops_chasing_at_the_edge_of_its_base() {
    // The subtle way "never attacks" goes wrong: an opponent that defends by
    // ordering an attack, and then keeps the order as the target retreats, has
    // attacked — whatever the code says it is doing.
    let mut sim = scenario(vec![
        (1, "soviet_construction_yard", 50, 50),
        (1, "rhino_tank", 52, 50),
        (0, "grizzly_tank", 46, 46),
    ]);
    let mut ai = Commander::new(PlayerId(1), Difficulty::Dummy);
    let ids = sim.units().ids();
    let (defender, scout) = (ids[1], ids[2]);

    run(&mut sim, &mut ai, 200);

    // The scout runs for the far corner.
    let flee = Command::new(
        PlayerId(0),
        0,
        CommandKind::Move {
            units: vec![scout],
            target: Cell::new(4, 4),
        },
    );
    sim.tick(&[flee]);
    run(&mut sim, &mut ai, 4_000);

    let at = sim.unit(defender).map(|u| u.cell());
    assert!(
        at.is_none_or(|c| c.chebyshev_to(Cell::new(50, 50)) < 30),
        "the defender chased to {at:?}"
    );
}

// -- How it thinks ----------------------------------------------------------

#[test]
fn it_thinks_at_its_own_pace() {
    // Reaction time is the whole of the difficulty scale, so an opponent that
    // ignored its own interval would be the same speed at every setting.
    let mut sim = scenario(vec![(1, "soviet_construction_yard", 50, 50)]);
    let mut ai = Commander::new(PlayerId(1), Difficulty::Dummy);

    let mut acted = 0;
    for _ in 0..600 {
        if !ai.think(&sim).is_empty() {
            acted += 1;
        }
        sim.tick(&[]);
    }
    // Twelve ticks between thoughts at forty percent competence: fifty chances
    // in six hundred ticks, and it cannot have taken more than that.
    assert!(
        acted <= 600 / Difficulty::Dummy.think_interval(),
        "acted {acted} times, more often than it is allowed to think"
    );
}

#[test]
fn a_dummy_is_as_clever_as_an_easy_opponent() {
    // The distinction the player was promised: same head, different appetite.
    assert_eq!(
        Difficulty::Dummy.competence(),
        Difficulty::Easy.competence()
    );
    assert!(!Difficulty::Dummy.attacks());
    assert!(Difficulty::Easy.attacks());
}

#[test]
fn the_ladder_climbs() {
    // Every number that describes competence has to move the same way, or a
    // "harder" opponent is only harder in some respects.
    let ladder = [Difficulty::Easy, Difficulty::Medium, Difficulty::Hard];
    for pair in ladder.windows(2) {
        let (worse, better) = (pair[0], pair[1]);
        assert!(better.competence() > worse.competence());
        assert!(
            better.think_interval() <= worse.think_interval(),
            "{better:?} reacts slower than {worse:?}"
        );
        assert!(better.spend_share() > worse.spend_share());
        assert!(better.harvesters_wanted() >= worse.harvesters_wanted());
    }
}

#[test]
fn it_is_deterministic() {
    // The constraint that makes an opponent shippable at all: two peers running
    // the same match must issue the same commands on the same ticks, or the
    // opponent is a desync generator that looks like a netcode bug.
    let go = || {
        let mut sim = scenario(vec![
            (1, "soviet_construction_yard", 50, 50),
            (0, "grizzly_tank", 48, 48),
        ]);
        let mut ai = Commander::new(PlayerId(1), Difficulty::Dummy);
        run(&mut sim, &mut ai, 3_000);
        sim.state_hash()
    };
    assert_eq!(go(), go());
}
