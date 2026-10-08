//! Tests of `crate::tf2::nav`: areas laid out in memory as the game lays out
//! `CTFNavArea`, with a mock vtable for the queries TF2 overrides, and mock
//! characters that report them as their last known area.

use super::*;
use crate::test_support::entities::{MockEntity, set_datamap, state_maps};
use crate::test_support::server::null_server;
use crate::test_support::tf2::spawning::patch_slots;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use sdk_raw::tf2::nav::GET_LAST_KNOWN_AREA_SLOT;
use sdk_raw::tf2::scoreboard::{TF_TEAM_BLUE, TF_TEAM_RED};
use std::cell::{Cell, RefCell};
use std::ptr::null_mut;

/// The data of a `CUtlVectorUltraConservative<T>` of `N` elements.
#[repr(C)]
struct VectorData<T, const N: usize> {
	size: c_int,
	elements: [T; N],
}

thread_local! {
	/// The area the mock characters last knew.
	static LAST_KNOWN: Cell<*mut sys::CNavArea> = const { Cell::new(null_mut()) };

	/// The teams the mock vtable's queries were asked about, by query.
	static QUERIES: RefCell<Vec<(&'static str, c_int, bool)>> = const { RefCell::new(Vec::new()) };
}

/// `CBaseCombatCharacter::GetLastKnownArea`, which returns [`LAST_KNOWN`].
unsafe extern "C" fn last_known_area(_: *const sys::CBaseCombatCharacter) -> *mut sys::CNavArea {
	LAST_KNOWN.get()
}

/// The ID of the areas [`is_blocked`] blocks for every team.
const BLOCKED_ID: u32 = 9;

/// `CTFNavArea::IsBlocked`, blocked for RED, and for every team in the area
/// with [`BLOCKED_ID`].
unsafe extern "C" fn is_blocked(area: *const sys::CTFNavArea, team: c_int, ignore: bool) -> bool {
	QUERIES.with_borrow_mut(|queries| queries.push(("IsBlocked", team, ignore)));

	// SAFETY: The mock vtable belongs to the leaked areas of these tests.
	team == TF_TEAM_RED || unsafe { (*area)._base.m_id } == BLOCKED_ID
}

/// `CTFNavArea::IsPotentiallyVisibleToTeam`, visible to BLU only.
unsafe extern "C" fn is_visible(_: *const sys::CTFNavArea, team: c_int) -> bool {
	QUERIES.with_borrow_mut(|queries| queries.push(("IsPotentiallyVisibleToTeam", team, false)));

	team == TF_TEAM_BLUE
}

/// Points an area's opaque vector to `data`, leaked.
fn set_vector<T, const N: usize>(vector: *mut sys::NavConnectVector, elements: [T; N]) {
	let data = Box::leak(Box::new(VectorData {
		size: c_int::try_from(N).unwrap(),
		elements,
	}));

	// SAFETY: The opaque vector is a pointer to its data.
	unsafe { vector.cast::<*const VectorData<T, N>>().write(data) };
}

/// A leaked area with no connections, hiding spots, attributes or players,
/// whose vtable answers [`is_blocked`] and [`is_visible`].
fn empty_area(id: u32) -> NonNull<sys::CTFNavArea> {
	// SAFETY: Every field of an area is an integer, float, bool, or pointer,
	// for which zero is valid.
	let area = Box::leak(unsafe { Box::<sys::CTFNavArea>::new_zeroed().assume_init() });

	// SAFETY: The vtable holds only function pointers, `unexpected_call` aborts
	// whichever other slot reaches it, and the patch only writes slots of the
	// vtable being built.
	let vtable = unsafe {
		mock_vtable::<sys::CTFNavArea__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).CTFNavArea_IsBlocked).write(is_blocked);
			(&raw mut (*vtable).CTFNavArea_IsPotentiallyVisibleToTeam).write(is_visible);
		})
	};

	let base = &mut area._base;

	base.vtable_ = Box::leak(vtable) as *const sys::CTFNavArea__bindgen_vtable as *const _;
	base.m_id = id;

	for direction in &mut base._base.m_connect {
		set_vector::<sys::NavConnect, 0>(direction, []);
	}

	set_vector::<*mut sys::HidingSpot, 0>(&raw mut base.m_hidingSpots, []);

	NonNull::from(area)
}

