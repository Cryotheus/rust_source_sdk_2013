//! TF2's navigation mesh: the areas (`CTFNavArea`) its bots and other NextBot
//! actors walk, which the game loads from the level's `.nav` file.
//!
//! [`NavArea::last_known`] finds the area a combat character, such as a
//! player or a NextBot actor, was last in, as the game tracks it
//! (`GetLastKnownArea`). [`NavArea`] reads an area in place, through the
//! generated layout of `CTFNavArea` and, for the queries TF2 overrides, its
//! vtable: its corners and the height of its ground, its attributes, the
//! players in it, whether it is blocked or in sight of a team, how far it
//! lies from each team's spawn rooms, the areas it connects to, and its
//! hiding spots.
//!
//! Finding an area by position or ID needs the mesh itself (`TheNavMesh`),
//! which this module does not reach yet.
//!
//! # Lifetimes
//!
//! An area lives as long as the mesh holds it. The mesh frees its areas when
//! a level loads its own, which [`Server::new`]'s contract keeps out of `'s`,
//! and when the nav editor's console commands, such as `nav_generate` or
//! `nav_delete`, change it. Those run only for the server's console or a
//! listen server's host, from the command buffer between frames, unless a
//! caller runs them at once through the unsafe
//! [`GameClient::execute_string_command`](crate::interfaces::GameClient::execute_string_command).
//!
//! # Unverified
//!
//! The wrappers follow the generated layouts and Valve's Source SDK 2013
//! (`game/server/nav_area.h` and `game/server/tf/nav_mesh/tf_nav_area.h`),
//! and have not been tested on a live server.

#[cfg(test)]
#[path = "../tests/tf2/nav.rs"]
mod tests;

use crate::entities::Entity;
use crate::math::Vector;
use crate::tf2::scoreboard::ScoringTeam;
use crate::{Game, Server};
use sdk_raw::tf2::nav::{self as raw, attribute, tf_attribute};
use sdk_raw::vcall;
use std::ffi::{CStr, c_int};
use std::marker::PhantomData;
use std::ptr::NonNull;

/// The data map class of every entity that tracks its area.
const COMBAT_CHARACTER_CLASS: &CStr = c"CBaseCombatCharacter";

/// Why an entity's area could not be found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NavError {
	/// The server is not running Team Fortress 2, whose areas these wrappers
	/// read as `CTFNavArea`s.
	#[error("navigation areas require Team Fortress 2")]
	UnsupportedGame,

	/// The entity is not a combat character, so it tracks no area.
	#[error("the entity is not a combat character")]
	NotACombatCharacter,
}

/// One of the four sides of an area, along which it connects to others
/// (`NavDirType`). North is towards −y and east towards +x.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NavDirection {
	/// Towards −y (`NORTH`).
	North,

	/// Towards +x (`EAST`).
	East,

	/// Towards +y (`SOUTH`).
	South,

	/// Towards −x (`WEST`).
	West,
}

impl NavDirection {
	/// Every direction, in native order.
	pub const ALL: [Self; 4] = [Self::North, Self::East, Self::South, Self::West];

	/// The direction's `NavDirType`.
	pub const fn to_raw(self) -> sys::NavDirType {
		match self {
			Self::North => sys::NavDirType_NORTH,
			Self::East => sys::NavDirType_EAST,
			Self::South => sys::NavDirType_SOUTH,
			Self::West => sys::NavDirType_WEST,
		}
	}
}

/// One of the four corners of an area (`NavCornerType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NavCorner {
	/// At the lowest x and y (`NORTH_WEST`).
	NorthWest,

	/// At the highest x and lowest y (`NORTH_EAST`).
	NorthEast,

	/// At the highest x and y (`SOUTH_EAST`).
	SouthEast,

	/// At the lowest x and highest y (`SOUTH_WEST`).
	SouthWest,
}

impl NavCorner {
	/// Every corner, in native order.
	pub const ALL: [Self; 4] = [
		Self::NorthWest,
		Self::NorthEast,
		Self::SouthEast,
		Self::SouthWest,
	];

