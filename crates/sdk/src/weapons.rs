//! TF2 weapon creation, inventory slots, equipment, and replacement.
//!
//! `give` uses TF2's native item generation to create a stock economy weapon
//! by classname, including its item definition and models. Attributes can be
//! changed through the `attributes` module.

use crate::entities::{Entity, EntityHandle, data_field_offset, data_map_class};
use crate::{Game, InterfaceError, Server};
use std::ffi::{CStr, c_char, c_void};
use std::mem::transmute;
use std::ptr::NonNull;

// SourceMod gamedata/sdktools.games/game.tf.txt (windows64/linux64).
// Bravo's server.dll primary CTFPlayer RTTI vtable confirms these Windows
// slots: Weapon_Equip=272, Weapon_GetSlot=279, RemovePlayerItem=281. Its
// Weapon_GetSlot calls weapon GetSlot at 334. TF2's CEconItemView overload
// of GiveNamedItem is slot 487 (server.dll VA 1805e5450); its null-item path
// generates a base economy item before spawning it. CBasePlayer's separate
// three-argument overload at 413 does not initialize the economy item.
// Linux server_srv.so's _ZTV9CTFPlayer places
// _ZN9CTFPlayer13GiveNamedItemEPKciPK13CEconItemViewb at 494; the later
// TF-specific slots do not share the one-slot base-class ABI difference.
// _ZTV13CTFWeaponBase places _ZNK17CBaseCombatWeapon7GetSlotEv at 340.
const ABI_SHIFT: usize = if cfg!(target_os = "linux") { 1 } else { 0 };
const GIVE: usize = if cfg!(target_os = "linux") { 494 } else { 487 };
const EQUIP: usize = 272 + ABI_SHIFT;
const GET_SLOT: usize = 279 + ABI_SHIFT;
const REMOVE: usize = 281 + ABI_SHIFT;
const WEAPON_SLOT: usize = if cfg!(target_os = "linux") { 340 } else { 334 };

/// A weapon inventory slot (not an item-schema loadout position).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WeaponSlot(pub u8);

impl WeaponSlot {
	pub const PRIMARY: Self = Self(0);
	pub const SECONDARY: Self = Self(1);
	pub const MELEE: Self = Self(2);
	pub const PDA: Self = Self(3);
	pub const PDA2: Self = Self(4);
	pub const BUILDING: Self = Self(5);
}

#[derive(Debug, thiserror::Error)]
pub enum WeaponError {
	#[error("weapon operations require a TF2 player")]
	NotTfPlayer,
	#[error("the entity is not a TF2 combat weapon")]
	NotWeapon,
	#[error("the entity is marked for deletion")]
	MarkedForDeletion,
	#[error("a weapon classname must start with tf_weapon_ and a subtype must be nonnegative")]
	InvalidRequest,
	#[error("the game could not create the weapon (including an existing weapon of the same type)")]
	CreationFailed,
	#[error("the weapon belongs to another player")]
	DifferentOwner,
	#[error("the weapon slot is already occupied; use replace to exchange its weapon")]
	SlotOccupied,
	#[error("the new weapon's native slot differs from the requested replacement slot")]
	WrongSlot,
	#[error("the game refused to detach or equip the weapon")]
	Rejected,
	#[error("the weapon's datamap does not describe its combat owner handle")]
	UnsupportedLayout,
	#[error(transparent)]
	Interface(#[from] InterfaceError),
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

	/// The native weapon slot, which can depend on its item definition.
	pub fn slot(self) -> Result<WeaponSlot, WeaponError> {
		check_live(self.entity)?;
		type GetSlot = unsafe extern "C" fn(*mut sys::CBaseEntity) -> i32;
		// SAFETY: `new` established the CTFWeaponBase ancestry; the native
		// GetSlot entry returns an integer and leaves the weapon alive.
		let get: GetSlot = unsafe { transmute(vslot(self.entity, WEAPON_SLOT)) };
		let index = unsafe { get(self.entity.as_ptr()) };
		u8::try_from(index)
			.map(WeaponSlot)
			.map_err(|_| WeaponError::WrongSlot)
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

	pub const fn player(self) -> Entity<'s> {
		self.player
	}

	pub fn slot(self, slot: WeaponSlot) -> Result<Option<Weapon<'s>>, WeaponError> {
		check_live(self.player)?;
		type GetSlot = unsafe extern "C" fn(*mut sys::CBaseEntity, i32) -> *mut sys::CBaseEntity;
		// SAFETY: The verified CTFPlayer virtual method scans its own inventory
		// and returns a live weapon or null. The slot is a comparison value.
		let get: GetSlot = unsafe { transmute(vslot(self.player, GET_SLOT)) };
		let raw = unsafe { get(self.player.as_ptr(), i32::from(slot.0)) };
		NonNull::new(raw)
			.map(|raw| {
				// SAFETY: The player's weapon remains alive through this callback.
				Weapon::new(self.server, unsafe { Entity::from_raw(raw) })
			})
			.transpose()
	}

