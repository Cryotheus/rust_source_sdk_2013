//! TF2 weapon creation, inventory slots, equipment, and replacement.
//!
//! `give` creates a stock weapon by classname; `give_item` selects an economy
//! item definition, such as the Iron Bomber. Native item generation initializes
//! item definitions, schema attributes, and models before spawning. Attributes
//! can also be changed through the `attributes` module.

use crate::entities::{Entity, EntityHandle, data_field_offset, data_map_class};
use crate::{Game, InterfaceError, Server};
use sdk_raw::weapons::WeaponCreationFailed;
use std::ffi::{CStr, c_int};
use std::ptr::NonNull;

pub trait IntoWeaponSlot {
	fn into_weapon_slot(self) -> c_int;
}

impl IntoWeaponSlot for c_int {
	fn into_weapon_slot(self) -> c_int {
		self
	}
}

/// An item definition in TF2's economy schema. A valid index need not exist in
/// the running server's schema, and can describe a cosmetic instead of a weapon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ItemDefinitionIndex(u16);

impl ItemDefinitionIndex {
	/// Excludes the engine's invalid sentinel, 65535. Index zero is valid.
	pub const fn new(index: u16) -> Option<Self> {
		if index == u16::MAX {
			None
		} else {
			Some(Self(index))
		}
	}

	pub const fn get(self) -> u16 {
		self.0
	}
}

/// A TF2 player's current weapon inventory, scoped to one engine callback.
#[derive(Debug, Clone, Copy)]
pub struct PlayerWeapons<'s> {
	server: Server<'s>,
	player: Entity<'s>,
}

impl<'s> PlayerWeapons<'s> {
	pub fn new(server: Server<'s>, player: Entity<'s>) -> Result<Self, WeaponError> {
		if server.game() != Game::TeamFortress2 || !has_class(player, c"CTFPlayer") {
			return Err(WeaponError::NotTfPlayer);
		}

		Ok(Self { server, player })
	}

	/// Detaches this player's weapon without deleting it. The detached entity
	/// can be equipped again or passed to `ServerTools::remove` for deletion.
	/// This is an inventory operation: native `RemovePlayerItem` can leave
	/// the entity's parenting and attribute-provider association until it is
	/// equipped again or removed. Use `replace` for a complete exchange.
	#[doc(alias = "RemovePlayerItem")]
	pub fn detach(self, weapon: Weapon<'s>) -> Result<(), WeaponError> {
		check_live(self.player)?;

		if weapon.owner()? != Some(self.player.handle()) {
			return Err(WeaponError::DifferentOwner);
		}

		let player = self.player.as_ptr().cast::<sys::CTFPlayer>();

		// SAFETY: `new` verified CTFPlayer's zero-offset primary entity base.
		// The player owns this live weapon. RemovePlayerItem detaches and
		// holsters it without immediately deleting either entity.
		let removed = unsafe {
			let vtable = player
				.cast::<*const sys::CTFPlayer__bindgen_vtable>()
				.read();
			((*vtable).CTFPlayer_RemovePlayerItem)(player, weapon.entity.as_ptr().cast())
		};

		if !removed {
			return Err(WeaponError::Rejected);
		}

		Ok(())
	}

	/// Equips an unowned weapon into an empty native slot. A weapon already
	/// owned and present in this player's inventory is left alone. A matching
	/// owner without inventory membership is reconciled through native equip.
	/// It never steals another player's weapon.
	#[doc(alias = "Equip")]
	pub fn equip(self, weapon: Weapon<'s>) -> Result<(), WeaponError> {
		check_live(self.player)?;
		check_live(weapon.entity)?;

		let owner = weapon.owner()?;

		if owner.is_some_and(|owner| owner != self.player.handle()) {
			return Err(WeaponError::DifferentOwner);
		}

		let slot = weapon.slot()?;

		if let Some(existing) = self.get_slot(slot)? {
			return if existing.entity != weapon.entity {
				Err(WeaponError::SlotOccupied)
			} else if owner == Some(self.player.handle()) {
				Ok(())
			} else {
				Err(WeaponError::Rejected)
			};
		}

		let player = self.player.as_ptr().cast::<sys::CTFPlayer>();
		// SAFETY: Both checked classes have CBaseEntity at primary offset zero.
		// The live weapon is not in an inventory. Native equip updates inventory,
		// ownership, and attribute providers through the generated TF2 vtable.
		unsafe {
			let vtable = player
				.cast::<*const sys::CTFPlayer__bindgen_vtable>()
				.read();
			((*vtable).CTFPlayer_Weapon_Equip)(player, weapon.entity.as_ptr().cast());
		}

		if weapon.owner()? != Some(self.player.handle())
			|| !self
				.get_slot(slot)?
				.is_some_and(|found| found.entity == weapon.entity)
		{
			return Err(WeaponError::Rejected);
		}

		Ok(())
	}

	#[doc(alias = "GetSlot")]
	pub fn get_slot(self, slot: impl IntoWeaponSlot) -> Result<Option<Weapon<'s>>, WeaponError> {
		check_live(self.player)?;