	/// The corner's `NavCornerType`.
	pub const fn to_raw(self) -> sys::NavCornerType {
		match self {
			Self::NorthWest => sys::NavCornerType_NORTH_WEST,
			Self::NorthEast => sys::NavCornerType_NORTH_EAST,
			Self::SouthEast => sys::NavCornerType_SOUTH_EAST,
			Self::SouthWest => sys::NavCornerType_SOUTH_WEST,
		}
	}
}

bitflags::bitflags! {
	/// An area's attributes (`NavAttributeType`), which mappers set in the
	/// nav editor. Unnamed bits, such as those of other games, are kept.
	#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
	pub struct NavAttributes: u32 {
		/// Is avoided unless other ways are too dangerous.
		#[doc(alias("NAV_MESH_AVOID"))]
		const AVOID = attribute::AVOID;

		/// Is next to a drop of at least the cliff height.
		#[doc(alias("NAV_MESH_CLIFF"))]
		const CLIFF = attribute::CLIFF;

		/// Must be crossed crouching.
		#[doc(alias("NAV_MESH_CROUCH"))]
		const CROUCH = attribute::CROUCH;

		/// Gets no hiding spots.
		#[doc(alias("NAV_MESH_DONT_HIDE"))]
		const DONT_HIDE = attribute::DONT_HIDE;

		/// Has a cost set by `func_nav_cost` entities.
		#[doc(alias("NAV_MESH_FUNC_COST"))]
		const FUNC_COST = attribute::FUNC_COST;

		/// Is in an elevator's path.
		#[doc(alias("NAV_MESH_HAS_ELEVATOR"))]
		const HAS_ELEVATOR = attribute::HAS_ELEVATOR;

		/// Must be crossed jumping; only used while generating the mesh.
		#[doc(alias("NAV_MESH_JUMP"))]
		const JUMP = attribute::JUMP;

		/// Is blocked by a `func_nav_blocker`.
		#[doc(alias("NAV_MESH_NAV_BLOCKER"))]
		const NAV_BLOCKER = attribute::NAV_BLOCKER;

		/// Is not used by hostages.
		#[doc(alias("NAV_MESH_NO_HOSTAGES"))]
		const NO_HOSTAGES = attribute::NO_HOSTAGES;

		/// Is never jumped across.
		#[doc(alias("NAV_MESH_NO_JUMP"))]
		const NO_JUMP = attribute::NO_JUMP;

		/// Is not merged with the areas next to it.
		#[doc(alias("NAV_MESH_NO_MERGE"))]
		const NO_MERGE = attribute::NO_MERGE;

		/// Is where an obstacle is climbed onto.
		#[doc(alias("NAV_MESH_OBSTACLE_TOP"))]
		const OBSTACLE_TOP = attribute::OBSTACLE_TOP;

		/// Is crossed without going around obstacles.
		#[doc(alias("NAV_MESH_PRECISE"))]
		const PRECISE = attribute::PRECISE;

		/// Is crossed running.
		#[doc(alias("NAV_MESH_RUN"))]
		const RUN = attribute::RUN;

		/// Bots hiding in it stand.
		#[doc(alias("NAV_MESH_STAND"))]
		const STAND = attribute::STAND;

		/// Is stairs, walked up rather than jumped.
		#[doc(alias("NAV_MESH_STAIRS"))]
		const STAIRS = attribute::STAIRS;

		/// Is entered at a stop.
		#[doc(alias("NAV_MESH_STOP"))]
		const STOP = attribute::STOP;

		/// May become blocked, so is checked now and then.
		#[doc(alias("NAV_MESH_TRANSIENT"))]
		const TRANSIENT = attribute::TRANSIENT;

		/// Is crossed walking.
		#[doc(alias("NAV_MESH_WALK"))]
		const WALK = attribute::WALK;
	}
}

