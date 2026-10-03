//! TF2 weapon creation, inventory slots, equipment, and replacement.
//!
//! [`PlayerWeapons::give`] creates a stock weapon by classname;
//! [`PlayerWeapons::give_item`] selects an economy item definition, such as the
//! Iron Bomber. Native item generation initializes item definitions, schema
//! attributes, and models before spawning. [`PlayerWeapons::give_item_with`]
//! also applies an [`AttributeSet`], and [`Weapon::attributes`] reads and
//! changes a weapon's attributes afterwards, through [`ItemAttributes`].

#[cfg(test)]
#[path = "../../tests/tf2/weapons.rs"]
mod tests;

use crate::entities::{Entity, EntityHandle};
use crate::math::Vector;
use crate::tf2::attributes::{self, AttributeError, AttributeSet, ItemAttributes, SchemaToken};
use crate::{Game, InterfaceError, Server};
use sdk_raw::tf2::item_generation::ItemGeneration;
use sdk_raw::tf2::weapons::INVALID_ITEM_DEF_INDEX;
use sdk_raw::vcall;
use std::ffi::{CStr, c_int};
use std::ptr::NonNull;

pub use sdk_raw::tf2::item_generation::ItemGenerationError;

/// A native weapon slot: a [`WeaponSlot`], or a raw slot number for the less
/// common slots it does not name.
pub trait IntoWeaponSlot {
	/// The raw slot number, as compared with each weapon's native `GetSlot`.
	fn into_weapon_slot(self) -> c_int;
}

impl IntoWeaponSlot for c_int {
	fn into_weapon_slot(self) -> c_int {
		self
	}
}

/// An item definition in TF2's economy schema. A valid index need not exist in
/// the running server's schema, and can describe a cosmetic instead of a weapon.
#[doc(alias("item_definition_index_t"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ItemDefinitionIndex(u16);

impl ItemDefinitionIndex {
	/// Excludes the engine's invalid sentinel, [`INVALID_ITEM_DEF_INDEX`].
	/// Index zero is valid.
	pub const fn new(index: u16) -> Option<Self> {
		if index == INVALID_ITEM_DEF_INDEX {
			None
		} else {
			Some(Self(index))
		}
	}

	/// The raw schema index, never the invalid sentinel.
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
	/// Wraps a player's inventory, or returns [`WeaponError::NotTfPlayer`]
	/// unless the server runs TF2 and `player`'s datamaps include `CTFPlayer`.
	pub fn new(server: Server<'s>, player: Entity<'s>) -> Result<Self, WeaponError> {
		if server.game() != Game::TeamFortress2 || !player.has_data_map_class(c"CTFPlayer") {
			return Err(WeaponError::NotTfPlayer);
		}

		Ok(Self { server, player })
	}