		let player = self.player.as_ptr().cast::<sys::CTFPlayer>();

		// SAFETY: `new` verified the zero-offset CTFPlayer primary base. The
		// generated virtual method scans its own inventory and returns a live
		// weapon or null. The slot is a comparison value.
		let raw = unsafe {
			let vtable = player
				.cast::<*const sys::CTFPlayer__bindgen_vtable>()
				.read();

			((*vtable).CTFPlayer_Weapon_GetSlot)(player, slot.into_weapon_slot())
		};

		NonNull::new(raw)
			.map(|raw| {
				// SAFETY: The player's weapon remains alive through this callback.
				Weapon::new(self.server, unsafe { Entity::from_raw(raw.cast()) })
			})
			.transpose()
	}

	/// Creates a stock economy weapon by its exact entity classname, then equips
	/// it. Native item generation initializes its item definition and models.
	/// Classnames are not translated for the player's class. Existing weapons
	/// are not replaced; use `replace` when its native slot is occupied.
	///
	/// # Safety
	/// The selected weapon's constructor, spawn, pickup and all callbacks they
	/// run must uphold `Server::new`'s no-immediate-deletion contract. As with
	/// `ServerTools::dispatch_spawn`, failed spawning can flush pending deletes.
	#[doc(alias = "GiveNamedItem")]
	pub unsafe fn give(self, classname: &CStr, subtype: i32) -> Result<Weapon<'s>, WeaponError> {
		check_live(self.player)?;

		let player = self.player.as_ptr().cast::<sys::CTFPlayer>();
		// SAFETY: `new` verified the primary CTFPlayer base. This generated
		// overload takes a nullable CEconItemView and a force flag. Null requests
		// native stock-item generation; force keeps the
		// exact classname instead of translating it for the player's class. The
		// caller vouches for the spawn/pickup path.
		let give = unsafe {
			let vtable = player
				.cast::<*const sys::CTFPlayer__bindgen_vtable>()
				.read();
			(*vtable).CTFPlayer_GiveNamedItem1
		};

		// SAFETY: GiveNamedItem returns a newly created callback-live entity.
		unsafe {
			self.give_with(None, || {
				NonNull::new(give(
					player,
					classname.as_ptr(),
					subtype,
					std::ptr::null(),
					true,
				))
				.ok_or(WeaponError::CreationFailed)
			})
		}
	}

	/// Creates and equips a weapon from its economy item definition, with its
	/// schema attributes and models initialized before spawning. Uses Unique
	/// quality and level 1. Existing weapons are not replaced.
	///
	/// Definitions whose schema uses a generic classname such as
	/// `tf_weapon_shotgun` need [`Self::give_item_as`] with a concrete classname.
	/// Missing definitions return `CreationFailed`; cosmetics return `NotWeapon`.
	///
	/// # Safety
	/// The definition's constructor, spawn, activation and equipment callbacks
	/// must uphold `Server::new`'s no-immediate-deletion contract.
	pub unsafe fn give_item(
		self,
		definition: ItemDefinitionIndex,
	) -> Result<Weapon<'s>, WeaponError> {
		// SAFETY: The caller supplies the native creation guarantees.
		unsafe { self._give_item(definition, None) }
	}

	unsafe fn _give_item(
		self,
		definition: ItemDefinitionIndex,
		classname: Option<&CStr>,
	) -> Result<Weapon<'s>, WeaponError> {
		check_live(self.player)?;

		let origin = self.player.position().ok_or(WeaponError::MissingOrigin)?;

		// SAFETY: The caller vouches for the native creation path. The generator
		// initializes CEconItemView before Spawn/Activate and returns a fresh
		// callback-live entity; it must not be passed through DispatchSpawn again.
		unsafe {
			self.give_with(classname, || {
				sdk_raw::weapons::spawn(
					self.server.game_server_factory().as_raw(),
					definition.get(),
					origin.into(),
					classname,
				)
				.map_err(WeaponError::CreationFailedNative)
			})
		}
	}

	/// As [`Self::give_item`], using an exact classname instead of the schema's
	/// classname. For example, a generic shotgun definition can use
	/// `tf_weapon_shotgun_soldier`. The definition's static attributes are retained.
	///
	/// # Safety
	/// The guarantees of `give_item` apply, and the classname must be a weapon
	/// implementation compatible with the chosen item definition.
	pub unsafe fn give_item_as(
		self,
		definition: ItemDefinitionIndex,
		classname: &CStr,
	) -> Result<Weapon<'s>, WeaponError> {
		// SAFETY: The caller supplies the native creation/class guarantees.
		unsafe { self._give_item(definition, Some(classname)) }
	}

	/// `create` must return a newly created entity, live through this callback.
	unsafe fn give_with(
		self,
		expected_classname: Option<&CStr>,
		create: impl FnOnce() -> Result<NonNull<sys::CBaseEntity>, WeaponError>,
	) -> Result<Weapon<'s>, WeaponError> {
		check_live(self.player)?;
		let tools = self.server.server_tools()?;

		// GiveNamedItem can equip before returning. Snapshot before creation so
		// slot iteration order cannot hide a collision after native pickup.
		let mut occupied = [false; 256];

		for (index, occupied) in occupied.iter_mut().enumerate() {
			*occupied = self.get_slot(WeaponSlot(index as u8))?.is_some();
		}

		let raw = create()?;

		// SAFETY: The caller guarantees a newly created callback-live entity.
		let entity = unsafe { Entity::from_raw(raw) };

		check_live(entity)?;

		if expected_classname.is_some_and(|expected| entity.class_name() != expected) {
			// SpawnItem falls back to the schema classname when an override has
			// no factory. Enforce *_as semantics before the fallback can be equipped.
			tools.remove(entity).ok();

			return Err(WeaponError::CreationFailed);
		}

		let weapon = match Weapon::new(self.server, entity) {
			Ok(weapon) => weapon,

			Err(error) => {
				let _ = tools.remove(entity);
				return Err(error);
			}
		};

		let equipped = weapon.slot().and_then(|slot| {
			if occupied[usize::from(slot.0)] {
				Err(WeaponError::SlotOccupied)
			} else {
				self.equip(weapon)
			}
		});

		if let Err(error) = equipped {
			let owner = weapon.owner()?;

			if owner == Some(self.player.handle()) {
				// Native Weapon_Equip sets the combat owner even when all inventory
				// entries are full. In that case RemovePlayerItem cannot find it to
				// detach, but the freshly created entity still needs deletion.
				let _ = self.detach(weapon);
				let _ = tools.remove(entity);
			} else if owner.is_none() {
				let _ = tools.remove(entity);
			}

			return Err(error);
		}

		Ok(weapon)
	}

	pub const fn player(self) -> Entity<'s> {
		self.player
	}

	/// Replaces one inventory slot. The old weapon is detached first so the
	/// game can create another of the same classname, but is deleted only after
	/// successful creation/equipment. Failure attempts to re-equip the old one.
	/// The replacement must use the requested native slot.
	///
	/// # Safety
	/// The same creation and callback contract as `give` applies.
	pub unsafe fn replace(
		self,
		slot: impl IntoWeaponSlot,
		classname: &CStr,
		subtype: i32,
	) -> Result<Weapon<'s>, WeaponError> {
		// SAFETY: The caller supplies the same guarantees as for `give`.
		self.replace_with(slot, || unsafe { self.give(classname, subtype) })
	}

	/// Replaces a slot with an economy item definition. The old weapon is only
	/// deleted after the new weapon is equipped in that slot. Failure attempts
	/// to restore the old weapon.
	/// Uses the same item generation and defaults as [`Self::give_item`].
	///
	/// # Safety
	/// The same native creation/callback guarantees as `give_item` apply.
	pub unsafe fn replace_item(
		self,
		slot: impl IntoWeaponSlot,
		definition: ItemDefinitionIndex,
	) -> Result<Weapon<'s>, WeaponError> {
		// SAFETY: The caller supplies the same guarantees as for `give_item`.
		self.replace_with(slot, || unsafe { self.give_item(definition) })
	}

	/// As [`Self::replace_item`], with the exact classname override described
	/// by [`Self::give_item_as`].
	///
	/// # Safety
	/// The same creation and compatible-classname guarantees as `give_item_as` apply.
	pub unsafe fn replace_item_as(
		self,
		slot: impl IntoWeaponSlot,
		definition: ItemDefinitionIndex,
		classname: &CStr,
	) -> Result<Weapon<'s>, WeaponError> {
		// SAFETY: The caller supplies the same guarantees as for `give_item_as`.
		self.replace_with(slot, || unsafe { self.give_item_as(definition, classname) })
	}

	fn replace_with(
		self,
		slot: impl IntoWeaponSlot,
		create: impl FnOnce() -> Result<Weapon<'s>, WeaponError>,
	) -> Result<Weapon<'s>, WeaponError> {
		let slot = slot.into_weapon_slot();
		let tools = self.server.server_tools()?;
		let old = self.get_slot(slot)?;

		if let Some(old) = old {
			self.detach(old)?;
		}

		let replacement = create().and_then(|weapon| {
			if weapon.slot_raw()? == slot {
				return Ok(weapon);
			}

			let detached = self.detach(weapon);

			// The new weapon must be removed even if native inventory detach
			// refuses it; otherwise a failed exchange leaks the unwanted item.
			let _ = tools.remove(weapon.entity);
			detached?;

			Err(WeaponError::WrongSlot)
		});

		match replacement {
			Ok(weapon) => {
				if let Some(old) = old {
					let _ = tools.remove(old.entity);
				}

				Ok(weapon)
			}

			Err(error) => {
				if let Some(old) = old {
					self.equip(old)?;
				}

				Err(error)
			}
		}
	}
}