/// An area from (0, 0) to (100, 200), whose ground rises from 10 at its
/// north-west corner to 20 at its north-east one, 30 at its south-east one,
/// and 40 at its south-west one, with attributes, players, distances, a
/// hiding spot, and connections to two other areas to its east, one of them
/// left null as corrupt data leaves it.
fn area() -> (NavArea<'static>, NonNull<sys::CTFNavArea>) {
	let mut raw = empty_area(7);
	let neighbour = empty_area(8);

	// SAFETY: The area was just leaked, and nothing else refers to it.
	let area = unsafe { raw.as_mut() };
	let base = &mut area._base;
	let critical = &mut base._base;

	critical.m_nwCorner = Vector::new(0.0, 0.0, 10.0).into();
	critical.m_seCorner = Vector::new(100.0, 200.0, 30.0).into();
	critical.m_invDxCorners = 1.0 / 100.0;
	critical.m_invDyCorners = 1.0 / 200.0;
	critical.m_neZ = 20.0;
	critical.m_swZ = 40.0;
	critical.m_center = Vector::new(50.0, 100.0, 25.0).into();
	critical.m_playerCount = [2, 3];
	critical.m_attributeFlags = (attribute::CROUCH | attribute::NAV_BLOCKER) as c_int;

	set_vector(
		&raw mut critical.m_connect[sys::NavDirType_EAST as usize],
		[
			sys::NavConnect {
				__bindgen_anon_1: sys::NavConnect__bindgen_ty_1 {
					area: neighbour.as_ptr().cast(),
				},
				length: 150.0,
			},
			sys::NavConnect {
				__bindgen_anon_1: sys::NavConnect__bindgen_ty_1 { area: null_mut() },
				length: 0.0,
			},
		],
	);

	// SAFETY: Every field of a hiding spot is an integer or pointer, for which
	// zero is valid.
	let spot = Box::leak(unsafe { Box::<sys::HidingSpot>::new_zeroed().assume_init() });

	spot.m_id = 11;
	spot.m_pos = Vector::new(5.0, 6.0, 12.0).into();
	spot.m_flags = (sys::HidingSpot_IN_COVER | sys::HidingSpot_EXPOSED) as u8;

	set_vector(&raw mut base.m_hidingSpots, [&raw mut *spot, null_mut()]);

	base.m_isUnderwater = true;
	area.m_attributeFlags = tf_attribute::SPAWN_ROOM_RED | tf_attribute::HAS_HEALTH | (1 << 31);
	area.m_distanceFromSpawnRoom = [-1.0, -1.0, 500.0, -1.0];
	area.m_distanceToBombTarget = -1.0;

	// SAFETY: The area is leaked, and laid out as the game's.
	(unsafe { NavArea::from_raw(raw) }, raw)
}

/// A leaked flat area at `height`, from `north_west` to `south_east` in x
/// and y.
fn flat_area(
	id: u32,
	north_west: (f32, f32),
	south_east: (f32, f32),
	height: f32,
) -> NonNull<sys::CTFNavArea> {
	let mut raw = empty_area(id);

	// SAFETY: The area was just leaked, and nothing else refers to it.
	let critical = unsafe { &mut raw.as_mut()._base._base };

	critical.m_nwCorner = Vector::new(north_west.0, north_west.1, height).into();
	critical.m_seCorner = Vector::new(south_east.0, south_east.1, height).into();
	critical.m_invDxCorners = 1.0 / (south_east.0 - north_west.0);
	critical.m_invDyCorners = 1.0 / (south_east.1 - north_west.1);
	critical.m_neZ = height;
	critical.m_swZ = height;

	raw
}

/// A leaked list of `areas`, as `TheNavAreas` holds them.
fn area_list(areas: Vec<NonNull<sys::CTFNavArea>>) -> NonNull<raw::NavAreaVector> {
	let elements = Box::leak(
		areas
			.into_iter()
			.map(|area| area.as_ptr().cast::<sys::CNavArea>())
			.collect::<Box<[_]>>(),
	);

	// SAFETY: A zeroed vector, of null pointers and counts, is an empty one.
	let list = Box::leak(unsafe { Box::<raw::NavAreaVector>::new_zeroed().assume_init() });

	list.m_Memory.m_pMemory = elements.as_mut_ptr();
	list.m_Memory.m_nAllocationCount = c_int::try_from(elements.len()).unwrap();
	list.m_Size = list.m_Memory.m_nAllocationCount;
	list.m_pElements = elements.as_mut_ptr();

	NonNull::from(list)
}

/// A mesh of two floors over (0, 0) to (100, 100), at heights 0 and 100, an
/// area from (200, 0) to (300, 100) at height 0, and between them an area
/// blocked for every team, from (160, 0) to (190, 100).
fn mesh() -> NavMesh<'static> {
	let areas = area_list(vec![
		flat_area(1, (0.0, 0.0), (100.0, 100.0), 0.0),
		flat_area(2, (0.0, 0.0), (100.0, 100.0), 100.0),
		flat_area(3, (200.0, 0.0), (300.0, 100.0), 0.0),
		flat_area(BLOCKED_ID, (160.0, 0.0), (190.0, 100.0), 0.0),
	]);

	// SAFETY: The list and its areas are leaked, and laid out as the game's.
	unsafe { NavMesh::from_raw(areas) }
}