	/// Detaches this player's weapon without deleting it. The detached entity
	/// can be equipped again or passed to `ServerTools::remove` for deletion.
	/// This is an inventory operation: native `RemovePlayerItem` can leave
	/// the entity's parenting and attribute-provider association until it is
	/// equipped again or removed. Use `replace` for a complete exchange.
	#[doc(alias("RemovePlayerItem"))]
	pub fn detach(self, weapon: Weapon<'s>) -> Result<(), WeaponError> {
		check_live(self.player)?;

		if weapon.owner()? != Some(self.player.handle()) {
			return Err(WeaponError::DifferentOwner);
		}

		let player = self.player.as_ptr().cast::<sys::CTFPlayer>();

		// SAFETY: `new` found `CTFPlayer` in the player's datamaps and
		// `Weapon::new` found `CTFWeaponBase` in the weapon's, so the player is
		// a `CTFPlayer` and the weapon a `CTFWeaponBase`, whose entity bases
		// `sdk_raw::tf2` and `sdk_raw::tf2::weapons` assert are at offset zero.
		// The player owns this live weapon. RemovePlayerItem detaches and
		// holsters it without immediately deleting either entity.
		let removed = unsafe {
			vcall!(player as sys::CTFPlayer__bindgen_vtable => CTFPlayer_RemovePlayerItem(
				weapon.entity.as_ptr().cast(),
			))
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
	#[doc(alias("Weapon_Equip"))]
	pub fn equip(self, weapon: Weapon<'s>) -> Result<(), WeaponError> {
		check_live(self.player)?;
		check_live(weapon.entity)?;

		let owner = weapon.owner()?;

		if owner.is_some_and(|owner| owner != self.player.handle()) {
			return Err(WeaponError::DifferentOwner);
		}

		let slot = weapon.slot_raw()?;

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
		// SAFETY: `new` found `CTFPlayer` in the player's datamaps and
		// `Weapon::new` found `CTFWeaponBase` in the weapon's, so the player is
		// a `CTFPlayer` and the weapon a `CTFWeaponBase`, whose entity bases
		// `sdk_raw::tf2` and `sdk_raw::tf2::weapons` assert are at offset zero.
		// The live weapon is not in an inventory. Native equip updates
		// inventory, ownership, and attribute providers through the generated
		// TF2 vtable.
		unsafe {
			vcall!(player as sys::CTFPlayer__bindgen_vtable => CTFPlayer_Weapon_Equip(
				weapon.entity.as_ptr().cast(),
			));
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

	/// A weapon in this player's inventory whose native slot matches, or
	/// `None` when none does.
	#[doc(alias("Weapon_GetSlot"))]
	pub fn get_slot(self, slot: impl IntoWeaponSlot) -> Result<Option<Weapon<'s>>, WeaponError> {
		check_live(self.player)?;

		let player = self.player.as_ptr().cast::<sys::CTFPlayer>();

		// SAFETY: `new` found `CTFPlayer` in the player's datamaps, so it is
		// one, whose entity base `sdk_raw::tf2` asserts is at offset zero. The
		// generated virtual method scans its own inventory and returns a live
		// weapon or null, whose entity base `sdk_raw::tf2::weapons` asserts is
		// likewise at offset zero. The slot is a comparison value.
		let raw = unsafe {
			vcall!(player as sys::CTFPlayer__bindgen_vtable => CTFPlayer_Weapon_GetSlot(
				slot.into_weapon_slot(),
			))
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
	#[doc(alias("GiveNamedItem"))]
	pub unsafe fn give(self, classname: &CStr, subtype: i32) -> Result<Weapon<'s>, WeaponError> {
		check_live(self.player)?;

		let player = self.player.as_ptr().cast::<sys::CTFPlayer>();

		// SAFETY: `new` found `CTFPlayer` in the player's datamaps, so it is
		// one, whose entity base `sdk_raw::tf2` asserts is at offset zero. This
		// generated overload takes a nullable CEconItemView and a force flag.
		// Null requests native stock-item generation; force keeps the exact
		// classname instead of translating it for the player's class. The
		// caller vouches for the spawn/pickup path. GiveNamedItem returns a
		// newly created callback-live entity.
		unsafe {
			self.give_with(
				None,
				|| {
					NonNull::new(
						vcall!(player as sys::CTFPlayer__bindgen_vtable => CTFPlayer_GiveNamedItem1(
							classname.as_ptr(),
							subtype,
							std::ptr::null(),
							true,
						)),
					)
					.ok_or(WeaponError::CreationFailed)
				},
				|_| Ok(()),
			)
		}
	}

	/// Creates and equips a weapon from its economy item definition, with its
	/// schema attributes and models initialized before spawning. Uses Unique
	/// quality and level 1. Existing weapons are not replaced.
	///
	/// Definitions whose schema uses a generic classname such as
	/// `tf_weapon_shotgun` need [`Self::give_item_as`] with a concrete classname.
	/// Native generation failures, such as a definition the running schema
	/// lacks ([`ItemGenerationError::UnknownDefinition`]), return
	/// [`WeaponError::CreationFailedNative`]; cosmetics return
	/// [`WeaponError::NotWeapon`].
	///
	/// # Safety
	/// The definition's constructor, spawn, activation and equipment callbacks
	/// must uphold `Server::new`'s no-immediate-deletion contract.
	#[doc(alias("SpawnItem"))]
	pub unsafe fn give_item(
		self,
		definition: ItemDefinitionIndex,
	) -> Result<Weapon<'s>, WeaponError> {
		// SAFETY: The caller supplies the native creation guarantees.
		unsafe { self.spawn_item(definition, None, |_| Ok(())) }
	}

	/// As [`Self::give_item`], using an exact classname instead of the schema's
	/// classname. For example, a generic shotgun definition can use
	/// `tf_weapon_shotgun_soldier`. The definition's static attributes are retained.
	///
	/// # Safety
	/// The guarantees of `give_item` apply, and the classname must be a weapon
	/// implementation compatible with the chosen item definition.
	#[doc(alias("SpawnItem"))]
	pub unsafe fn give_item_as(
		self,
		definition: ItemDefinitionIndex,
		classname: &CStr,
	) -> Result<Weapon<'s>, WeaponError> {
		// SAFETY: The caller supplies the native creation/class guarantees.
		unsafe { self.spawn_item(definition, Some(classname), |_| Ok(())) }
	}

	/// As [`Self::give_item`], then sets `attributes` on the equipped weapon
	/// as [`AttributeSet::apply`] does, and reapplies its provision
	/// ([`ItemAttributes::reapply_provision`]). The game decides whether a
	/// weapon provides its attributes to its owner when equipping it, before
	/// the set's attributes exist; without reapplying, a weapon given
	/// [`PROVIDE_ON_ACTIVE`] would keep providing while holstered until it
	/// was next deployed and holstered.
	///
	/// If either step fails, the new weapon is detached and deleted, and the
	/// error returned as [`WeaponError::Attribute`]. An empty set changes
	/// nothing, as with [`Self::give_item`].
	///
	/// The game replaces weapons it did not hand out itself on resupply and
	/// respawn, so apply the set again to the new weapons it hands out.
	///
	/// # Safety
	/// The guarantees of `give_item` apply.
	///
	/// [`PROVIDE_ON_ACTIVE`]: crate::tf2::attributes::catalog::PROVIDE_ON_ACTIVE
	#[doc(alias("SpawnItem", "AddAttribute"))]
	pub unsafe fn give_item_with(
		self,
		token: SchemaToken<'s>,
		definition: ItemDefinitionIndex,
		attributes: &AttributeSet,
	) -> Result<Weapon<'s>, WeaponError> {
		// SAFETY: The caller supplies the native creation guarantees.
		unsafe {
			self.spawn_item(definition, None, |weapon| {
				apply_attributes(token, weapon, attributes)
			})
		}
	}

	/// `create` must return a newly created entity, live through this callback.
	/// `finish` runs once the weapon is equipped; if it fails, the weapon is
	/// detached and deleted as if equipping it had failed.
	unsafe fn give_with(
		self,
		expected_classname: Option<&CStr>,
		create: impl FnOnce() -> Result<NonNull<sys::CBaseEntity>, WeaponError>,
		finish: impl FnOnce(Weapon<'s>) -> Result<(), WeaponError>,
	) -> Result<Weapon<'s>, WeaponError> {
		check_live(self.player)?;
		let tools = self.server.server_tools()?;

		// GiveNamedItem can equip before returning. Snapshot before creation so
		// slot iteration order cannot hide a collision after native pickup.
		let mut occupied = [false; 256];

		for slot in 0..=u8::MAX {
			occupied[usize::from(slot)] = self.get_slot(c_int::from(slot))?.is_some();
		}

		let raw = create()?;

		// SAFETY: The caller guarantees a newly created callback-live entity.
		let entity = unsafe { Entity::from_raw(raw) };

		check_live(entity)?;

		if expected_classname.is_some_and(|expected| entity.class_name() != expected) {
			// SpawnItem falls back to the schema classname when an override has
			// no factory. Enforce *_as semantics before the fallback can be equipped.
			let _ = tools.remove(entity);

			return Err(WeaponError::CreationFailed);
		}

		let weapon = match Weapon::new(self.server, entity) {
			Ok(weapon) => weapon,

			Err(error) => {
				let _ = tools.remove(entity);
				return Err(error);
			}
		};

		let equipped = weapon
			.slot_raw()
			.and_then(|slot| {
				let slot = u8::try_from(slot).map_err(|_| WeaponError::WrongSlot)?;

				if occupied[usize::from(slot)] {
					Err(WeaponError::SlotOccupied)
				} else {
					self.equip(weapon)
				}
			})
			.and_then(|()| finish(weapon));

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

	/// The player whose inventory this is.
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

	/// Creates an economy item through native item generation, then equips it
	/// and runs `finish` as [`Self::give_with`] does.
	///
	/// # Safety
	/// The guarantees of `give_item`, or of `give_item_as` with a classname.
	unsafe fn spawn_item(
		self,
		definition: ItemDefinitionIndex,
		classname: Option<&CStr>,
		finish: impl FnOnce(Weapon<'s>) -> Result<(), WeaponError>,
	) -> Result<Weapon<'s>, WeaponError> {
		check_live(self.player)?;

		let origin = self.player.position().ok_or(WeaponError::MissingOrigin)?;

		// SAFETY: The caller vouches for the native creation path and the
		// classname. The generator initializes CEconItemView before
		// Spawn/Activate and returns a fresh callback-live entity; it must not be
		// passed through DispatchSpawn again.
		unsafe {
			self.give_with(
				classname,
				|| {
					generate_item(self.server, definition, origin, classname)
						.map_err(WeaponError::CreationFailedNative)
				},
				finish,
			)
		}
	}
}

/// A callback-scoped TF2 weapon. Keep its entity handle across callbacks.
#[doc(alias("CTFWeaponBase"))]
#[derive(Debug, Clone, Copy)]
pub struct Weapon<'s> {
	server: Server<'s>,
	entity: Entity<'s>,
	owner_offset: usize,
}

impl<'s> Weapon<'s> {
	/// Wraps a TF2 weapon entity. Returns [`WeaponError::NotWeapon`] unless the
	/// server runs TF2 and `entity`'s datamaps include `CTFWeaponBase`, and
	/// [`WeaponError::UnsupportedLayout`] if `CBaseCombatWeapon`'s datamap lacks
	/// a usable `m_hOwner` EHANDLE.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, WeaponError> {
		if server.game() != Game::TeamFortress2 || !entity.has_data_map_class(c"CTFWeaponBase") {
			return Err(WeaponError::NotWeapon);
		}