/// A callback-scoped TF2 weapon. Keep its entity handle across callbacks.
#[derive(Debug, Clone, Copy)]
pub struct Weapon<'s> {
	entity: Entity<'s>,
	owner_offset: usize,
}

impl<'s> Weapon<'s> {
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, WeaponError> {
		if server.game() != Game::TeamFortress2 || !has_class(entity, c"CTFWeaponBase") {
			return Err(WeaponError::NotWeapon);
		}

		let owner_offset = entity
			.data_maps()
			.find(|map| data_map_class(map) == Some(c"CBaseCombatWeapon"))
			.and_then(|map| data_field_offset(map, c"m_hOwner", sys::_fieldtypes_FIELD_EHANDLE))
			.filter(|offset| *offset < 65_536 && offset.is_multiple_of(align_of::<u32>()))
			.ok_or(WeaponError::UnsupportedLayout)?;

		Ok(Self {
			entity,
			owner_offset,
		})
	}

	pub const fn entity(self) -> Entity<'s> {
		self.entity
	}

	/// Its combat owner's handle (`m_hOwner`), or None for a detached weapon.
	/// The separate base-entity `m_hOwnerEntity` can remain set after detaching
	/// and is not the authority for membership in a player's weapon inventory.
	pub fn owner(self) -> Result<Option<EntityHandle>, WeaponError> {
		check_live(self.entity)?;

		// SAFETY: `new` validated the EHANDLE field in CBaseCombatWeapon's own
		// datamap. Every EHANDLE contains one u32 on both supported ABIs; the
		// callback keeps this entity allocated, and no Rust borrow is formed.
		let raw = unsafe {
			self.entity
				.as_ptr()
				.byte_add(self.owner_offset)
				.cast::<u32>()
				.read()
		};

		let handle = EntityHandle::from_raw(raw);

		Ok(handle.is_valid().then_some(handle))
	}

	/// The native weapon slot, which can depend on its item definition.
	pub fn slot(self) -> Result<WeaponSlot, WeaponError> {
		self.slot_raw()
			.and_then(|slot| WeaponSlot::from_raw(slot).ok_or(WeaponError::WrongSlot))
	}

	/// The native weapon slot, which can depend on its item definition.
	pub fn slot_raw(self) -> Result<c_int, WeaponError> {
		check_live(self.entity)?;

		let weapon = self.entity.as_ptr().cast::<sys::CTFWeaponBase>();

		// SAFETY: `new` established CTFWeaponBase's zero-offset primary entity
		// base. The generated GetSlot entry leaves the weapon alive.
		Ok(unsafe {
			let vtable = weapon
				.cast::<*const sys::CTFWeaponBase__bindgen_vtable>()
				.read();
			((*vtable).CTFWeaponBase_GetSlot)(weapon)
		})
	}
}