bitflags::bitflags! {
	/// An area's TF2 attributes (`TFNavAttributeType`), which the game sets as
	/// the round changes, and mappers in the nav editor. Unnamed bits are
	/// kept.
	#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
	pub struct TfNavAttributes: u32 {
		/// Blocked for a TF2 reason, such as a closed door.
		#[doc(alias("TF_NAV_BLOCKED"))]
		const BLOCKED = tf_attribute::BLOCKED;

		/// Blocked once the first point is captured.
		#[doc(alias("TF_NAV_BLOCKED_AFTER_POINT_CAPTURE"))]
		const BLOCKED_AFTER_POINT_CAPTURE = tf_attribute::BLOCKED_AFTER_POINT_CAPTURE;

		/// Blocked until the first point is captured.
		#[doc(alias("TF_NAV_BLOCKED_UNTIL_POINT_CAPTURE"))]
		const BLOCKED_UNTIL_POINT_CAPTURE = tf_attribute::BLOCKED_UNTIL_POINT_CAPTURE;

		/// A door only BLU passes.
		#[doc(alias("TF_NAV_BLUE_ONE_WAY_DOOR"))]
		const BLUE_ONE_WAY_DOOR = tf_attribute::BLUE_ONE_WAY_DOOR;

		/// Within a BLU sentry's reach.
		#[doc(alias("TF_NAV_BLUE_SENTRY_DANGER"))]
		const BLUE_SENTRY_DANGER = tf_attribute::BLUE_SENTRY_DANGER;

		/// Blocked for BLU until setup ends.
		#[doc(alias("TF_NAV_BLUE_SETUP_GATE"))]
		const BLUE_SETUP_GATE = tf_attribute::BLUE_SETUP_GATE;

		/// Where the Mann vs. Machine bomb may drop and robots reach it.
		#[doc(alias("TF_NAV_BOMB_CAN_DROP_HERE"))]
		const BOMB_CAN_DROP_HERE = tf_attribute::BOMB_CAN_DROP_HERE;

		/// On a control point.
		#[doc(alias("TF_NAV_CONTROL_POINT"))]
		const CONTROL_POINT = tf_attribute::CONTROL_POINT;

		/// Its door always blocks it.
		#[doc(alias("TF_NAV_DOOR_ALWAYS_BLOCKS"))]
		const DOOR_ALWAYS_BLOCKS = tf_attribute::DOOR_ALWAYS_BLOCKS;

		/// Its door never blocks it.
		#[doc(alias("TF_NAV_DOOR_NEVER_BLOCKS"))]
		const DOOR_NEVER_BLOCKS = tf_attribute::DOOR_NEVER_BLOCKS;

		/// On the escape route of the unreleased Raid mode.
		#[doc(alias("TF_NAV_ESCAPE_ROUTE"))]
		const ESCAPE_ROUTE = tf_attribute::ESCAPE_ROUTE;

		/// In sight of Raid mode's escape route.
		#[doc(alias("TF_NAV_ESCAPE_ROUTE_VISIBLE"))]
		const ESCAPE_ROUTE_VISIBLE = tf_attribute::ESCAPE_ROUTE_VISIBLE;

		/// Holds ammo.
		#[doc(alias("TF_NAV_HAS_AMMO"))]
		const HAS_AMMO = tf_attribute::HAS_AMMO;

		/// Holds health.
		#[doc(alias("TF_NAV_HAS_HEALTH"))]
		const HAS_HEALTH = tf_attribute::HAS_HEALTH;

		/// Where bots are not spawned.
		#[doc(alias("TF_NAV_NO_SPAWNING"))]
		const NO_SPAWNING = tf_attribute::NO_SPAWNING;

		/// A door only RED passes.
		#[doc(alias("TF_NAV_RED_ONE_WAY_DOOR"))]
		const RED_ONE_WAY_DOOR = tf_attribute::RED_ONE_WAY_DOOR;

		/// Within a RED sentry's reach.
		#[doc(alias("TF_NAV_RED_SENTRY_DANGER"))]
		const RED_SENTRY_DANGER = tf_attribute::RED_SENTRY_DANGER;

		/// Blocked for RED until setup ends.
		#[doc(alias("TF_NAV_RED_SETUP_GATE"))]
		const RED_SETUP_GATE = tf_attribute::RED_SETUP_GATE;

		/// Where Raid mode respawns players.
		#[doc(alias("TF_NAV_RESCUE_CLOSET"))]
		const RESCUE_CLOSET = tf_attribute::RESCUE_CLOSET;

		/// A good place for a sentry.
		#[doc(alias("TF_NAV_SENTRY_SPOT"))]
		const SENTRY_SPOT = tf_attribute::SENTRY_SPOT;

		/// A good place for a Sniper.
		#[doc(alias("TF_NAV_SNIPER_SPOT"))]
		const SNIPER_SPOT = tf_attribute::SNIPER_SPOT;

		/// In BLU's spawn room.
		#[doc(alias("TF_NAV_SPAWN_ROOM_BLUE"))]
		const SPAWN_ROOM_BLUE = tf_attribute::SPAWN_ROOM_BLUE;

		/// At a spawn room's exit.
		#[doc(alias("TF_NAV_SPAWN_ROOM_EXIT"))]
		const SPAWN_ROOM_EXIT = tf_attribute::SPAWN_ROOM_EXIT;

		/// In RED's spawn room.
		#[doc(alias("TF_NAV_SPAWN_ROOM_RED"))]
		const SPAWN_ROOM_RED = tf_attribute::SPAWN_ROOM_RED;

		/// Never blocked.
		#[doc(alias("TF_NAV_UNBLOCKABLE"))]
		const UNBLOCKABLE = tf_attribute::UNBLOCKABLE;

		/// Makes the point-capture blocks wait for the fifth point.
		#[doc(alias("TF_NAV_WITH_FIFTH_POINT"))]
		const WITH_FIFTH_POINT = tf_attribute::WITH_FIFTH_POINT;

		/// Makes the point-capture blocks wait for the fourth point.
		#[doc(alias("TF_NAV_WITH_FOURTH_POINT"))]
		const WITH_FOURTH_POINT = tf_attribute::WITH_FOURTH_POINT;

		/// Makes the point-capture blocks wait for the second point.
		#[doc(alias("TF_NAV_WITH_SECOND_POINT"))]
		const WITH_SECOND_POINT = tf_attribute::WITH_SECOND_POINT;

		/// Makes the point-capture blocks wait for the third point.
		#[doc(alias("TF_NAV_WITH_THIRD_POINT"))]
		const WITH_THIRD_POINT = tf_attribute::WITH_THIRD_POINT;
	}
}