	/// Equips an unowned weapon into an empty native slot. A weapon already
	/// owned and present in this player's inventory is left alone. A matching
	/// owner without inventory membership is reconciled through native equip.
	/// It never steals another player's weapon.
	pub fn equip(self, weapon: Weapon<'s>) -> Result<(), WeaponError> {
		check_live(self.player)?;
		check_live(weapon.entity)?;
		let owner = weapon.owner()?;
		if owner.is_some_and(|owner| owner != self.player.handle()) {
			return Err(WeaponError::DifferentOwner);
		}
		let slot = weapon.slot()?;
		if let Some(existing) = self.slot(slot)? {
			return if existing.entity != weapon.entity {
				Err(WeaponError::SlotOccupied)
			} else if owner == Some(self.player.handle()) {
				Ok(())
			} else {
				Err(WeaponError::Rejected)
			};
		}
		type Equip = unsafe extern "C" fn(*mut sys::CBaseEntity, *mut sys::CBaseEntity);
		// SAFETY: Both checked objects are live and the weapon is not in an inventory. The
		// native method updates inventory, ownership, and attribute providers.
		let equip: Equip = unsafe { transmute(vslot(self.player, EQUIP)) };
		unsafe { equip(self.player.as_ptr(), weapon.entity.as_ptr()) };
		if weapon.owner()? != Some(self.player.handle())
			|| !self
				.slot(slot)?
				.is_some_and(|found| found.entity == weapon.entity)
		{
			return Err(WeaponError::Rejected);
		}
		Ok(())
	}