#[derive(Debug, thiserror::Error)]
pub enum WeaponError {
	#[error("The game could not create the weapon (including an existing weapon of the same type)")]
	CreationFailed,

	#[error("{0}")]
	CreationFailedNative(#[source] WeaponCreationFailed),

	#[error("The weapon belongs to another player")]
	DifferentOwner,

	#[error(transparent)]
	Interface(#[from] InterfaceError),

	#[error("A weapon classname must start with tf_weapon_ and a subtype must be nonnegative")]
	InvalidRequest,

	#[error("The entity is marked for deletion")]
	MarkedForDeletion,

	#[error("The player's absolute position could not be read")]
	MissingOrigin,

	#[error("Weapon operations require a TF2 player")]
	NotTfPlayer,

	#[error("The entity is not a TF2 combat weapon")]
	NotWeapon,

	#[error("The game refused to detach or equip the weapon")]
	Rejected,

	#[error("The weapon slot is already occupied; use replace to exchange its weapon")]
	SlotOccupied,

	#[error("The weapon's datamap does not describe its combat owner handle")]
	UnsupportedLayout,

	#[error("The new weapon's native slot differs from the requested replacement slot")]
	WrongSlot,
}

impl From<WeaponCreationFailed> for WeaponError {
	fn from(_value: WeaponCreationFailed) -> Self {
		Self::CreationFailed
	}
}

/// A weapon inventory slot (not an item-schema loadout position).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WeaponSlot(pub u8);

impl WeaponSlot {
	pub const BUILDING: Self = Self(5);
	pub const MELEE: Self = Self(2);
	pub const PDA: Self = Self(3);
	pub const PDA2: Self = Self(4);
	pub const PRIMARY: Self = Self(0);
	pub const SECONDARY: Self = Self(1);

	pub fn from_raw(raw: c_int) -> Option<Self> {
		u8::try_from(raw).ok().map(Self)

		// match raw {
		//
		// }
	}
}

impl IntoWeaponSlot for WeaponSlot {
	fn into_weapon_slot(self) -> c_int {
		self.0 as c_int
	}
}

fn check_live(entity: Entity<'_>) -> Result<(), WeaponError> {
	if entity.is_marked_for_deletion() {
		Err(WeaponError::MarkedForDeletion)
	} else {
		Ok(())
	}
}

fn has_class(entity: Entity<'_>, class: &CStr) -> bool {
	entity
		.data_maps()
		.any(|map| data_map_class(map) == Some(class))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::InterfaceFactory;
	use crate::entities::test_support::{base_entity_fields, data_map, field};
	use crate::ffi::test_support::{mock_vtable, unexpected_call};
	use std::cell::Cell;
	use std::ffi::{c_char, c_void};
	use std::mem::{offset_of, size_of};
	use std::ptr::null_mut;

	const EQUIP: usize =
		offset_of!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_Weapon_Equip) / size_of::<usize>();

	const GET_SLOT: usize =
		offset_of!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_Weapon_GetSlot) / size_of::<usize>();