bitflags::bitflags! {
	/// A hiding spot's flags, which the mesh's analysis sets. Unnamed bits are
	/// kept.
	#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
	pub struct HidingSpotFlags: u8 {
		/// In the open, usually on a ledge or cliff.
		const EXPOSED = sys::HidingSpot_EXPOSED as u8;

		/// With at least one decent sniping corridor.
		const GOOD_SNIPER_SPOT = sys::HidingSpot_GOOD_SNIPER_SPOT as u8;

		/// Seeing very far, a large area, or both.
		const IDEAL_SNIPER_SPOT = sys::HidingSpot_IDEAL_SNIPER_SPOT as u8;

		/// In a corner with hard cover nearby.
		const IN_COVER = sys::HidingSpot_IN_COVER as u8;
	}
}

/// A connection from an area to one next to it, in one direction
/// (`NavConnect`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavConnection<'s> {
	/// The area connected to.
	pub area: NavArea<'s>,

	/// The distance between the two areas' centres.
	pub length: f32,
}

/// A spot in an area where a bot can hide (`HidingSpot`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HidingSpot {
	/// The spot's ID, unique within the mesh.
	pub id: u32,

	/// What the mesh's analysis found the spot good for.
	pub flags: HidingSpotFlags,

	/// Where the spot is.
	pub position: Vector,
}

/// One of the areas of TF2's navigation mesh (`CTFNavArea`), within the
/// engine callback `'s`.
///
/// An area is a rectangle in x and y, whose four corners may lie at
/// different heights. The [module documentation](self#lifetimes) explains
/// how long it lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NavArea<'s> {
	area: NonNull<sys::CTFNavArea>,
	_scope: PhantomData<&'s ()>,
}

