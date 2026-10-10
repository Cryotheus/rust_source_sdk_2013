//! TF2 weapon creation, inventory slots, equipment, and replacement.
//!
//! [`PlayerWeapons::give`] creates a stock weapon by classname;
//! [`PlayerWeapons::give_item`] selects an economy item definition, such as the
//! Iron Bomber. Native item generation initializes item definitions, schema
//! attributes, and models before spawning. [`PlayerWeapons::give_item_with`]
//! also applies an [`AttributeSet`], and [`Weapon::attributes`] reads and
//! changes a weapon's attributes afterwards, through [`ItemAttributes`].

pub mod identity;

#[cfg(test)]
#[path = "../../tests/tf2/weapons.rs"]
mod tests;

use crate::datatables::NetPropError;
use crate::entities::{Entity, EntityHandle};
use crate::math::Vector;
use crate::tf2::attributes::{self, AttributeError, AttributeSet, ItemAttributes, SchemaToken};
use crate::tf2::item_schema::{ItemLevel, ItemQuality};
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

	/// As [`Self::give_item_with`], but exchanging the weapon the new one's
	/// native slot holds, if any, for it, as [`Self::replace_item`] does,
	/// without the slot being named first. With `classname`, the item is created
	/// as that class instead of the schema's, as for [`Self::give_item_as`],
	/// such as the class [`ItemDefinition::item_class_for`] gives for the
	/// player's class.
	///
	/// The new weapon is created first. Then the old one is detached, the new
	/// one equipped and given `attributes`, and only then the old one deleted.
	/// If equipping the new weapon or setting its attributes fails, it is
	/// deleted, and the old one equipped again.
	///
	/// Detaching the weapon in the player's hands leaves them holding nothing
	/// until they switch weapons, as [`Self::switch_to`] makes them.
	///
	/// # Safety
	/// The guarantees of `give_item` apply, and with `classname`, those of
	/// `give_item_as`.
	///
	/// [`ItemDefinition::item_class_for`]: crate::tf2::item_schema::ItemDefinition::item_class_for
	#[doc(alias("SpawnItem", "AddAttribute"))]
	pub unsafe fn exchange_item_with(
		self,
		token: SchemaToken<'s>,
		definition: ItemDefinitionIndex,
		classname: Option<&CStr>,
		attributes: &AttributeSet,
	) -> Result<Weapon<'s>, WeaponError> {
		check_live(self.player)?;

		let origin = self.player.position().ok_or(WeaponError::MissingOrigin)?;

		// SAFETY: As for `spawn_item`: the caller vouches for the native creation
		// path and the classname, and the generator returns a fresh callback-live
		// entity, initialized before Spawn/Activate.
		unsafe {
			self.exchange_with(
				classname,
				|| {
					generate_quality_item(
						self.server,
						definition,
						origin,
						classname,
						ItemQuality::Unique,
						ItemLevel::DEFAULT,
					)
					.map_err(WeaponError::CreationFailedNative)
				},
				|weapon| apply_attributes(token, weapon, attributes),
			)
		}
	}

	/// Creates a weapon with `create`, then exchanges it for the weapon its
	/// native slot holds, if any, as [`Self::exchange_item_with`] describes,
	/// running `finish` once it is equipped.
	///
	/// # Safety
	/// `create` must return a newly created entity, live through this callback,
	/// as for [`Self::give_with`].
	unsafe fn exchange_with(
		self,
		expected_classname: Option<&CStr>,
		create: impl FnOnce() -> Result<NonNull<sys::CBaseEntity>, WeaponError>,
		finish: impl FnOnce(Weapon<'s>) -> Result<(), WeaponError>,
	) -> Result<Weapon<'s>, WeaponError> {
		check_live(self.player)?;
		let tools = self.server.server_tools()?;
		let raw = create()?;

		// SAFETY: The caller guarantees a newly created callback-live entity.
		let entity = unsafe { Entity::from_raw(raw) };

		check_live(entity)?;

		if expected_classname.is_some_and(|expected| entity.class_name() != expected) {
			// As in `give_with`: item generation falls back to the schema's
			// classname when an override has no factory.
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

		// A weapon the game already equipped holds its own slot, and is not
		// exchanged for itself.
		let old = match weapon.slot_raw().and_then(|slot| self.get_slot(slot)) {
			Ok(old) => old.filter(|old| old.entity != entity),

			Err(error) => {
				let _ = tools.remove(entity);
				return Err(error);
			}
		};

		if let Some(old) = old
			&& let Err(error) = self.detach(old)
		{
			let _ = tools.remove(entity);
			return Err(error);
		}

		if let Err(error) = self.equip(weapon).and_then(|()| finish(weapon)) {
			// As in `give_with`: a refused equip can still have set the owner, and
			// another player's weapon is never deleted.
			match weapon.owner() {
				Ok(Some(owner)) if owner == self.player.handle() => {
					let _ = self.detach(weapon);
					let _ = tools.remove(entity);
				}

				Ok(None) => {
					let _ = tools.remove(entity);
				}

				Ok(Some(_)) | Err(_) => {}
			}

			if let Some(old) = old {
				self.equip(old)?;
			}

			return Err(error);
		}

		if let Some(old) = old {
			let _ = tools.remove(old.entity);
		}

		Ok(weapon)
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
	/// quality and level 1, as [`Self::give_item_with_quality`] does with
	/// others. Existing weapons are not replaced.
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
		unsafe { self.give_item_with_quality(definition, ItemQuality::Unique, ItemLevel::DEFAULT) }
	}

	/// As [`Self::give_item`], but creates the weapon at `level` and with
	/// `quality`, which clients show in its description and color its name
	/// by, such as a level 100 Strange weapon. Neither changes how the weapon
	/// plays, as [`ItemQuality`] describes.
	///
	/// # Safety
	/// The guarantees of `give_item` apply.
	#[doc(alias("SpawnItem"))]
	pub unsafe fn give_item_with_quality(
		self,
		definition: ItemDefinitionIndex,
		quality: ItemQuality,
		level: ItemLevel,
	) -> Result<Weapon<'s>, WeaponError> {
		// SAFETY: The caller supplies the native creation guarantees.
		unsafe { self.spawn_item(definition, None, quality, level, |_| Ok(())) }
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
		unsafe {
			self.spawn_item(
				definition,
				Some(classname),
				ItemQuality::Unique,
				ItemLevel::DEFAULT,
				|_| Ok(()),
			)
		}
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
			self.spawn_item(
				definition,
				None,
				ItemQuality::Unique,
				ItemLevel::DEFAULT,
				|weapon| apply_attributes(token, weapon, attributes),
			)
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

	/// Creates an economy item through native item generation, at `level` and
	/// with `quality`, then equips it and runs `finish` as
	/// [`Self::give_with`] does.
	///
	/// # Safety
	/// The guarantees of `give_item`, or of `give_item_as` with a classname.
	unsafe fn spawn_item(
		self,
		definition: ItemDefinitionIndex,
		classname: Option<&CStr>,
		quality: ItemQuality,
		level: ItemLevel,
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
					generate_quality_item(
						self.server,
						definition,
						origin,
						classname,
						quality,
						level,
					)
					.map_err(WeaponError::CreationFailedNative)
				},
				finish,
			)
		}
	}

	/// The weapon the player holds (`m_hActiveWeapon`), or `None` if they hold
	/// none, or hold an entity that is no TF2 combat weapon.
	#[doc(alias("m_hActiveWeapon", "GetActiveWeapon"))]
	pub fn active(self) -> Result<Option<Weapon<'s>>, WeaponError> {
		let handle = self
			.server
			.server_game_dll()?
			.entity_net_prop(self.player, c"m_hActiveWeapon")?
			.get_handle(self.player)?;

		match self.server.server_tools()?.entity_by_handle(handle) {
			Some(weapon) => weapon_of(self.server, weapon),
			None => Ok(None),
		}
	}

	/// The weapons the player carries (`m_hMyWeapons`), in the order of their
	/// inventory, which is not that of their slots. Entities in it that are no
	/// TF2 combat weapon are skipped.
	#[doc(alias("m_hMyWeapons", "GetWeapon"))]
	pub fn all(self) -> Result<Vec<Weapon<'s>>, WeaponError> {
		let weapons = self
			.server
			.server_game_dll()?
			.entity_net_prop(self.player, c"m_hMyWeapons")?;
		let tools = self.server.server_tools()?;
		let count = weapons
			.element_count()
			.ok_or_else(|| NetPropError::NotAnArray {
				name: String::from("m_hMyWeapons"),
				kind: weapons.prop().kind(),
			})?;
		let mut all = Vec::new();

		for index in 0..count {
			let handle = weapons.element(index)?.get_handle(self.player)?;

			if let Some(weapon) = tools.entity_by_handle(handle) {
				all.extend(weapon_of(self.server, weapon)?);
			}
		}

		Ok(all)
	}

	/// Switches the player to `weapon`, one of theirs, as choosing it does
	/// (`Weapon_Switch`), and returns whether they hold it now.
	///
	/// TF2 refuses the switch for a ghost, for a weapon that cannot be deployed,
	/// and while the held weapon cannot be holstered, such as the Heavy's spun
	/// up minigun. A weapon with a holster animation, through its
	/// `holster_anim_time` attribute, switches when the animation ends instead,
	/// so this returns false for it, although the switch happens later.
	///
	/// Holstering and deploying run the game's, other plugins', and this
	/// plugin's own callbacks synchronously, such as hooks of the weapons'
	/// `Deploy`, which must keep to the contract of [`Server::new`]. Fails with
	/// [`WeaponError::DifferentOwner`] for a weapon this player does not own,
	/// before the game is called.
	#[doc(alias("Weapon_Switch"))]
	pub fn switch_to(self, weapon: Weapon<'s>) -> Result<bool, WeaponError> {
		check_live(self.player)?;

		if weapon.owner()? != Some(self.player.handle()) {
			return Err(WeaponError::DifferentOwner);
		}

		let player = self.player.as_ptr().cast::<sys::CTFPlayer>();

		// SAFETY: As for `detach`, at the slot `sdk_raw::tf2::player` checks: the
		// player owns this live weapon, which `CTFPlayer::Weapon_Switch` casts
		// to `CTFWeaponBase`, as `Weapon::new` found it to be, for the first
		// view model. Holstering and deploying free entities only through
		// deferred deletion, as do the callbacks they run (`Server::new`'s
		// contract).
		Ok(unsafe {
			vcall!(player as sys::CTFPlayer__bindgen_vtable => CTFPlayer_Weapon_Switch(
				weapon.entity.as_ptr().cast(),
				0,
			))
		})
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

	/// The rounds in the weapon's clip (`m_iClip1`), or -1 for a weapon without
	/// one, such as a melee weapon or a minigun. Energy weapons count their
	/// shots in energy instead, and leave it unused.
	///
	/// Fails with [`WeaponError::UnsupportedLayout`] if `CBaseCombatWeapon`'s
	/// datamap lacks a usable `m_iClip1` integer.
	#[doc(alias("m_iClip1"))]
	pub fn clip(self) -> Result<c_int, WeaponError> {
		check_live(self.entity)?;

		let offset = self.clip_offset()?;

		// SAFETY: `clip_offset` validated the integer field in the datamap of
		// `CBaseCombatWeapon`, which `new` found the weapon's class derives from,
		// through `CTFWeaponBase`. The callback keeps the weapon allocated, and the
		// member is read without forming a reference, as the game writes it too.
		Ok(unsafe { self.entity.as_ptr().byte_add(offset).cast::<c_int>().read() })
	}

	/// The offset of the clip, `m_iClip1`, in `CBaseCombatWeapon`'s own datamap.
	fn clip_offset(self) -> Result<usize, WeaponError> {
		self.entity
			.data_maps()
			.find(|map| map.class_name() == Some(c"CBaseCombatWeapon"))
			.and_then(|map| map.field_offset(c"m_iClip1", sys::_fieldtypes_FIELD_INTEGER))
			.filter(|offset| *offset < 65_536 && offset.is_multiple_of(align_of::<c_int>()))
			.ok_or(WeaponError::UnsupportedLayout)
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

	/// Whether the weapon counts its shots in energy, which recharges on its own,
	/// instead of ammo (`IsEnergyWeapon`), as the Cow Mangler 5000 and the
	/// Righteous Bison do.
	#[doc(alias("IsEnergyWeapon"))]
	pub fn is_energy_weapon(self) -> Result<bool, WeaponError> {
		check_live(self.entity)?;

		let weapon = self.entity.as_ptr().cast::<sys::CTFWeaponBase>();

		// SAFETY: As for `slot_raw`. The generated IsEnergyWeapon entry only reads
		// the weapon.
		Ok(unsafe {
			vcall!(weapon as sys::CTFWeaponBase__bindgen_vtable => CTFWeaponBase_IsEnergyWeapon())
		})
	}

	/// The most rounds the weapon's clip holds, after its attributes and its
	/// owner's powerups (`GetMaxClip1`), or `None` for a weapon without a clip.
	/// For an energy weapon, it is the most energy the weapon holds instead.
	#[doc(alias("GetMaxClip1"))]
	pub fn max_clip(self) -> Result<Option<c_int>, WeaponError> {
		check_live(self.entity)?;

		let weapon = self.entity.as_ptr().cast::<sys::CTFWeaponBase>();

		// SAFETY: As for `slot_raw`. The generated GetMaxClip1 entry only reads the
		// weapon, its owner, and their attributes.
		let max = unsafe {
			vcall!(weapon as sys::CTFWeaponBase__bindgen_vtable => CTFWeaponBase_GetMaxClip1())
		};

		// `WEAPON_NOCLIP` is -1.
		Ok((max >= 0).then_some(max))
	}

	/// Traces this melee weapon's current swing through native `DoSwingTrace`,
	/// including its range/bounds attributes and the game's collision filters.
	/// Returns the live hit entity, or `None` for a miss. Surface storage does
	/// not escape the call. This does not start lag compensation or deal damage.
	/// Fails with [`WeaponError::NotMelee`] unless its datamaps include
	/// `CTFWeaponBaseMelee`, and refuses a weapon marked for deletion.
	#[doc(alias("DoSwingTrace"))]
	pub fn melee_hit(self) -> Result<Option<Entity<'s>>, WeaponError> {
		check_live(self.entity)?;
		if !self.entity.has_data_map_class(c"CTFWeaponBaseMelee") {
			return Err(WeaponError::NotMelee);
		}
		let this = self.entity.as_ptr();
		// SAFETY: Zero initializes the native out-parameter; DoSwingTrace
		// fills it before returning a hit. The verified melee primary vtable
		// has bool(this, trace_t*) under both supported 64-bit ABIs.
		let mut trace: sys::trace_t = unsafe { std::mem::zeroed() };
		// SAFETY: The live validated melee stays allocated for this callback.
		// Native tracing only reads collision state and attributes.
		let hit = unsafe {
			sdk_raw::vcall!(this as sys::CTFWeaponBaseMelee__bindgen_vtable => CTFWeaponBaseMelee_DoSwingTrace(&raw mut trace))
		};
		if !hit {
			return Ok(None);
		}
		// SAFETY: The native trace's hit pointer names a live entity during
		// this callback, including the world for brushes/static props.
		Ok(NonNull::new(trace.m_pEnt)
			.map(|entity| unsafe { Entity::from_live(self.server, entity) }))
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

	/// Sets the rounds in the weapon's clip (`m_iClip1`), and records the change
	/// so the engine sends it to the weapon's owner.
	///
	/// Fails with [`WeaponError::NoClip`] for a weapon without a clip, or an
	/// energy weapon, with [`WeaponError::InvalidClip`] for a count outside 0 to
	/// [`Self::max_clip`], and as [`Self::clip`] does.
	#[doc(alias("m_iClip1"))]
	pub fn set_clip(self, clip: c_int) -> Result<(), WeaponError> {
		if self.is_energy_weapon()? {
			return Err(WeaponError::NoClip);
		}

		let max = self.max_clip()?.ok_or(WeaponError::NoClip)?;

		if !(0..=max).contains(&clip) {
			return Err(WeaponError::InvalidClip { clip, max });
		}

		let offset = self.clip_offset()?;
		let changed = u16::try_from(offset).map_err(|_| WeaponError::UnsupportedLayout)?;
		let engine = self.server.valve_engine()?;
		let edict = self.entity.edict().ok_or(WeaponError::NotWeapon)?;

		// SAFETY: As for `clip`. The game writes its networked variables the same
		// way, through its own pointers, on the main thread, and fills clips with
		// anything from none to their max.
		unsafe {
			self.entity
				.as_ptr()
				.byte_add(offset)
				.cast::<c_int>()
				.write(clip)
		};

		// The clip is networked from the same member.
		edict.state_changed(engine, changed);

		Ok(())
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

	/// [`Weapon::set_clip`] was given a count outside 0 to the weapon's max.
	#[error("cannot fill a clip of at most {max} rounds with {clip}")]
	InvalidClip {
		/// The count given.
		clip: c_int,

		/// The weapon's max clip.
		max: c_int,
	},

	/// The player or weapon is already marked for deletion.
	#[error("the entity is marked for deletion")]
	MarkedForDeletion,

	/// The player's position, needed to spawn an economy item, is unavailable.
	#[error("the player's absolute position could not be read")]
	MissingOrigin,

	/// The player's weapons, which [`PlayerWeapons::active`] and
	/// [`PlayerWeapons::all`] read from its networked variables, could not
	/// be read.
	#[error(transparent)]
	NetProp(#[from] NetPropError),

	/// The weapon has no clip, as melee weapons and miniguns, or counts its
	/// shots in energy.
	#[error("the weapon has no clip")]
	NoClip,

	/// The weapon does not derive from `CTFWeaponBaseMelee`.
	#[error("the weapon is not a TF2 melee weapon")]
	NotMelee,

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
	/// [`Weapon::clip`] and [`Weapon::set_clip`], a usable `m_iClip1` field,
	/// or, for [`Weapon::definition`], its networked variables do not place its
	/// item definition index where the SDK's layout does.
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
/// class `classname`, as [`generate_quality_item`] does.
///
/// # Safety
///
/// As for [`generate_quality_item`].
pub(crate) unsafe fn generate_item(
	server: Server<'_>,
	definition: ItemDefinitionIndex,
	origin: Vector,
	classname: Option<&CStr>,
) -> Result<NonNull<sys::CBaseEntity>, ItemGenerationError> {
	// SAFETY: The caller upholds the same guarantees.
	unsafe {
		generate_quality_item(
			server,
			definition,
			origin,
			classname,
			ItemQuality::Unique,
			ItemLevel::DEFAULT,
		)
	}
}

/// Creates the economy item `definition` at `origin` through native item
/// generation, at `level` and with `quality`, optionally as the entity class
/// `classname`, as [`ItemGeneration::spawn_with_quality`] describes. The
/// entity is newly created and spawned, and must not be spawned again.
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
pub(crate) unsafe fn generate_quality_item(
	server: Server<'_>,
	definition: ItemDefinitionIndex,
	origin: Vector,
	classname: Option<&CStr>,
	quality: ItemQuality,
	level: ItemLevel,
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
	unsafe {
		generation.spawn_with_quality(
			definition.get(),
			origin.into(),
			classname,
			c_int::from(level.get()),
			quality.to_raw(),
		)
	}
}

/// Wraps `entity` as a weapon, or returns `None` if it is no TF2 combat
/// weapon.
fn weapon_of<'s>(
	server: Server<'s>,
	entity: Entity<'s>,
) -> Result<Option<Weapon<'s>>, WeaponError> {
	match Weapon::new(server, entity) {
		Ok(weapon) => Ok(Some(weapon)),
		Err(WeaponError::NotWeapon) => Ok(None),
		Err(error) => Err(error),
	}
}
