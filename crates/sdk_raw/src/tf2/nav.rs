//! Hand-written values of the navigation mesh TF2's bots walk, which the
//! generated bindings of `CNavArea` (`game/server/nav_area.h`) and
//! `CTFNavArea` (`game/server/tf/nav_mesh/tf_nav_area.h`) do not describe:
//! the bits of an area's attributes, the number of teams an area counts, the
//! slot of the method that finds a character's area, how to read the vectors
//! an area keeps its connections and hiding spots in, and where the game
//! server module keeps the list of every area (`TheNavAreas`).

#[cfg(test)]
#[path = "../tests/tf2/nav.rs"]
mod tests;

use crate::interfaces::CreateInterfaceFn;
use crate::util::{self, ModuleCache, ModuleKey};
use crate::vtable_slot;
use std::ffi::c_int;
use std::mem::offset_of;
use std::ptr::NonNull;
use std::slice;

#[cfg(target_os = "linux")]
use crate::util::elf::LoadedElf;

#[cfg(any(target_os = "windows", test))]
use crate::util::{Image, SignaturePattern, relative, sig};

// `TheNavAreas` is a `CUtlVector` whose memory starts with the pointer to
// its elements, which its count follows, as the code that finds it reads
// them.
const _: () = assert!(
	offset_of!(NavAreaVector, m_Memory) == 0
		&& offset_of!(sys::CUtlMemory<*mut sys::CNavArea>, m_pMemory) == 0
		&& offset_of!(NavAreaVector, m_Size) == 16
);

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

/// `NavAreaVector` (`nav_area.h`): the vector of areas that `TheNavAreas`,
/// the list of every area of the mesh, is.
pub type NavAreaVector = sys::CUtlVector<*mut sys::CNavArea, sys::CUtlMemory<*mut sys::CNavArea>>;

/// What [`cached_nav_areas`] found in the last module it searched: the
/// address of `TheNavAreas`, or `None` if the module lacks it where this
/// module looks.
static NAV_AREAS: ModuleCache<Option<usize>> = ModuleCache::new();

/// The global variable `TheNavAreas`, whose name the Itanium ABI does not
/// mangle.
#[cfg(target_os = "linux")]
const NAV_AREAS_SYMBOL: &[u8] = b"TheNavAreas";

/// The message `nav_update_lighting` prints with the count of
/// `TheNavAreas`, with its terminator.
#[cfg(any(target_os = "windows", test))]
const UPDATE_LIGHTING_MESSAGE: &[u8] = b"Computed lighting for %d/%d areas\n\0";

// `nav_update_lighting`'s loop over `TheNavAreas`, `CNavMesh::
// CommandNavUpdateLighting`, in TF2's 64-bit Windows `server.dll`, up to its
// message with the areas' count. The wildcards are `rip`-relative operands,
// the slot of `ComputeLighting`, and the loop's branch back.
//
//  +0  mov rax, [rip+TheNavAreas]          ; m_Memory.m_pMemory
//  +7  mov ecx, edi
//  +9  mov rcx, [rax+rcx*8]
// +13  mov rax, [rcx]
// +16  call [rax+ComputeLighting]
// +22  test al, al
// +24  lea ecx, [rbx+1]
// +27  cmovz ecx, ebx
// +30  inc edi
// +32  cmp edi, [rip+TheNavAreas+16]       ; m_Size
// +38  mov ebx, ecx
// +40  jl +0
// +42  mov r8d, [rip+TheNavAreas+16]       ; m_Size
// +49  lea rcx, [rip+message]
// +56  mov edx, ebx
// +58  call [rip+DevMsg]
#[cfg(any(target_os = "windows", test))]
const UPDATE_LIGHTING: [SignaturePattern; 64] = sig![
	0x48 0x8b 0x05 ? ? ? ?
	0x8b 0xcf
	0x48 0x8b 0x0c 0xc8
	0x48 0x8b 0x01
	0xff 0x90 ? ? ? ?
	0x84 0xc0
	0x8d 0x4b 0x01
	0x0f 0x44 0xcb
	0xff 0xc7
	0x3b 0x3d ? ? ? ?
	0x8b 0xd9
	0x7c ?
	0x44 0x8b 0x05 ? ? ? ?
	0x48 0x8d 0x0d ? ? ? ?
	0x8b 0xd3
	0xff 0x15 ? ? ? ?
];

/// Where [`UPDATE_LIGHTING`] reads the pointer to the areas.
#[cfg(any(target_os = "windows", test))]
const UPDATE_LIGHTING_ELEMENTS: usize = 3;

/// Where [`UPDATE_LIGHTING`] loads [`UPDATE_LIGHTING_MESSAGE`].
#[cfg(any(target_os = "windows", test))]
const UPDATE_LIGHTING_MESSAGE_OPERAND: usize = 52;

/// Where [`UPDATE_LIGHTING`] reads the areas' count, in the loop and for the
/// message.
#[cfg(any(target_os = "windows", test))]
const UPDATE_LIGHTING_SIZES: [usize; 2] = [34, 45];