impl<'s> NavArea<'s> {
	/// Wraps a TF2 area.
	///
	/// # Safety
	///
	/// `area` must point to a `CTFNavArea` of the running server's mesh, which
	/// stays allocated for all of `'s`.
	pub const unsafe fn from_raw(area: NonNull<sys::CTFNavArea>) -> Self {
		Self {
			area,
			_scope: PhantomData,
		}
	}

	/// The area a combat character, such as a player or a NextBot actor, was
	/// last in, as the game tracks it (`GetLastKnownArea`), or `None` if it
	/// has not been on the mesh, such as on a level without one.
	///
	/// The game updates it as the character moves, and keeps it while the
	/// character is off the mesh, such as in the air.
	#[doc(alias("GetLastKnownArea"))]
	pub fn last_known(server: Server<'s>, entity: Entity<'s>) -> Result<Option<Self>, NavError> {
		if server.game() != Game::TeamFortress2 {
			return Err(NavError::UnsupportedGame);
		}

		if !entity.is_a(COMBAT_CHARACTER_CLASS) {
			return Err(NavError::NotACombatCharacter);
		}

		let character = entity.as_ptr().cast::<sys::CBaseCombatCharacter>();

		// SAFETY: The entity's data maps include `CBaseCombatCharacter`'s, so
		// it is one, whose primary vtable has the entry at the slot
		// `sdk_raw::tf2::nav` reads from the generated binding, and checks
		// `CTFPlayer` overrides in place. The method returns a field.
		let area = unsafe {
			vcall!(character as sys::CBaseCombatCharacter__bindgen_vtable => CBaseCombatCharacter_GetLastKnownArea())
		};

		// SAFETY: TF2's mesh makes every area a `CTFNavArea`, whose `CNavArea`
		// is at offset zero, and which the mesh frees only as the module
		// documentation describes.
		Ok(NonNull::new(area).map(|area| unsafe { Self::from_raw(area.cast()) }))
	}

	/// The area's attributes.
	pub fn attributes(self) -> NavAttributes {
		NavAttributes::from_bits_retain(self.critical().m_attributeFlags as u32)
	}

	/// The point of the area's ground nearest to `position`
	/// (`GetClosestPointOnArea`).
	#[doc(alias("GetClosestPointOnArea"))]
	pub fn closest_point(self, position: Vector) -> Vector {
		let critical = self.critical();
		let x = position
			.x
			.max(critical.m_nwCorner.x)
			.min(critical.m_seCorner.x);
		let y = position
			.y
			.max(critical.m_nwCorner.y)
			.min(critical.m_seCorner.y);

		Vector::new(x, y, self.ground_height(x, y))
	}

	/// The area's centre (`GetCenter`).
	#[doc(alias("GetCenter"))]
	pub fn center(self) -> Vector {
		self.critical().m_center.into()
	}

	/// The area's ground at one of its corners (`GetCorner`).
	#[doc(alias("GetCorner"))]
	pub fn corner(self, corner: NavCorner) -> Vector {
		let critical = self.critical();
		let (north_west, south_east) = (critical.m_nwCorner, critical.m_seCorner);

		match corner {
			NavCorner::NorthWest => north_west.into(),
			NavCorner::NorthEast => Vector::new(south_east.x, north_west.y, critical.m_neZ),
			NavCorner::SouthEast => south_east.into(),
			NavCorner::SouthWest => Vector::new(north_west.x, south_east.y, critical.m_swZ),
		}
	}

	/// The areas the area connects to along one side (`GetAdjacentArea`).
	/// A connection may be one way.
	#[doc(alias("GetAdjacentArea", "GetAdjacentCount", "m_connect"))]
	pub fn connections(
		self,
		direction: NavDirection,
	) -> impl Iterator<Item = NavConnection<'s>> + use<'s> {
		let vector = &raw const self.critical().m_connect[direction.to_raw() as usize];

		// SAFETY: The area is live for `'s`, and its connections are a
		// `CUtlVectorUltraConservative<NavConnect>`, which only the nav editor
		// changes. Once the mesh has loaded, each connection holds the area it
		// connects to.
		let connections = unsafe { raw::ultra_conservative_elements::<sys::NavConnect>(vector) };

		connections.iter().filter_map(|connection| {
			// SAFETY: As above. Only corrupt navigation data, which the mesh
			// reports as it loads, leaves the area null.
			let area = NonNull::new(unsafe { connection.__bindgen_anon_1.area })?;

			Some(NavConnection {
				// SAFETY: The connected area is another of the mesh's areas,
				// which all are `CTFNavArea`s.
				area: unsafe { Self::from_raw(area.cast()) },
				length: connection.length,
			})
		})
	}