		let owner_offset = entity
			.data_maps()
			.find(|map| map.class_name() == Some(c"CBaseCombatWeapon"))
			.and_then(|map| map.field_offset(c"m_hOwner", sys::_fieldtypes_FIELD_EHANDLE))
			.filter(|offset| *offset < 65_536 && offset.is_multiple_of(align_of::<u32>()))
			.ok_or(WeaponError::UnsupportedLayout)?;

		Ok(Self {
			server,
			entity,
			owner_offset,
		})
	}

	/// The weapon's item attributes, as [`ItemAttributes::new`] wraps them.
	pub fn attributes(self) -> Result<ItemAttributes<'s>, AttributeError> {
		ItemAttributes::new(self.server, self.entity)
	}

	/// The weapon's item definition index (`m_iItemDefinitionIndex`), such as
	/// the Iron Bomber's, or `None` for a weapon without one.
	///
	/// Only the networked variables placing the index are checked, not the
	/// rest of the item's attribute storage that [`Self::attributes`] checks.
	/// Fails with [`WeaponError::UnsupportedLayout`] unless they place it
	/// where the SDK's layout does, and with [`WeaponError::Interface`]
	/// without the game DLL's interface.
	#[doc(alias("m_iItemDefinitionIndex", "GetItemDefIndex"))]
	pub fn definition(self) -> Result<Option<ItemDefinitionIndex>, WeaponError> {
		check_live(self.entity)?;

		let dll = self.server.server_game_dll()?;

		attributes::item_definition(dll, self.entity).map_err(|_| WeaponError::UnsupportedLayout)
	}

	/// The weapon's entity.
	pub const fn entity(self) -> Entity<'s> {
		self.entity
	}

	/// Its combat owner's handle (`m_hOwner`), or None for a detached weapon.
	/// The separate base-entity `m_hOwnerEntity` can remain set after detaching
	/// and is not the authority for membership in a player's weapon inventory.
	#[doc(alias("m_hOwner"))]
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

	/// The native weapon slot, which can depend on its item definition, or
	/// `None` for a slot outside [`WeaponSlot`] (see [`Self::slot_raw`]).
	#[doc(alias("GetSlot"))]
	pub fn slot(self) -> Result<Option<WeaponSlot>, WeaponError> {
		self.slot_raw().map(WeaponSlot::from_raw)
	}

	/// The native weapon slot number, including those [`WeaponSlot`] does not name.
	#[doc(alias("GetSlot"))]
	pub fn slot_raw(self) -> Result<c_int, WeaponError> {
		check_live(self.entity)?;

		let weapon = self.entity.as_ptr().cast::<sys::CTFWeaponBase>();

		// SAFETY: `new` found `CTFWeaponBase` in the weapon's datamaps, so it
		// is one, whose entity base `sdk_raw::tf2::weapons` asserts is at offset
		// zero. The generated GetSlot entry leaves the weapon alive.
		Ok(unsafe {
			vcall!(weapon as sys::CTFWeaponBase__bindgen_vtable => CTFWeaponBase_GetSlot())
		})
	}
}

