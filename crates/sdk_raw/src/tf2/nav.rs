//! Hand-written values of the navigation mesh TF2's bots walk, which the
//! generated bindings of `CNavArea` (`game/server/nav_area.h`) and
//! `CTFNavArea` (`game/server/tf/nav_mesh/tf_nav_area.h`) do not describe:
//! the bits of an area's attributes, the number of teams an area counts, the
//! slot of the method that finds a character's area, and how to read the
//! vectors an area keeps its connections and hiding spots in.

use crate::vtable_slot;
use std::ffi::c_int;
use std::slice;

// The opaque `CUtlVectorUltraConservative`s of an area are each a pointer to
// their data.
const _: () = assert!(
	size_of::<sys::NavConnectVector>() == size_of::<*const ()>()
		&& size_of::<sys::HidingSpotVector>() == size_of::<*const ()>()
);

/// The slot of `CBaseCombatCharacter::GetLastKnownArea`, the area a combat
/// character was last in, which `CTFPlayer` overrides in the same slot.
pub const GET_LAST_KNOWN_AREA_SLOT: usize = vtable_slot!(
	sys::CBaseCombatCharacter__bindgen_vtable,
	CBaseCombatCharacter_GetLastKnownArea
);

const _: () = assert!(
	GET_LAST_KNOWN_AREA_SLOT
		== vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_GetLastKnownArea)
);

/// How many teams an area counts its players and blocks for (`MAX_NAV_TEAMS`
/// in `nav_area.h`). An area indexes them by the team's number modulo this,
/// so RED (2) is 0 and BLU (3) is 1.
pub const MAX_NAV_TEAMS: usize = 2;

/// The team that stands for every team in the mesh's queries (`TEAM_ANY` in
/// `game/shared/shareddefs.h`).
pub const TEAM_ANY: c_int = -2;

/// `NavAttributeType` (`game/server/nav.h`): the bits of `CNavArea`'s
/// attributes, which mappers set in the nav editor.
pub mod attribute {
	/// Must be crossed crouching (`NAV_MESH_CROUCH`).
	pub const CROUCH: u32 = 0x0000_0001;

	/// Must be crossed jumping; only used while generating
	/// (`NAV_MESH_JUMP`).
	pub const JUMP: u32 = 0x0000_0002;

	/// Is crossed without going around obstacles (`NAV_MESH_PRECISE`).
	pub const PRECISE: u32 = 0x0000_0004;

	/// Is never jumped across (`NAV_MESH_NO_JUMP`).
	pub const NO_JUMP: u32 = 0x0000_0008;

	/// Is entered at a stop (`NAV_MESH_STOP`).
	pub const STOP: u32 = 0x0000_0010;

	/// Is crossed running (`NAV_MESH_RUN`).
	pub const RUN: u32 = 0x0000_0020;

	/// Is crossed walking (`NAV_MESH_WALK`).
	pub const WALK: u32 = 0x0000_0040;

	/// Is avoided unless other ways are too dangerous (`NAV_MESH_AVOID`).
	pub const AVOID: u32 = 0x0000_0080;

	/// May become blocked, so is checked now and then
	/// (`NAV_MESH_TRANSIENT`).
	pub const TRANSIENT: u32 = 0x0000_0100;

	/// Gets no hiding spots (`NAV_MESH_DONT_HIDE`).
	pub const DONT_HIDE: u32 = 0x0000_0200;

	/// Bots hiding in it stand (`NAV_MESH_STAND`).
	pub const STAND: u32 = 0x0000_0400;

	/// Is not used by hostages (`NAV_MESH_NO_HOSTAGES`).
	pub const NO_HOSTAGES: u32 = 0x0000_0800;

	/// Is stairs, walked up rather than jumped (`NAV_MESH_STAIRS`).
	pub const STAIRS: u32 = 0x0000_1000;