	const GIVE: usize =
		offset_of!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_GiveNamedItem1) / size_of::<usize>();

	const REMOVE: usize =
		offset_of!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_RemovePlayerItem) / size_of::<usize>();

	const WEAPON_SLOT: usize =
		offset_of!(sys::CTFWeaponBase__bindgen_vtable, CTFWeaponBase_GetSlot) / size_of::<usize>();

	#[repr(C)]
	struct FakeEntity {
		vtable: *const *const (),
		map: *mut sys::datamap_t,
		weapon: *mut sys::CBaseEntity,
		slot: i32,
		padding: i32,
		flags: i32,
		owner: u32,
		handle: u32,
		owner_entity: u32,
		second_weapon: *mut sys::CBaseEntity,
		equip_calls: usize,
	}

	thread_local! {
		static TOOLS: Cell<*mut c_void> = const { Cell::new(null_mut()) };
		static GIVE_RESULT: Cell<*mut sys::CBaseEntity> = const { Cell::new(null_mut()) };
		static INVENTORY_FULL: Cell<bool> = const { Cell::new(false) };
		static REJECT_DETACH: Cell<bool> = const { Cell::new(false) };
		static NETWORKABLE: Cell<*mut sys::IServerNetworkable> = const { Cell::new(null_mut()) };
	}

	unsafe extern "C" fn classname(_: *const sys::IServerNetworkable) -> *const c_char {
		c"tf_weapon_bottle".as_ptr()
	}

	unsafe extern "C" fn datamap(entity: *mut sys::CBaseEntity) -> *mut sys::datamap_t {
		unsafe { (*entity.cast::<FakeEntity>()).map }
	}

	unsafe extern "C" fn detach(
		player: *mut sys::CTFPlayer,
		weapon: *mut sys::CBaseCombatWeapon,
	) -> bool {
		if REJECT_DETACH.get() {
			return false;
		}
		unsafe {
			let player = player.cast::<FakeEntity>();
			let weapon = weapon.cast::<sys::CBaseEntity>();
			if (*player).weapon == weapon {
				(*player).weapon = null_mut();
			} else if (*player).second_weapon == weapon {
				(*player).second_weapon = null_mut();
			} else {
				return false;
			}
			// Matches native Weapon_Detach: m_hOwnerEntity deliberately stays set.
			(*weapon.cast::<FakeEntity>()).owner = EntityHandle::INVALID.to_raw();
			true
		}
	}

	unsafe extern "C" fn equip(player: *mut sys::CTFPlayer, weapon: *mut sys::CBaseCombatWeapon) {
		unsafe {
			let player = player.cast::<FakeEntity>();
			let weapon = weapon.cast::<sys::CBaseEntity>();
			(*player).equip_calls += 1;
			if !INVENTORY_FULL.get() {
				if (*player).weapon.is_null() {
					(*player).weapon = weapon;
				} else {
					(*player).second_weapon = weapon;
				}
			}
			(*weapon.cast::<FakeEntity>()).owner = (*player).handle;
			(*weapon.cast::<FakeEntity>()).owner_entity = (*player).handle;
		}
	}

	unsafe extern "C" fn factory(name: *const c_char, _: *mut i32) -> *mut c_void {
		if unsafe { CStr::from_ptr(name) } == c"VSERVERTOOLS003" {
			TOOLS.get()
		} else {
			null_mut()
		}
	}

	#[test]
	fn failed_replacement_restores_inventory_and_rejected_pickup_does_not_leak_weapon() {
		let base = data_map(c"CBaseEntity", Vec::from(base_entity_fields()), null_mut());
		let player_map = data_map(c"CTFPlayer", vec![], base);
		let weapon_map = weapon_map(base);
		let mut player_table = vec![std::ptr::null(); GIVE + 1];
		player_table[sys::CBASEENTITY_DATAMAP_VTABLE_SLOT] = datamap as *const ();
		player_table[GET_SLOT] = inventory_slot as *const ();
		player_table[GIVE] = give as *const ();
		player_table[EQUIP] = equip as *const ();
		player_table[REMOVE] = detach as *const ();
		let handle_slot = offset_of!(
			sys::IServerUnknown__bindgen_vtable,
			IServerUnknown_GetRefEHandle
		) / size_of::<usize>();
		player_table[handle_slot] = handle as *const ();
		let mut weapon_table = vec![std::ptr::null(); WEAPON_SLOT + 1];
		weapon_table[sys::CBASEENTITY_DATAMAP_VTABLE_SLOT] = datamap as *const ();
		weapon_table[WEAPON_SLOT] = weapon_slot as *const ();
		weapon_table[handle_slot] = handle as *const ();
		let networkable_slot = offset_of!(
			sys::IServerUnknown__bindgen_vtable,
			IServerUnknown_GetNetworkable
		) / size_of::<usize>();
		weapon_table[networkable_slot] = networkable as *const ();
		let networkable_vtable = unsafe {
			mock_vtable::<sys::IServerNetworkable__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IServerNetworkable_GetClassName).write(classname);
				},
			)
		};
		let mut networkable = sys::IServerNetworkable {
			vtable_: &*networkable_vtable,
		};
		NETWORKABLE.set(&raw mut networkable);

		let mut old = FakeEntity {
			vtable: weapon_table.as_ptr(),
			map: weapon_map,
			weapon: null_mut(),
			slot: 2,
			padding: 0,
			flags: 0,
			owner: 1,
			handle: 2,
			owner_entity: 1,
			second_weapon: null_mut(),
			equip_calls: 0,
		};

		let old_ptr = (&raw mut old).cast();
		let mut fresh = FakeEntity {
			vtable: weapon_table.as_ptr(),
			map: weapon_map,
			weapon: null_mut(),
			slot: 2,
			padding: 0,
			flags: 0,
			owner: EntityHandle::INVALID.to_raw(),
			handle: 3,
			owner_entity: 0,
			second_weapon: null_mut(),
			equip_calls: 0,
		};
		let mut player = FakeEntity {
			vtable: player_table.as_ptr(),
			map: player_map,
			weapon: old_ptr,
			slot: 2,
			padding: 0,
			flags: 0,
			owner: 0,
			handle: 1,
			owner_entity: 0,
			second_weapon: null_mut(),
			equip_calls: 0,
		};
		let tools_vtable = unsafe {
			mock_vtable::<sys::IServerTools__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IServerTools_RemoveEntity).write(remove);
				},
			)
		};
		let mut tools = sys::IServerTools {
			vtable_: &*tools_vtable,
		};
		TOOLS.set((&raw mut tools).cast());
		GIVE_RESULT.set(null_mut());
		let scope = ();
		let factory = InterfaceFactory::new(factory);
		let server = unsafe { Server::new(factory, factory, Game::TeamFortress2, &scope) };
		let entity = unsafe { Entity::from_raw(NonNull::from(&mut player).cast()) };
		let inventory = PlayerWeapons::new(server, entity).unwrap();
		let weapon = inventory.get_slot(WeaponSlot::MELEE).unwrap().unwrap();
		inventory.detach(weapon).unwrap();
		assert_eq!(weapon.owner().unwrap(), None);
		assert_eq!(
			old.owner_entity, 1,
			"native detach leaves the unrelated base owner intact"
		);
		assert!(inventory.get_slot(WeaponSlot::MELEE).unwrap().is_none());
		inventory.equip(weapon).unwrap();
		assert_eq!(
			inventory
				.get_slot(WeaponSlot::MELEE)
				.unwrap()
				.unwrap()
				.entity
				.as_ptr(),
			old_ptr
		);
		assert_eq!(player.equip_calls, 1);
		// Native creation failure must restore the previously detached weapon.
		assert!(matches!(
			unsafe { inventory.replace(WeaponSlot::MELEE, c"tf_weapon_missing", 0) },
			Err(WeaponError::CreationFailed)
		));
		assert_eq!(
			inventory
				.get_slot(WeaponSlot::MELEE)
				.unwrap()
				.unwrap()
				.entity
				.as_ptr(),
			old_ptr
		);
		assert_eq!(weapon.owner().unwrap(), Some(entity.handle()));
		assert_eq!(player.equip_calls, 2);
		assert_eq!(old.flags, 0);
		// Matching owner alone cannot short-circuit reconciliation.
		unsafe { (&raw mut player.weapon).write(null_mut()) };
		inventory.equip(weapon).unwrap();
		assert_eq!(
			inventory
				.get_slot(WeaponSlot::MELEE)
				.unwrap()
				.unwrap()
				.entity
				.as_ptr(),
			old_ptr
		);
		assert_eq!(player.equip_calls, 3);

		// Touch equips the fresh weapon before GiveNamedItem returns, and the
		// native slot getter even returns it first. The snapshot must catch it.
		GIVE_RESULT.set((&raw mut fresh).cast());
		assert!(matches!(
			unsafe { inventory.give(c"tf_weapon_bottle", 0) },
			Err(WeaponError::SlotOccupied)
		));
		assert_eq!(
			inventory
				.get_slot(WeaponSlot::MELEE)
				.unwrap()
				.unwrap()
				.entity
				.as_ptr(),
			old_ptr
		);
		assert!(player.second_weapon.is_null());
		assert_eq!(fresh.owner, EntityHandle::INVALID.to_raw());
		assert_eq!(fresh.flags, 1, "the rejected new entity must be removed");
		assert_eq!(old.flags, 0);

		// A full native inventory can set ownership without recording the new
		// weapon. Detach then fails, but cleanup must still delete that entity.
		unsafe {
			(&raw mut player.weapon).write(null_mut());
			(&raw mut fresh.flags).write(0);
		}
		INVENTORY_FULL.set(true);
		assert!(matches!(
			unsafe { inventory.give(c"tf_weapon_bottle", 0) },
			Err(WeaponError::Rejected)
		));
		assert!(inventory.get_slot(WeaponSlot::MELEE).unwrap().is_none());
		assert_eq!(
			fresh.flags, 1,
			"failed native detach must not leak the new entity"
		);
		INVENTORY_FULL.set(false);

		// Item generation returns a spawned, unequipped entity. Exercise that
		// path independently of stock GiveNamedItem's implicit Touch pickup.
		unsafe {
			(&raw mut player.weapon).write(old_ptr);
			(&raw mut fresh.flags).write(0);
			(&raw mut fresh.owner).write(EntityHandle::INVALID.to_raw());
		}
		let fresh_ptr = NonNull::from(&mut fresh).cast();
		assert!(matches!(
			unsafe { inventory.give_with(None, || Ok(fresh_ptr)) },
			Err(WeaponError::SlotOccupied)
		));
		assert_eq!(player.weapon, old_ptr);
		assert_eq!(fresh.flags, 1);
		assert_eq!(old.flags, 0);

		assert!(matches!(
			inventory.replace_with(WeaponSlot::MELEE, || Err(WeaponError::CreationFailed)),
			Err(WeaponError::CreationFailed)
		));
		assert_eq!(
			player.weapon, old_ptr,
			"missing item definition restores the old weapon"
		);

		unsafe {
			(&raw mut fresh.flags).write(0);
			(&raw mut fresh.slot).write(0);
		}
		assert!(matches!(
			inventory.replace_with(WeaponSlot::MELEE, || unsafe {
				inventory.give_with(None, || Ok(fresh_ptr))
			}),
			Err(WeaponError::WrongSlot)
		));
		assert_eq!(
			player.weapon, old_ptr,
			"wrong item slot restores the old weapon"
		);
		assert_eq!(fresh.flags, 1, "wrong-slot item is removed");
		assert_eq!(fresh.owner, EntityHandle::INVALID.to_raw());

		unsafe {
			(&raw mut fresh.flags).write(0);
		}
		let rejected_detach = inventory.replace_with(WeaponSlot::MELEE, || {
			let replacement = unsafe { inventory.give_with(None, || Ok(fresh_ptr)) }?;
			REJECT_DETACH.set(true);
			Ok(replacement)
		});
		REJECT_DETACH.set(false);
		assert!(matches!(rejected_detach, Err(WeaponError::Rejected)));
		assert_eq!(
			fresh.flags, 1,
			"wrong-slot item is removed even when detach rejects"
		);
		assert_eq!(
			inventory
				.get_slot(WeaponSlot::MELEE)
				.unwrap()
				.unwrap()
				.entity()
				.as_ptr(),
			old_ptr
		);

		unsafe {
			(&raw mut player.weapon).write(old_ptr);
			(&raw mut player.second_weapon).write(null_mut());
			(&raw mut fresh.flags).write(0);
			(&raw mut fresh.owner).write(EntityHandle::INVALID.to_raw());
			(&raw mut fresh.map).write(data_map(c"CEconWearable", vec![], base));
		}
		assert!(matches!(
			inventory.replace_with(WeaponSlot::MELEE, || unsafe {
				inventory.give_with(None, || Ok(fresh_ptr))
			}),
			Err(WeaponError::NotWeapon)
		));
		assert_eq!(
			player.weapon, old_ptr,
			"cosmetic item restores the old weapon"
		);
		assert_eq!(fresh.flags, 1, "nonweapon item is removed");
		assert_eq!(old.flags, 0);

		unsafe {
			(&raw mut fresh.flags).write(0);
			(&raw mut fresh.map).write(weapon_map);
			(&raw mut fresh.slot).write(2);
		}
		assert!(matches!(
			inventory.replace_with(WeaponSlot::MELEE, || unsafe {
				inventory.give_with(Some(c"tf_weapon_sdk_missing"), || Ok(fresh_ptr))
			}),
			Err(WeaponError::CreationFailed)
		));
		assert_eq!(fresh.flags, 1, "native classname fallback must be removed");
		assert_eq!(
			player.weapon, old_ptr,
			"override mismatch restores the old weapon"
		);
		unsafe {
			(&raw mut fresh.flags).write(0);
		}
		let replacement = inventory
			.replace_with(WeaponSlot::MELEE, || unsafe {
				inventory.give_with(Some(c"tf_weapon_bottle"), || Ok(fresh_ptr))
			})
			.unwrap();
		assert_eq!(replacement.entity().as_ptr(), fresh_ptr.as_ptr());
		assert_eq!(player.weapon, fresh_ptr.as_ptr());
		assert_eq!(fresh.owner, 1);
		assert_eq!(fresh.flags, 0);
		assert_eq!(
			old.flags, 1,
			"successful economy replacement removes the old weapon"
		);
		TOOLS.set(null_mut());
		GIVE_RESULT.set(null_mut());
		NETWORKABLE.set(null_mut());
	}

	unsafe extern "C" fn give(
		player: *mut sys::CTFPlayer,
		_: *const c_char,
		_: i32,
		item: *const sys::CEconItemView,
		force: bool,
	) -> *mut sys::CBaseEntity {
		assert!(
			item.is_null(),
			"stock generation requires a null CEconItemView"
		);
		assert!(force, "the requested classname must not be translated");
		let weapon = GIVE_RESULT.get();
		if !weapon.is_null() {
			unsafe { equip(player, weapon.cast()) };
		}
		weapon
	}

	unsafe extern "C" fn handle(entity: *const sys::IServerUnknown) -> *const sys::CBaseHandle {
		unsafe { (&raw const (*entity.cast::<FakeEntity>()).handle).cast() }
	}

	unsafe extern "C" fn inventory_slot(
		entity: *const sys::CTFPlayer,
		slot: i32,
	) -> *mut sys::CBaseCombatWeapon {
		unsafe {
			let entity = entity.cast::<FakeEntity>();
			for weapon in [(*entity).second_weapon, (*entity).weapon] {
				if !weapon.is_null() && (*weapon.cast::<FakeEntity>()).slot == slot {
					return weapon.cast();
				}
			}
			null_mut()
		}
	}

	#[test]
	fn native_inventory_slots_use_validated_classes_and_refuse_deleted_entities() {
		assert_eq!(
			std::mem::offset_of!(FakeEntity, flags),
			crate::entities::test_support::MOCK_EFLAGS_OFFSET
		);
		let base = data_map(c"CBaseEntity", Vec::from(base_entity_fields()), null_mut());
		let player_map = data_map(c"CTFPlayer", vec![], base);
		let weapon_map = weapon_map(base);
		let mut player_table = vec![std::ptr::null(); GIVE + 1];
		player_table[sys::CBASEENTITY_DATAMAP_VTABLE_SLOT] = datamap as *const ();
		player_table[GET_SLOT] = inventory_slot as *const ();
		let mut weapon_table = vec![std::ptr::null(); WEAPON_SLOT + 1];
		weapon_table[sys::CBASEENTITY_DATAMAP_VTABLE_SLOT] = datamap as *const ();
		weapon_table[WEAPON_SLOT] = weapon_slot as *const ();
		let mut weapon = FakeEntity {
			vtable: weapon_table.as_ptr(),
			map: weapon_map,
			weapon: null_mut(),
			slot: 2,
			padding: 0,
			flags: 0,
			owner: 1,
			handle: 2,
			owner_entity: 1,
			second_weapon: null_mut(),
			equip_calls: 0,
		};
		let raw_weapon = (&raw mut weapon).cast();
		let mut player = FakeEntity {
			vtable: player_table.as_ptr(),
			map: player_map,
			weapon: raw_weapon,
			slot: 2,
			padding: 0,
			flags: 0,
			owner: 0,
			handle: 1,
			owner_entity: 0,
			second_weapon: null_mut(),
			equip_calls: 0,
		};
		let scope = ();
		let factory = InterfaceFactory::new(factory);
		let server = unsafe { Server::new(factory, factory, Game::TeamFortress2, &scope) };
		let entity = unsafe { Entity::from_raw(NonNull::from(&mut player).cast()) };
		let inventory = PlayerWeapons::new(server, entity).unwrap();
		let found = inventory.get_slot(WeaponSlot::MELEE).unwrap().unwrap();
		assert_eq!(found.entity.as_ptr(), raw_weapon);
		assert_eq!(found.slot().unwrap(), WeaponSlot::MELEE);
		assert!(inventory.get_slot(WeaponSlot::PRIMARY).unwrap().is_none());
		assert!(matches!(
			PlayerWeapons::new(server, found.entity()),
			Err(WeaponError::NotTfPlayer)
		));
		assert!(matches!(
			Weapon::new(server, entity),
			Err(WeaponError::NotWeapon)
		));
		unsafe { (&raw mut player.flags).write(1) };
		assert!(matches!(
			inventory.get_slot(WeaponSlot::MELEE),
			Err(WeaponError::MarkedForDeletion)
		));
	}

	unsafe extern "C" fn networkable(_: *mut sys::IServerUnknown) -> *mut sys::IServerNetworkable {
		NETWORKABLE.get()
	}

	unsafe extern "C" fn remove(_: *mut sys::IServerTools, entity: *mut sys::CBaseEntity) {
		unsafe {
			(*entity.cast::<FakeEntity>()).flags |= 1;
		}
	}

	fn weapon_map(base: *mut sys::datamap_t) -> *mut sys::datamap_t {
		let mut owner = field();
		owner.fieldName = c"m_hOwner".as_ptr();
		owner.fieldType = sys::_fieldtypes_FIELD_EHANDLE;
		owner.fieldOffset[0] = offset_of!(FakeEntity, owner) as i32;
		owner.fieldSize = 1;
		owner.fieldSizeInBytes = 4;
		let combat = data_map(c"CBaseCombatWeapon", vec![owner], base);

		data_map(c"CTFWeaponBase", vec![], combat)
	}

	unsafe extern "C" fn weapon_slot(entity: *const sys::CTFWeaponBase) -> i32 {
		unsafe { (*entity.cast::<FakeEntity>()).slot }
	}
}