/// Why a weapon or inventory operation failed.
#[derive(Debug, thiserror::Error)]
pub enum WeaponError {
	/// Reading the weapon's attributes failed, or
	/// [`PlayerWeapons::give_item_with`] could not apply its attributes.
	#[error(transparent)]
	Attribute(#[from] AttributeError),

	/// `GiveNamedItem` returned no entity (for example, because the player
	/// already has that weapon type), or a classname override produced an
	/// entity of another classname.
	#[error("the game could not create the weapon (including an existing weapon of the same type)")]
	CreationFailed,

	/// Native item generation, used by `give_item` and its variants, failed,
	/// as the [`ItemGenerationError`] tells.
	#[error(transparent)]
	CreationFailedNative(#[from] ItemGenerationError),

	/// The weapon's combat owner is not this player: another entity owns it,
	/// or [`PlayerWeapons::detach`] was given an unowned weapon.
	#[error("the weapon is not owned by this player")]
	DifferentOwner,

	/// A required engine interface is unavailable.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// The player or weapon is already marked for deletion.
	#[error("the entity is marked for deletion")]
	MarkedForDeletion,

	/// The player's position, needed to spawn an economy item, is unavailable.
	#[error("the player's absolute position could not be read")]
	MissingOrigin,

	/// The entity is not a TF2 player, or the server does not run TF2.
	#[error("weapon operations require a TF2 player")]
	NotTfPlayer,

	/// The entity is not a TF2 weapon (for example, it is a cosmetic), or the
	/// server does not run TF2.
	#[error("the entity is not a TF2 combat weapon")]
	NotWeapon,

	/// The game did not complete a detach or equip: `RemovePlayerItem` refused
	/// the weapon, or it did not end up in its slot with this player as its
	/// combat owner. A refused equip may still have set the weapon's owner.
	#[error("the game refused to detach or equip the weapon")]
	Rejected,

	/// The weapon's native slot already holds another weapon.
	#[error("the weapon slot is already occupied; use replace to exchange its weapon")]
	SlotOccupied,

	/// The weapon's datamap lacks a usable `m_hOwner` field, or, for
	/// [`Weapon::definition`], its networked variables do not place its item
	/// definition index where the SDK's layout does.
	#[error("the weapon's datamap or networked variables do not match the sdk's layout")]
	UnsupportedLayout,

	/// A replacement's native slot is not the requested slot, or a new
	/// weapon's slot number is outside 0..=255, the range creation tracks.
	#[error("the weapon's native slot is not the requested slot, or is outside 0..=255")]
	WrongSlot,
}

/// One of the six common native weapon slots, which are not item-schema
/// loadout positions. Weapons can report other slot numbers; use
/// [`Weapon::slot_raw`] and raw `c_int` slots for those.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum WeaponSlot {
	/// Slot 0.
	Primary = 0,

	/// Slot 1.
	Secondary = 1,

	/// Slot 2.
	Melee = 2,

	/// Slot 3, such as the Engineer's construction PDA.
	Pda = 3,

	/// Slot 4, such as the Engineer's destruction PDA.
	Pda2 = 4,

	/// Slot 5, such as the Engineer's builder (`tf_weapon_builder`), held
	/// while placing a building.
	Building = 5,
}

impl WeaponSlot {
	/// Every named slot, in native order.
	pub const ALL: [Self; 6] = [
		Self::Primary,
		Self::Secondary,
		Self::Melee,
		Self::Pda,
		Self::Pda2,
		Self::Building,
	];