	/// Detaches this player's weapon without deleting it. The detached entity
	/// can be equipped again or passed to `ServerTools::remove` for deletion.
	/// This is an inventory operation: native `RemovePlayerItem` can leave
	/// the entity's parenting and attribute-provider association until it is
	/// equipped again or removed. Use `replace` for a complete exchange.
	pub fn detach(self, weapon: Weapon<'s>) -> Result<(), WeaponError> {
		check_live(self.player)?;
		if weapon.owner()? != Some(self.player.handle()) {
			return Err(WeaponError::DifferentOwner);
		}
		type Remove = unsafe extern "C" fn(*mut sys::CBaseEntity, *mut sys::CBaseEntity) -> bool;
		// SAFETY: The player owns this live weapon. RemovePlayerItem detaches
		// and holsters it; it does not immediately delete either entity.
		let remove: Remove = unsafe { transmute(vslot(self.player, REMOVE)) };
		if !unsafe { remove(self.player.as_ptr(), weapon.entity.as_ptr()) } {
			return Err(WeaponError::Rejected);
		}
		Ok(())
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
	pub unsafe fn give(self, classname: &CStr, subtype: i32) -> Result<Weapon<'s>, WeaponError> {
		check_live(self.player)?;
		validate_request(classname, subtype)?;
		let tools = self.server.server_tools()?;
		// GiveNamedItem runs Touch, which can equip a different classname into
		// an occupied slot before returning. Snapshot first so slot iteration
		// order cannot hide that collision after the pickup.
		let mut occupied = [false; 256];
		for (index, occupied) in occupied.iter_mut().enumerate() {
			*occupied = self.slot(WeaponSlot(index as u8))?.is_some();
		}
		type Give = unsafe extern "C" fn(
			*mut sys::CBaseEntity,
			*const c_char,
			i32,
			*const c_void,
			bool,
		) -> *mut sys::CBaseEntity;
		// SAFETY: This CTFPlayer overload takes a nullable CEconItemView and a
		// force flag. Null requests native stock-item generation; force keeps the
		// exact classname instead of translating it for the player's class. The
		// caller vouches for the spawn/pickup path.
		let give: Give = unsafe { transmute(vslot(self.player, GIVE)) };
		let raw = unsafe {
			give(
				self.player.as_ptr(),
				classname.as_ptr(),
				subtype,
				std::ptr::null(),
				true,
			)
		};
		let raw = NonNull::new(raw).ok_or(WeaponError::CreationFailed)?;
		// SAFETY: The native method returned the spawned entity; the caller's
		// contract guarantees it remains allocated for the callback.
		let entity = unsafe { Entity::from_raw(raw) };
		check_live(entity)?;
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

	/// Replaces one inventory slot. The old weapon is detached first so the
	/// game can create another of the same classname, but is deleted only after
	/// successful creation/equipment. Failure attempts to re-equip the old one.
	/// The replacement must use the requested native slot.
	///
	/// # Safety
	/// The same creation and callback contract as `give` applies.
	pub unsafe fn replace(
		self,
		slot: WeaponSlot,
		classname: &CStr,
		subtype: i32,
	) -> Result<Weapon<'s>, WeaponError> {
		validate_request(classname, subtype)?;
		let tools = self.server.server_tools()?;
		let old = self.slot(slot)?;
		if let Some(old) = old {
			self.detach(old)?;
		}
		// SAFETY: The caller supplies the same guarantees as for `give`.
		let replacement = unsafe { self.give(classname, subtype) }.and_then(|weapon| {
			if weapon.slot()? == slot {
				return Ok(weapon);
			}
			self.detach(weapon)?;
			let _ = tools.remove(weapon.entity);
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

fn validate_request(classname: &CStr, subtype: i32) -> Result<(), WeaponError> {
	let name = classname.to_bytes();
	if !name.starts_with(b"tf_weapon_")
		|| name.len() <= 10
		|| !name
			.iter()
			.all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
		|| subtype < 0
	{
		return Err(WeaponError::InvalidRequest);
	}
	Ok(())
}

fn has_class(entity: Entity<'_>, class: &CStr) -> bool {
	entity
		.data_maps()
		.any(|map| data_map_class(map) == Some(class))
}

fn check_live(entity: Entity<'_>) -> Result<(), WeaponError> {
	if entity.is_marked_for_deletion() {
		Err(WeaponError::MarkedForDeletion)
	} else {
		Ok(())
	}
}

/// # Safety
/// `slot` must be present in this entity's actual primary vtable.
unsafe fn vslot(entity: Entity<'_>, slot: usize) -> *const () {
	// SAFETY: CBaseEntity is the zero-offset primary base; callers have checked
	// the relevant TF2 class and pass slots confirmed for its native vtable.
	unsafe {
		entity
			.as_ptr()
			.cast::<*const *const ()>()
			.read()
			.add(slot)
			.read()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::InterfaceFactory;
	use crate::entities::test_support::{base_entity_fields, data_map, field};
	use crate::ffi::test_support::{mock_vtable, unexpected_call};
	use std::cell::Cell;
	use std::ffi::c_void;
	use std::mem::{offset_of, size_of};
	use std::ptr::null_mut;

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
	}

	unsafe extern "C" fn factory(name: *const c_char, _: *mut i32) -> *mut c_void {
		if unsafe { CStr::from_ptr(name) } == c"VSERVERTOOLS003" {
			TOOLS.get()
		} else {
			null_mut()
		}
	}
	unsafe extern "C" fn handle(entity: *const sys::IServerUnknown) -> *const sys::CBaseHandle {
		unsafe { (&raw const (*entity.cast::<FakeEntity>()).handle).cast() }
	}
	unsafe extern "C" fn datamap(entity: *mut sys::CBaseEntity) -> *mut sys::datamap_t {
		unsafe { (*entity.cast::<FakeEntity>()).map }
	}
	unsafe extern "C" fn inventory_slot(
		entity: *mut sys::CBaseEntity,
		slot: i32,
	) -> *mut sys::CBaseEntity {
		unsafe {
			let entity = entity.cast::<FakeEntity>();
			for weapon in [(*entity).second_weapon, (*entity).weapon] {
				if !weapon.is_null() && (*weapon.cast::<FakeEntity>()).slot == slot {
					return weapon;
				}
			}
			null_mut()
		}
	}
	unsafe extern "C" fn weapon_slot(entity: *mut sys::CBaseEntity) -> i32 {
		unsafe { (*entity.cast::<FakeEntity>()).slot }
	}

	unsafe extern "C" fn equip(player: *mut sys::CBaseEntity, weapon: *mut sys::CBaseEntity) {
		unsafe {
			let player = player.cast::<FakeEntity>();
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
	unsafe extern "C" fn detach(
		player: *mut sys::CBaseEntity,
		weapon: *mut sys::CBaseEntity,
	) -> bool {
		unsafe {
			let player = player.cast::<FakeEntity>();
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
	unsafe extern "C" fn give(
		player: *mut sys::CBaseEntity,
		_: *const c_char,
		_: i32,
		item: *const c_void,
		force: bool,
	) -> *mut sys::CBaseEntity {
		assert!(
			item.is_null(),
			"stock generation requires a null CEconItemView"
		);
		assert!(force, "the requested classname must not be translated");
		let weapon = GIVE_RESULT.get();
		if !weapon.is_null() {
			unsafe { equip(player, weapon) };
		}
		weapon
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
		let found = inventory.slot(WeaponSlot::MELEE).unwrap().unwrap();
		assert_eq!(found.entity.as_ptr(), raw_weapon);
		assert_eq!(found.slot().unwrap(), WeaponSlot::MELEE);
		assert!(inventory.slot(WeaponSlot::PRIMARY).unwrap().is_none());
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
			inventory.slot(WeaponSlot::MELEE),
			Err(WeaponError::MarkedForDeletion)
		));
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
		let weapon = inventory.slot(WeaponSlot::MELEE).unwrap().unwrap();
		inventory.detach(weapon).unwrap();
		assert_eq!(weapon.owner().unwrap(), None);
		assert_eq!(
			old.owner_entity, 1,
			"native detach leaves the unrelated base owner intact"
		);
		assert!(inventory.slot(WeaponSlot::MELEE).unwrap().is_none());
		inventory.equip(weapon).unwrap();
		assert_eq!(
			inventory
				.slot(WeaponSlot::MELEE)
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
				.slot(WeaponSlot::MELEE)
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
				.slot(WeaponSlot::MELEE)
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
				.slot(WeaponSlot::MELEE)
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
		assert!(inventory.slot(WeaponSlot::MELEE).unwrap().is_none());
		assert_eq!(
			fresh.flags, 1,
			"failed native detach must not leak the new entity"
		);
		INVENTORY_FULL.set(false);
		TOOLS.set(null_mut());
		GIVE_RESULT.set(null_mut());
	}

	#[test]
	fn weapon_creation_rejects_nonweapon_entities_and_negative_subtypes() {
		for classname in [
			c"point_servercommand",
			c"tf_weapon_",
			c"tf_weapon_rocketlauncher;quit",
			c"",
		] {
			assert!(matches!(
				validate_request(classname, 0),
				Err(WeaponError::InvalidRequest)
			));
		}
		assert!(matches!(
			validate_request(c"tf_weapon_rocketlauncher", -1),
			Err(WeaponError::InvalidRequest)
		));
		assert!(validate_request(c"tf_weapon_rocketlauncher", 0).is_ok());
	}
}