	/// The lowest and highest corners of the box that holds the area's ground
	/// (`GetExtent`).
	#[doc(alias("GetExtent"))]
	pub fn extent(self) -> (Vector, Vector) {
		let critical = self.critical();
		let (north_west, south_east) = (critical.m_nwCorner, critical.m_seCorner);
		let heights = [north_west.z, south_east.z, critical.m_neZ, critical.m_swZ];
		let low = heights.into_iter().fold(f32::INFINITY, f32::min);
		let high = heights.into_iter().fold(f32::NEG_INFINITY, f32::max);

		(
			Vector::new(north_west.x, north_west.y, low),
			Vector::new(south_east.x, south_east.y, high),
		)
	}

	/// The height of the area's ground at `x` and `y`, which are clamped to the
	/// area (`GetZ`). The ground runs straight between the corners' heights
	/// along each axis.
	#[doc(alias("GetZ"))]
	pub fn ground_height(self, x: f32, y: f32) -> f32 {
		let critical = self.critical();

		if critical.m_invDxCorners == 0.0 || critical.m_invDyCorners == 0.0 {
			return critical.m_neZ;
		}

		let (north_west, south_east) = (critical.m_nwCorner, critical.m_seCorner);
		let u = ((x - north_west.x) * critical.m_invDxCorners).clamp(0.0, 1.0);
		let v = ((y - north_west.y) * critical.m_invDyCorners).clamp(0.0, 1.0);
		let north = north_west.z + u * (critical.m_neZ - north_west.z);
		let south = critical.m_swZ + u * (south_east.z - critical.m_swZ);

		north + v * (south - north)
	}

	/// The spots in the area where bots can hide.
	#[doc(alias("m_hidingSpots"))]
	pub fn hiding_spots(self) -> impl Iterator<Item = HidingSpot> + use<'s> {
		// SAFETY: The area is live for `'s`, and its hiding spots are a
		// `CUtlVectorUltraConservative<HidingSpot *>`, which only the nav
		// editor changes, of spots the mesh frees with the area.
		let spots = unsafe {
			raw::ultra_conservative_elements::<*mut sys::HidingSpot>(
				&raw const self.base().m_hidingSpots,
			)
		};