/// The ID of `area`, if any.
fn id(area: Option<NavArea<'_>>) -> Option<u32> {
	area.map(NavArea::id)
}

#[test]
fn meshes_list_their_areas() {
	let mesh = mesh();

	assert_eq!(mesh.area_count(), 4);
	assert_eq!(
		mesh.areas().map(NavArea::id).collect::<Vec<_>>(),
		[1, 2, 3, BLOCKED_ID]
	);
	assert_eq!(id(mesh.area_by_id(3)), Some(3));
	assert_eq!(id(mesh.area_by_id(42)), None);
	assert_eq!(id(mesh.area_by_id(0)), None);

	// SAFETY: The list is leaked, and empty, as on a level without a mesh.
	let empty = unsafe { NavMesh::from_raw(area_list(Vec::new())) };

	assert_eq!(empty.area_count(), 0);
	assert_eq!(empty.areas().count(), 0);
	assert_eq!(
		id(empty.area_at(Vector::new(0.0, 0.0, 0.0), NavMesh::BENEATH_LIMIT)),
		None
	);
	assert_eq!(
		id(empty.nearest_area(Vector::new(0.0, 0.0, 0.0), 1000.0, None)),
		None
	);
}

#[test]
fn meshes_find_the_highest_ground_under_a_position() {
	let mesh = mesh();
	let at = |x, y, z, beneath| id(mesh.area_at(Vector::new(x, y, z), beneath));

	assert_eq!(at(50.0, 50.0, 0.0, NavMesh::BENEATH_LIMIT), Some(1));
	assert_eq!(at(50.0, 50.0, 100.0, NavMesh::BENEATH_LIMIT), Some(2));
	assert_eq!(at(50.0, 50.0, 60.0, NavMesh::BENEATH_LIMIT), Some(1));

	// Ground up to 5 units above the position counts as under it.
	assert_eq!(at(50.0, 50.0, -5.0, NavMesh::BENEATH_LIMIT), Some(1));
	assert_eq!(at(50.0, 50.0, -5.5, NavMesh::BENEATH_LIMIT), None);
	assert_eq!(at(50.0, 50.0, 95.0, NavMesh::BENEATH_LIMIT), Some(2));

	// Ground further down than the limit does not.
	assert_eq!(at(50.0, 50.0, 230.0, NavMesh::BENEATH_LIMIT), None);
	assert_eq!(at(50.0, 50.0, 230.0, 130.0), Some(2));
	assert_eq!(at(150.0, 50.0, 0.0, NavMesh::BENEATH_LIMIT), None);

	// Blocked areas are still found, as in the game.
	assert_eq!(
		at(170.0, 50.0, 0.0, NavMesh::BENEATH_LIMIT),
		Some(BLOCKED_ID)
	);
}

#[test]
fn meshes_find_the_nearest_area_not_blocked() {
	let mesh = mesh();
	let nearest = |x, max, team| id(mesh.nearest_area(Vector::new(x, 50.0, 0.0), max, team));

	// The blocked area between is skipped, leaving the third area 20 units
	// away and the first 80.
	assert_eq!(nearest(180.0, 1000.0, None), Some(3));
	assert_eq!(nearest(180.0, 1000.0, Some(ScoringTeam::Blue)), Some(3));
	assert_eq!(nearest(180.0, 20.5, None), Some(3));
	assert_eq!(nearest(180.0, 20.0, None), None);
	assert_eq!(nearest(120.0, 1000.0, None), Some(1));

	// The mock blocks every area for RED.
	assert_eq!(nearest(180.0, 1000.0, Some(ScoringTeam::Red)), None);

	let _ = QUERIES.take();
}

#[test]
fn meshes_are_found_in_tf2_only() {
	let scope = ();

	assert_eq!(
		NavMesh::find(null_server(Game::SourceSdk2013, &scope)),
		Err(NavError::UnsupportedGame)
	);
}

#[test]
fn areas_report_their_shape() {
	let (area, _) = area();

	assert_eq!(area.id(), 7);
	assert_eq!(area.center(), Vector::new(50.0, 100.0, 25.0));
	assert_eq!(
		NavCorner::ALL.map(|corner| area.corner(corner)),
		[
			Vector::new(0.0, 0.0, 10.0),
			Vector::new(100.0, 0.0, 20.0),
			Vector::new(100.0, 200.0, 30.0),
			Vector::new(0.0, 200.0, 40.0),
		]
	);
	assert_eq!(
		area.extent(),
		(Vector::new(0.0, 0.0, 10.0), Vector::new(100.0, 200.0, 40.0))
	);

	// Halfway along each axis, the ground is halfway between the north edge's
	// 15 and the south edge's 35.
	assert_eq!(area.ground_height(50.0, 100.0), 25.0);
	assert_eq!(area.ground_height(-50.0, 300.0), 40.0);
	assert_eq!(
		area.closest_point(Vector::new(150.0, -10.0, 0.0)),
		Vector::new(100.0, 0.0, 20.0)
	);
	assert!(area.is_overlapping(Vector::new(105.0, 100.0, 0.0), 5.0));
	assert!(!area.is_overlapping(Vector::new(105.0, 100.0, 0.0), 4.0));
	assert!(area.is_underwater());
}