	/// The named slot with this native number, or `None` for any other number.
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		match raw {
			0 => Some(Self::Primary),
			1 => Some(Self::Secondary),
			2 => Some(Self::Melee),
			3 => Some(Self::Pda),
			4 => Some(Self::Pda2),
			5 => Some(Self::Building),
			_ => None,
		}
	}

	/// The native slot number.
	pub const fn to_raw(self) -> c_int {
		self as c_int
	}
}

impl IntoWeaponSlot for WeaponSlot {
	fn into_weapon_slot(self) -> c_int {
		self.to_raw()
	}
}

/// Sets `attributes` on a newly equipped weapon, then reapplies its
/// provision, which the game decided when equipping it. An empty set changes
/// nothing.
fn apply_attributes<'s>(
	token: SchemaToken<'s>,
	weapon: Weapon<'s>,
	attributes: &AttributeSet,
) -> Result<(), WeaponError> {
	if attributes.is_empty() {
		return Ok(());
	}

	let item = weapon.attributes()?;

	attributes.apply(token, item)?;
	item.reapply_provision(token)?;

	Ok(())
}

fn check_live(entity: Entity<'_>) -> Result<(), WeaponError> {
	if entity.is_marked_for_deletion() {
		Err(WeaponError::MarkedForDeletion)
	} else {
		Ok(())
	}
}