		spots.iter().filter_map(|&spot| {
			// SAFETY: As above, the area holds live spots, read before any game
			// code runs.
			let spot = unsafe { spot.as_ref() }?;

			Some(HidingSpot {
				id: spot.m_id,
				flags: HidingSpotFlags::from_bits_retain(spot.m_flags),
				position: spot.m_pos.into(),
			})
		})
	}

	/// The area's ID, unique within the mesh and kept in the `.nav` file
	/// (`GetID`).
	#[doc(alias("GetID"))]
	pub fn id(self) -> u32 {
		self.base().m_id
	}

	/// How far a team's players travel from their spawn rooms to reach the
	/// area, or `None` if they cannot reach it (`GetIncursionDistance`). The
	/// game computes this as the round starts.
	#[doc(alias("GetIncursionDistance", "IsReachableByTeam"))]
	pub fn incursion_distance(self, team: ScoringTeam) -> Option<f32> {
		let distance = self.tf().m_distanceFromSpawnRoom[team.to_raw() as usize];

		(distance >= 0.0).then_some(distance)
	}

	/// Whether the area is blocked for a team, or for either team for `None`
	/// (`IsBlocked`): by TF2's attributes, a closed door, or a
	/// `func_nav_blocker`.
	#[doc(alias("IsBlocked"))]
	pub fn is_blocked(self, team: Option<ScoringTeam>) -> bool {
		let area = self.area.as_ptr().cast_const();
		let team = team.map_or(raw::TEAM_ANY, ScoringTeam::to_raw);

		// SAFETY: The area is a live `CTFNavArea`, whose vtable is laid out as
		// the generated binding's. The method reads the area's fields.
		unsafe {
			vcall!(area as sys::CTFNavArea__bindgen_vtable => CTFNavArea_IsBlocked(team, false))
		}
	}

	/// Whether `position` lies within the area's x and y, widened by
	/// `tolerance` (`IsOverlapping`).
	#[doc(alias("IsOverlapping"))]
	pub fn is_overlapping(self, position: Vector, tolerance: f32) -> bool {
		let critical = self.critical();

		position.x + tolerance >= critical.m_nwCorner.x
			&& position.x - tolerance <= critical.m_seCorner.x
			&& position.y + tolerance >= critical.m_nwCorner.y
			&& position.y - tolerance <= critical.m_seCorner.y
	}

	/// Whether a living player of `team` may see part of the area, as the
	/// game last found (`IsPotentiallyVisibleToTeam`).
	#[doc(alias("IsPotentiallyVisibleToTeam"))]
	pub fn is_potentially_visible_to(self, team: ScoringTeam) -> bool {
		let area = self.area.as_ptr().cast_const();
		let team: c_int = team.to_raw();

		// SAFETY: As for `is_blocked`.
		unsafe {
			vcall!(area as sys::CTFNavArea__bindgen_vtable => CTFNavArea_IsPotentiallyVisibleToTeam(team))
		}
	}

	/// Whether the area is under water (`IsUnderwater`).
	#[doc(alias("IsUnderwater"))]
	pub fn is_underwater(self) -> bool {
		self.base().m_isUnderwater
	}

	/// How many players of a team are in the area, or of both teams for
	/// `None` (`GetPlayerCount`).
	#[doc(alias("GetPlayerCount"))]
	pub fn player_count(self, team: Option<ScoringTeam>) -> u32 {
		let counts = self.critical().m_playerCount;

		match team {
			Some(team) => counts[team.to_raw() as usize % raw::MAX_NAV_TEAMS].into(),
			None => counts.iter().map(|&count| u32::from(count)).sum(),
		}
	}

	/// The area's TF2 attributes (`GetAttributesTF`).
	#[doc(alias("GetAttributesTF"))]
	pub fn tf_attributes(self) -> TfNavAttributes {
		TfNavAttributes::from_bits_retain(self.tf().m_attributeFlags)
	}

	/// How far Mann vs. Machine's robots travel from the area to the bomb's
	/// hatch, or `None` if they cannot reach it
	/// (`GetTravelDistanceToBombTarget`). The game computes this in Mann vs.
	/// Machine only, as the round starts, and leaves it 0 otherwise.
	#[doc(alias("GetTravelDistanceToBombTarget"))]
	pub fn travel_distance_to_bomb_target(self) -> Option<f32> {
		let distance = self.tf().m_distanceToBombTarget;

		(distance >= 0.0).then_some(distance)
	}

	/// The area's raw pointer, for calls this crate does not wrap.
	pub const fn as_ptr(self) -> *mut sys::CTFNavArea {
		self.area.as_ptr()
	}

	/// The area's `CNavArea` part.
	fn base(&self) -> &sys::CNavArea {
		&self.tf()._base
	}

	/// The fields of the area that searches through the mesh read most
	/// (`CNavAreaCriticalData`).
	fn critical(&self) -> &sys::CNavAreaCriticalData {
		&self.base()._base
	}

	/// The area's `CTFNavArea` fields, borrowed only while a method reads
	/// them, since the game changes some of them, such as its player counts,
	/// as characters move.
	fn tf(&self) -> &sys::CTFNavArea {
		// SAFETY: `from_raw`'s caller vouches that the area is live for `'s`,
		// and the borrow ends before the method returns, with no game code run
		// while it lasts.
		unsafe { self.area.as_ref() }
	}
}