	/// Is not merged with the areas next to it (`NAV_MESH_NO_MERGE`).
	pub const NO_MERGE: u32 = 0x0000_2000;

	/// Is where an obstacle is climbed onto (`NAV_MESH_OBSTACLE_TOP`).
	pub const OBSTACLE_TOP: u32 = 0x0000_4000;

	/// Is next to a drop of at least the cliff height (`NAV_MESH_CLIFF`).
	pub const CLIFF: u32 = 0x0000_8000;

	/// Has a cost set by `func_nav_cost` entities (`NAV_MESH_FUNC_COST`).
	pub const FUNC_COST: u32 = 0x2000_0000;

	/// Is in an elevator's path (`NAV_MESH_HAS_ELEVATOR`).
	pub const HAS_ELEVATOR: u32 = 0x4000_0000;

	/// Is blocked by a `func_nav_blocker` (`NAV_MESH_NAV_BLOCKER`).
	pub const NAV_BLOCKER: u32 = 0x8000_0000;
}

/// `TFNavAttributeType` (`tf_nav_area.h`): the bits of `CTFNavArea`'s own
/// attributes, which TF2 sets as the round changes, and mappers in the nav
/// editor.
pub mod tf_attribute {
	/// Blocked for a TF2 reason, such as a closed door (`TF_NAV_BLOCKED`).
	pub const BLOCKED: u32 = 0x0000_0001;

	/// In RED's spawn room (`TF_NAV_SPAWN_ROOM_RED`).
	pub const SPAWN_ROOM_RED: u32 = 0x0000_0002;

	/// In BLU's spawn room (`TF_NAV_SPAWN_ROOM_BLUE`).
	pub const SPAWN_ROOM_BLUE: u32 = 0x0000_0004;

	/// At a spawn room's exit (`TF_NAV_SPAWN_ROOM_EXIT`).
	pub const SPAWN_ROOM_EXIT: u32 = 0x0000_0008;

	/// Holds ammo (`TF_NAV_HAS_AMMO`).
	pub const HAS_AMMO: u32 = 0x0000_0010;

	/// Holds health (`TF_NAV_HAS_HEALTH`).
	pub const HAS_HEALTH: u32 = 0x0000_0020;

	/// On a control point (`TF_NAV_CONTROL_POINT`).
	pub const CONTROL_POINT: u32 = 0x0000_0040;

	/// Within a BLU sentry's reach (`TF_NAV_BLUE_SENTRY_DANGER`).
	pub const BLUE_SENTRY_DANGER: u32 = 0x0000_0080;

	/// Within a RED sentry's reach (`TF_NAV_RED_SENTRY_DANGER`).
	pub const RED_SENTRY_DANGER: u32 = 0x0000_0100;

	/// Blocked for BLU until setup ends (`TF_NAV_BLUE_SETUP_GATE`).
	pub const BLUE_SETUP_GATE: u32 = 0x0000_0800;

	/// Blocked for RED until setup ends (`TF_NAV_RED_SETUP_GATE`).
	pub const RED_SETUP_GATE: u32 = 0x0000_1000;

	/// Blocked once the first point is captured
	/// (`TF_NAV_BLOCKED_AFTER_POINT_CAPTURE`).
	pub const BLOCKED_AFTER_POINT_CAPTURE: u32 = 0x0000_2000;

	/// Blocked until the first point is captured
	/// (`TF_NAV_BLOCKED_UNTIL_POINT_CAPTURE`).
	pub const BLOCKED_UNTIL_POINT_CAPTURE: u32 = 0x0000_4000;

	/// A door only BLU passes (`TF_NAV_BLUE_ONE_WAY_DOOR`).
	pub const BLUE_ONE_WAY_DOOR: u32 = 0x0000_8000;

	/// A door only RED passes (`TF_NAV_RED_ONE_WAY_DOOR`).
	pub const RED_ONE_WAY_DOOR: u32 = 0x0001_0000;