/// Finds `TheNavAreas`, the list of every area of TF2's navigation mesh, in
/// the module whose `CreateInterface` export is `factory`, such as the game
/// server module, or `Ok(None)` if the module lacks it where this function
/// looks. The list holds `CTFNavArea`s, and is empty until a level with a
/// mesh loads.
///
/// On Windows, the list is the one `nav_update_lighting`'s code counts, found
/// by a signature of that code verified in TF2's 64-bit `server.dll`: its three
/// references to the list must agree, and the message it prints must be the
/// command's. On Linux, it is the variable the module's symbols name
/// `TheNavAreas`, which is not checked against a retail build. Either way, the
/// list must lie in a writable section of the module, which on Windows must
/// not be executable.
///
/// The address is metadata from a snapshot of the module: it does not keep
/// the module loaded, and is the list only while the module stays loaded.
///
/// # Safety
///
/// `factory` must be the `CreateInterface` export of a module that stays
/// loaded throughout this call.
#[doc(alias("TheNavAreas"))]
pub unsafe fn find_nav_areas(
	factory: CreateInterfaceFn,
) -> Result<Option<NonNull<NavAreaVector>>, util::Error> {
	#[cfg(target_os = "windows")]
	let address = {
		// SAFETY: The factory is an executable address in its module, which the
		// caller keeps loaded while it is inspected.
		let image = unsafe { Image::load(factory as usize) }?;

		nav_areas_in(&image)
	};

	#[cfg(target_os = "linux")]
	let address = {
		// SAFETY: As above.
		let elf = unsafe { LoadedElf::at(factory as usize) }?;

		elf.resolve_data(NAV_AREAS_SYMBOL)
			.filter(|&(address, size)| {
				size >= size_of::<NavAreaVector>() && address % align_of::<NavAreaVector>() == 0
			})
			.map(|(address, _)| address)
	};

	Ok(address.and_then(|address| NonNull::new(address as *mut NavAreaVector)))
}

/// Finds `TheNavAreas` as [`find_nav_areas`] does, but inspects each module
/// once: a later call for the same module, mapped at the same base with its
/// `CreateInterface` at the same address, returns what the first found,
/// including `None`. A failure is not kept.
///
/// # Safety
///
/// - `factory` must be the `CreateInterface` export of a module that stays
///   loaded throughout this call.
/// - If an earlier call inspected a module mapped at the same base, with its
///   `CreateInterface` at the same address, the module containing `factory`
///   must be that same image, as [`ModuleCache`] assumes.
pub unsafe fn cached_nav_areas(
	factory: CreateInterfaceFn,
) -> Result<Option<NonNull<NavAreaVector>>, util::Error> {
	// SAFETY: The caller keeps the factory's module loaded for this call.
	let key = unsafe { ModuleKey::of(factory as usize) }?;

	let address = NAV_AREAS.get_or_resolve(key, || {
		// SAFETY: As the caller promises.
		unsafe { find_nav_areas(factory) }.map(|areas| areas.map(|areas| areas.as_ptr() as usize))
	})?;

	// SAFETY: The address came from a search of this module, in this call or,
	// on a cache hit, in an earlier one, of the same image, as the caller
	// promises.
	Ok(address.and_then(|address| NonNull::new(address as *mut NavAreaVector)))
}

/// The areas `areas` holds, read in place.
///
/// # Safety
///
/// `areas` must point to a live `NavAreaVector`, such as `TheNavAreas`, whose
/// elements nothing changes or frees while the slice lives.
pub unsafe fn nav_area_elements<'a>(areas: NonNull<NavAreaVector>) -> &'a [*mut sys::CNavArea] {
	// SAFETY: As the caller promises. The fields are read without forming a
	// reference to the vector, which the game writes.
	unsafe {
		let areas = areas.as_ptr();
		let elements = (&raw const (*areas).m_Memory.m_pMemory).read();
		let len = usize::try_from((&raw const (*areas).m_Size).read()).unwrap_or(0);

		if elements.is_null() || len == 0 {
			&[]
		} else {
			slice::from_raw_parts(elements, len)
		}
	}
}

/// Finds `TheNavAreas` in a snapshot of a Windows module, through
/// [`UPDATE_LIGHTING`].
#[cfg(any(target_os = "windows", test))]
fn nav_areas_in(image: &Image) -> Option<usize> {
	let at = image.unique(&UPDATE_LIGHTING, 1)?;
	let code = image.read(at, UPDATE_LIGHTING.len())?;
	let areas = relative(at, code, UPDATE_LIGHTING_ELEMENTS)?;
	let size = areas.checked_add(offset_of!(NavAreaVector, m_Size))?;
	let message = relative(at, code, UPDATE_LIGHTING_MESSAGE_OPERAND)?;

	let agrees = UPDATE_LIGHTING_SIZES
		.iter()
		.all(|&operand| relative(at, code, operand) == Some(size));

	(agrees
		&& !image.executable(message)
		&& image.read(message, UPDATE_LIGHTING_MESSAGE.len()) == Some(UPDATE_LIGHTING_MESSAGE)
		&& !image.executable(areas)
		&& image.contains(areas, size_of::<NavAreaVector>(), false, true))
	.then_some(areas)
}