/// Creates the economy item `definition` at `origin` through native item
/// generation, at level 1 and with Unique quality, optionally as the entity
/// class `classname`, as [`ItemGeneration::spawn`] describes. The entity is
/// newly created and spawned, and must not be spawned again.
///
/// Once a call has resolved item generation in the game server module, later
/// calls reuse it without inspecting the module again, as
/// [`ItemGeneration::cached`] describes. This relies on Source never unloading
/// that module while plugins are loaded, so that the module at its base
/// address stays the image that was inspected.
///
/// # Safety
///
/// The item's constructor, spawn and activation, and everything they reach,
/// must uphold [`Server::new`]'s no-immediate-deletion contract. `classname`,
/// if given, must name an entity class compatible with `definition`.
pub(crate) unsafe fn generate_item(
	server: Server<'_>,
	definition: ItemDefinitionIndex,
	origin: Vector,
	classname: Option<&CStr>,
) -> Result<NonNull<sys::CBaseEntity>, ItemGenerationError> {
	// SAFETY: `Server::new` guarantees that the game server module, whose
	// factory this is, stays loaded through the callback. A cached resolution
	// for the same factory and module base was made in this same image: Source
	// never unloads the game server module while plugins are loaded, since
	// Metamod:Source and the engine unload plugins first, and the cache, a
	// static of this plugin, is unloaded with it.
	let generation = unsafe { ItemGeneration::cached(server.game_server_factory().as_raw()) }?;

	// SAFETY: As above, the module stays loaded, and this runs on the main
	// thread, inside the engine's callback. The caller vouches for the game
	// code the generation runs, and for the classname.
	unsafe { generation.spawn(definition.get(), origin.into(), classname) }
}