	/// Makes the point-capture blocks wait for the second point
	/// (`TF_NAV_WITH_SECOND_POINT`).
	pub const WITH_SECOND_POINT: u32 = 0x0002_0000;

	/// Makes the point-capture blocks wait for the third point
	/// (`TF_NAV_WITH_THIRD_POINT`).
	pub const WITH_THIRD_POINT: u32 = 0x0004_0000;

	/// Makes the point-capture blocks wait for the fourth point
	/// (`TF_NAV_WITH_FOURTH_POINT`).
	pub const WITH_FOURTH_POINT: u32 = 0x0008_0000;

	/// Makes the point-capture blocks wait for the fifth point
	/// (`TF_NAV_WITH_FIFTH_POINT`).
	pub const WITH_FIFTH_POINT: u32 = 0x0010_0000;

	/// A good place for a Sniper (`TF_NAV_SNIPER_SPOT`).
	pub const SNIPER_SPOT: u32 = 0x0020_0000;

	/// A good place for a sentry (`TF_NAV_SENTRY_SPOT`).
	pub const SENTRY_SPOT: u32 = 0x0040_0000;

	/// On the escape route of the unreleased Raid mode
	/// (`TF_NAV_ESCAPE_ROUTE`).
	pub const ESCAPE_ROUTE: u32 = 0x0080_0000;

	/// In sight of Raid mode's escape route
	/// (`TF_NAV_ESCAPE_ROUTE_VISIBLE`).
	pub const ESCAPE_ROUTE_VISIBLE: u32 = 0x0100_0000;

	/// Where bots are not spawned (`TF_NAV_NO_SPAWNING`).
	pub const NO_SPAWNING: u32 = 0x0200_0000;

	/// Where Raid mode respawns players (`TF_NAV_RESCUE_CLOSET`).
	pub const RESCUE_CLOSET: u32 = 0x0400_0000;

	/// Where the Mann vs. Machine bomb may drop and robots reach it
	/// (`TF_NAV_BOMB_CAN_DROP_HERE`).
	pub const BOMB_CAN_DROP_HERE: u32 = 0x0800_0000;

	/// Its door never blocks it (`TF_NAV_DOOR_NEVER_BLOCKS`).
	pub const DOOR_NEVER_BLOCKS: u32 = 0x1000_0000;

	/// Its door always blocks it (`TF_NAV_DOOR_ALWAYS_BLOCKS`).
	pub const DOOR_ALWAYS_BLOCKS: u32 = 0x2000_0000;

	/// Never blocked (`TF_NAV_UNBLOCKABLE`).
	pub const UNBLOCKABLE: u32 = 0x4000_0000;
}

/// The elements of a `CUtlVectorUltraConservative<T>` (`tier1/utlvector.h`),
/// such as an area's `NavConnectVector` of `NavConnect`s or its
/// `HidingSpotVector` of `HidingSpot *`s, whose generated bindings are
/// opaque: the vector is a pointer to its count followed by its elements, or
/// to a shared, empty block.
///
/// # Safety
///
/// `vector` must point to a live `CUtlVectorUltraConservative<T>`, whose
/// elements nothing changes or frees while the slice lives.
pub unsafe fn ultra_conservative_elements<'a, T>(vector: *const impl Sized) -> &'a [T] {
	// SAFETY: The vector is a single pointer to its data, which is never null:
	// an empty vector points to its static empty block.
	let data = unsafe {
		vector
			.cast::<*const sys::CUtlVectorUltraConservative_Data_t<T>>()
			.read()
	};

	// SAFETY: The data holds the count, then that many elements, aligned as
	// the generated `Data_t` lays them out.
	unsafe {
		let len = usize::try_from((*data).m_Size).unwrap_or(0);
		let elements = (&raw const (*data).m_Elements).cast::<T>();

		slice::from_raw_parts(elements, len)
	}
}