#[test]
fn areas_report_their_attributes_and_teams() {
	let (area, _) = area();

	assert_eq!(
		area.attributes(),
		NavAttributes::CROUCH | NavAttributes::NAV_BLOCKER
	);
	assert_eq!(
		area.tf_attributes(),
		TfNavAttributes::SPAWN_ROOM_RED
			| TfNavAttributes::HAS_HEALTH
			| TfNavAttributes::from_bits_retain(1 << 31)
	);
	assert_eq!(area.player_count(Some(ScoringTeam::Red)), 2);
	assert_eq!(area.player_count(Some(ScoringTeam::Blue)), 3);
	assert_eq!(area.player_count(None), 5);
	assert_eq!(area.incursion_distance(ScoringTeam::Red), Some(500.0));
	assert_eq!(area.incursion_distance(ScoringTeam::Blue), None);
	assert_eq!(area.travel_distance_to_bomb_target(), None);
}

#[test]
fn areas_ask_their_vtable_whether_they_are_blocked_or_visible() {
	let (area, _) = area();

	assert!(area.is_blocked(Some(ScoringTeam::Red)));
	assert!(!area.is_blocked(Some(ScoringTeam::Blue)));
	assert!(!area.is_blocked(None));
	assert!(area.is_potentially_visible_to(ScoringTeam::Blue));
	assert!(!area.is_potentially_visible_to(ScoringTeam::Red));
	assert_eq!(
		QUERIES.take(),
		[
			("IsBlocked", TF_TEAM_RED, false),
			("IsBlocked", TF_TEAM_BLUE, false),
			("IsBlocked", raw::TEAM_ANY, false),
			("IsPotentiallyVisibleToTeam", TF_TEAM_BLUE, false),
			("IsPotentiallyVisibleToTeam", TF_TEAM_RED, false),
		]
	);
}

#[test]
fn areas_list_their_connections_and_hiding_spots() {
	let (area, _) = area();

	let east = area.connections(NavDirection::East).collect::<Vec<_>>();

	assert_eq!(east.len(), 1, "the null connection is skipped");
	assert_eq!(east[0].area.id(), 8);
	assert_eq!(east[0].length, 150.0);
	assert_eq!(area.connections(NavDirection::West).count(), 0);
	assert_eq!(east[0].area.connections(NavDirection::West).count(), 0);
	assert_eq!(
		area.hiding_spots().collect::<Vec<_>>(),
		[HidingSpot {
			id: 11,
			flags: HidingSpotFlags::IN_COVER | HidingSpotFlags::EXPOSED,
			position: Vector::new(5.0, 6.0, 12.0),
		}]
	);
	assert_eq!(east[0].area.hiding_spots().count(), 0);
}

#[test]
fn directions_and_corners_have_their_game_values() {
	assert_eq!(NavDirection::ALL.map(NavDirection::to_raw), [0, 1, 2, 3]);
	assert_eq!(NavCorner::ALL.map(NavCorner::to_raw), [0, 1, 2, 3]);
}

#[test]
fn characters_report_their_last_known_area() {
	let scope = ();
	let server = null_server(Game::TeamFortress2, &scope);
	let (area, raw) = area();
	let mut character = MockEntity::new(1);
	let mut prop = MockEntity::new(2);

	patch_slots(
		&mut character,
		&[(GET_LAST_KNOWN_AREA_SLOT, last_known_area as *const ())],
	);

	// SAFETY: Mock entities are leaked, and their vtables answer what the
	// wrappers call of a `CBaseEntity` and of a combat character.
	let (character, prop) = unsafe {
		(
			Entity::from_raw(NonNull::new(character.as_ptr()).unwrap()),
			Entity::from_raw(NonNull::new(prop.as_ptr()).unwrap()),
		)
	};

	set_datamap(state_maps(vec![(c"CBaseCombatCharacter", Vec::new())]));

	assert_eq!(NavArea::last_known(server, character), Ok(None));

	LAST_KNOWN.set(raw.as_ptr().cast());

	assert_eq!(NavArea::last_known(server, character), Ok(Some(area)));

	let other_game = null_server(Game::SourceSdk2013, &scope);

	assert_eq!(
		NavArea::last_known(other_game, character),
		Err(NavError::UnsupportedGame)
	);

	set_datamap(state_maps(Vec::new()));

	assert_eq!(
		NavArea::last_known(server, prop),
		Err(NavError::NotACombatCharacter)
	);
}
