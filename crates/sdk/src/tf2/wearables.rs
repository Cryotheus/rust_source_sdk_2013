//! TF2 wearables (`CTFWearable`): cosmetics, and the other items players wear,
//! such as shields, boots and backpacks.
//!
//! [`PlayerWearables::give`] creates an item from its economy definition
//! through native item generation, as
//! [`PlayerWeapons::give_item`](crate::tf2::weapons::PlayerWeapons::give_item)
//! does, then equips it through the player's own `EquipWearable`, the path the
//! game takes for loadout items. Like a loadout item, it gets the player's
//! class model and team skin, and hides the bodygroups it covers. Players,
//! TFBots and puppet bots are handled alike.
//!
//! # Persistence
//!
//! Whenever a player spawns or resupplies, the game removes every wearable that
//! does not match the player's loadout from the item server
//! (`CTFPlayer::ValidateWearables`), so given wearables last until the next
//! respawn or resupply locker. To keep them, give them again from a listener
//! for [`GameEventId::PostInventoryApplication`], which the game fires after
//! that validation each time. It fires for every player, bots included, so
//! remember the wanted items by player and [`ItemDefinitionIndex`], not by
//! entity handle, since each respawn deletes the old entities. Giving a
//! wearable does not fire the event again, but regenerating the player does:
//! if the listener, or a plugin it triggers, regenerates the player, guard the
//! listener against the nested event.
//!
//! # Limits
//!
//! - The game networks at most [`MAX_NETWORKED_WEARABLES`] entries of a
//!   player's list, and clients only hide bodygroups and apply attributes for
//!   those. Equipping adds a wearable at the head of the list, so equipping
//!   one into a full list would push its oldest entry out of the networked
//!   ones, be it a loadout item, a weapon's extra wearable, an action item or
//!   a disguised Spy's disguise. [`PlayerWearables::give`] and
//!   [`PlayerWearables::equip`] refuse a full list instead.
//! - Other players' clients draw an item attached to a human player only if it
//!   is in that player's item server inventory, is a stock item of their
//!   class, or the server sets the item's `m_bValidatedAttachedEntity`.
//!   [`PlayerWearables::give`] and [`PlayerWearables::equip`] set it before
//!   equipping, in the same callback, since clients cache their verdict.
//!   Clients trust the server anyway for owners whose inventory they have not
//!   loaded, which includes every bot, and for disguised Spies, so the flag
//!   only matters for human owners. The owner always sees their own items
//!   (`CEconEntity::ValidateEntityAttachedToPlayer`).
//! - The game refuses items restricted to a holiday, such as Halloween
//!   cosmetics, outside that holiday.
//! - Bots have no item server inventory, so they wear nothing unless given
//!   wearables.
//! - Neither the item's classes nor its equip regions are checked, but
//!   [`PlayerWearables::give`] refuses an item the player's class has no
//!   model for. An item for another class that has one, or an item
//!   overlapping another, may not draw correctly.
//!
//! # Wearable loops
//!
//! The game walks a player's wearable list (`m_hMyWearables`) by index, and
//! `CBasePlayer::RemoveWearable` removes the entry at the index it found
//! after running the wearable's `UnEquip`. A change to the list in between
//! makes it remove another entry instead, or shrink an emptied list to a
//! negative length, which corrupts the game's container.
//! [`PlayerWearables::give`], [`PlayerWearables::equip`],
//! [`PlayerWearables::remove`] and [`PlayerWearables::strip`] change the
//! list, so they must not run while the game iterates the player's
//! wearables: not from a hook on `EquipWearable` or `RemoveWearable`, a
//! wearable's `Equip` or `UnEquip`, or anything they reach, such as the
//! removal callbacks of the entities they delete.
//! [`GameEventId::PostInventoryApplication`] listeners run outside these
//! loops. Reading the list and its wearables is always allowed.
//!
//! # Unverified
//!
//! On TF2's 64-bit Windows server, `EquipWearable` has been observed to put a
//! created wearable at the head of a puppet bot's list, owned by and parented
//! to the bot, and `RemoveWearable` to unlist it and delete it, deferred. The
//! networked entries read as [`PlayerWearables::list`] decodes them, and
//! [`PlayerWearables::give`] refused an item restricted to other classes on a
//! bot with [`WearableError::MissingModel`]. A retail client drew a wearable
//! given to its own player and hid the bodygroup it covers, and a resupply
//! locker removed the given wearables. What other players' clients, SourceTV
//! and demos draw of a given wearable, removal on respawn, re-giving from a
//! [`GameEventId::PostInventoryApplication`] listener, Mann vs. Machine, and
//! Linux servers, whose vtables are laid out differently, have not been
//! tested.
//!
//! [`GameEventId::PostInventoryApplication`]: crate::tf2::game_events::GameEventId::PostInventoryApplication

use crate::datatables::{NetProp, NetPropError, NetValue, PropKind};
use crate::entities::{Entity, EntityHandle};
use crate::interfaces::ServerTools;
use crate::math::Vector;
use crate::tf2::weapons::ItemDefinitionIndex;
use crate::{Game, InterfaceError, Server};
use sdk_raw::tf2::item_generation::WeaponCreationFailed;
use sdk_raw::vcall;
use std::ffi::{CStr, c_int};
use std::mem::{offset_of, size_of};
use std::ptr::NonNull;

// The entity pointer of a wearable is passed as the `CEconWearable *` that
// `EquipWearable` and `RemoveWearable` take, which needs every class from
// `CTFWearable` down to `CBaseEntity` to start with its primary base. The
// Itanium bindings describe `CEconWearable` and `CBaseAnimating` as opaque
// blobs, whose single, polymorphic primary bases that ABI also places first.
const _: () = {
	assert!(offset_of!(sys::CTFWearable, _base) == 0 && offset_of!(sys::CEconEntity, _base) == 0);

	#[cfg(target_os = "windows")]
	assert!(
		offset_of!(sys::CEconWearable, _base) == 0 && offset_of!(sys::CBaseAnimating, _base) == 0
	);
};

/// The names `SendPropUtlVector` gives the networked elements of
/// `m_hMyWearables`, in order (`DT_ArrayElementNameForIdx`).
const ELEMENT_NAMES: [&CStr; MAX_NETWORKED_WEARABLES] = [
	c"000", c"001", c"002", c"003", c"004", c"005", c"006", c"007",
];

/// `FIRST_GAME_TEAM`, TF2's RED. Lower teams are unassigned and spectators.
const FIRST_GAME_TEAM: c_int = 2;

/// `INVALID_NETWORKED_EHANDLE_VALUE`, which `SendProxy_EHandleToInt` sends
/// for a null handle.
const INVALID_NETWORKED_HANDLE: u32 = (1 << (NETWORKED_INDEX_BITS + NETWORKED_SERIAL_BITS)) - 1;

/// `MAX_WEARABLES_SENT_FROM_SERVER` (TF2's `LOADOUT_MAX_WEARABLES_COUNT`): the
/// most entries of a player's wearable list the game networks.
#[doc(alias = "MAX_WEARABLES_SENT_FROM_SERVER")]
#[doc(alias = "LOADOUT_MAX_WEARABLES_COUNT")]
pub const MAX_NETWORKED_WEARABLES: usize = 8;

/// How far up the move hierarchy a wearable's player is looked for: a view
/// model wearable follows the player's view model, which follows the player.
const MAX_PARENT_DEPTH: usize = 8;

/// The most times `RemoveWearable` is called for one wearable before it is
/// deleted through [`ServerTools`] instead. While the wearable is listed, each
/// call removes one entry, the wearable or the first null entry it meets from
/// the end of the list, so it reaches the wearable within the list's length.
const MAX_REMOVE_ATTEMPTS: usize = 32;

/// `MAX_EDICT_BITS`: the bits of a networked handle holding the entry index.
const NETWORKED_INDEX_BITS: u32 = 11;

/// `NUM_NETWORKED_EHANDLE_SERIAL_NUMBER_BITS`: the bits of a networked handle
/// holding the low bits of the serial number.
const NETWORKED_SERIAL_BITS: u32 = 10;

/// An entity handle as `SendProxy_EHandleToInt` networks it: the entry index
/// in the low 11 bits, and the low 10 bits of the serial number above them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NetworkedHandle(u32);

impl NetworkedHandle {
	/// Decodes a networked handle. Returns `None` for a null handle, for zero,
	/// which would be the world, and for a value wider than a networked handle.
	fn decode(raw: c_int) -> Option<Self> {
		let raw = raw.cast_unsigned();

		(raw != 0
			&& raw != INVALID_NETWORKED_HANDLE
			&& raw >> (NETWORKED_INDEX_BITS + NETWORKED_SERIAL_BITS) == 0)
			.then_some(Self(raw))
	}

	/// The entity's slot in the entity list, which is its edict index.
	const fn index(self) -> usize {
		(self.0 & ((1 << NETWORKED_INDEX_BITS) - 1)) as usize
	}

	/// Whether `handle` has this slot and the serial number bits sent.
	const fn matches(self, handle: EntityHandle) -> bool {
		let serial = handle.serial_number() & ((1 << NETWORKED_SERIAL_BITS) - 1);

		matches!(handle.index(), Some(index) if index == self.index())
			&& serial == self.0 >> NETWORKED_INDEX_BITS
	}

	/// Finds the networked entity the handle refers to, if it still exists.
	fn resolve<'s>(self, tools: ServerTools<'s>) -> Option<Entity<'s>> {
		let entity = tools.entity_by_index(c_int::try_from(self.index()).ok()?)?;

		self.matches(entity.handle()).then_some(entity)
	}
}

/// A TF2 player's wearables (`m_hMyWearables`), scoped to one engine callback.
#[derive(Debug, Clone, Copy)]
pub struct PlayerWearables<'s> {
	server: Server<'s>,
	player: Entity<'s>,
}

impl<'s> PlayerWearables<'s> {
	/// Wraps a player's wearables, or returns [`WearableError::NotTfPlayer`]
	/// unless the server runs TF2 and `player`'s datamaps include `CTFPlayer`.
	pub fn new(server: Server<'s>, player: Entity<'s>) -> Result<Self, WearableError> {
		if server.game() != Game::TeamFortress2 || !player.has_data_map_class(c"CTFPlayer") {
			return Err(WearableError::NotTfPlayer);
		}

		Ok(Self { server, player })
	}

	/// Equips a validated wearable through the player's `EquipWearable`, and
	/// discards it unless it ends up owned by, parented to and listed for the
	/// player.
	///
	/// # Safety
	///
	/// The game must not be iterating the player's wearables, as for
	/// [`Self::equip`].
	unsafe fn attach(self, wearable: Wearable<'s>) -> Result<(), WearableError> {
		// SAFETY: The caller guarantees that the game is not iterating the list.
		unsafe { self.equip_native(wearable) };

		match self.is_attached(wearable) {
			Ok(true) => Ok(()),

			attached => {
				// `Equip` hands a wearable `CanEquip` refuses to `RemoveWearable`,
				// which deletes it, but may remove a null entry instead.
				// SAFETY: As above.
				let discarded = unsafe { self.discard(wearable) };

				attached?;
				discarded?;
				Err(WearableError::Rejected)
			}
		}
	}

	/// Refuses a player marked for deletion, dead, or not on a playing team.
	fn check_playing(self) -> Result<(), WearableError> {
		check_live(self.player)?;

		let manager = self.server.player_info_manager()?;
		let info = self
			.player
			.edict()
			.and_then(|edict| manager.player_info(edict))
			.ok_or(WearableError::NotTfPlayer)?;

		if info.is_dead() || info.team() < FIRST_GAME_TEAM {
			return Err(WearableError::PlayerNotPlaying);
		}

		Ok(())
	}

	/// Unequips and deletes a wearable through `RemoveWearable`, called until
	/// the wearable leaves the list, or through [`ServerTools::remove`] if the
	/// game never removes or deletes it, which skips its `UnEquip`.
	///
	/// # Safety
	///
	/// As for [`Self::attach`].
	unsafe fn discard(self, wearable: Wearable<'s>) -> Result<(), WearableError> {
		let tools = self.server.server_tools()?;
		let mut result = Ok(());

		for _ in 0..MAX_REMOVE_ATTEMPTS {
			match self.list() {
				Ok(list) if list.contains(wearable) => {
					// SAFETY: The caller guarantees that the game is not iterating
					// the list.
					unsafe { self.remove_native(wearable) };
				}

				Ok(_) => break,

				Err(error) => {
					result = Err(error);
					break;
				}
			}
		}

		// `RemoveWearable` marked the wearable for deletion if it reached it,
		// which this then leaves alone. One it never reached stays listed until
		// the game drops its null entry after it is freed.
		let _ = tools.remove(wearable.entity);

		result
	}

	/// Equips a spawned wearable that no other entity owns, such as one the
	/// caller created and spawned with [`ServerTools`], and marks it validated
	/// first, as [`Self::give`] does. A wearable already in this player's list
	/// is left alone.
	///
	/// Fails with [`WearableError::DifferentOwner`] if another entity owns the
	/// wearable, [`WearableError::Full`] if the player's list is full, and
	/// [`WearableError::PlayerNotPlaying`] for a dead player or one not on a
	/// playing team. If the game refuses the wearable, it is deleted, as the
	/// game itself does, and [`WearableError::Rejected`] is returned.
	///
	/// # Safety
	///
	/// It must not be called while the game iterates this player's wearables,
	/// such as from a hook on `EquipWearable`, `RemoveWearable` or an item's
	/// `UnEquip`, as the
	/// [module documentation](crate::tf2::wearables#wearable-loops) describes.
	#[doc(alias = "EquipWearable")]
	pub unsafe fn equip(self, wearable: Wearable<'s>) -> Result<(), WearableError> {
		self.check_playing()?;
		check_live(wearable.entity)?;

		let list = self.list()?;

		if list.contains(wearable) {
			return Ok(());
		}

		if wearable
			.owner()?
			.is_some_and(|owner| owner != self.player.handle())
		{
			return Err(WearableError::DifferentOwner);
		}

		if list.is_full() {
			return Err(WearableError::Full);
		}

		wearable.set_validated(true)?;

		// SAFETY: The caller guarantees that the game is not iterating the list.
		unsafe { self.attach(wearable) }
	}

	/// Calls the player's `EquipWearable`.
	///
	/// # Safety
	///
	/// As for [`Self::attach`].
	unsafe fn equip_native(self, wearable: Wearable<'s>) {
		let player = self.player.as_ptr().cast::<sys::CTFPlayer>();

		// SAFETY: `new` verified `CTFPlayer`, whose entity base is at offset zero,
		// and `Wearable::new` verified `CTFWearable`, whose `CEconWearable` base
		// is at offset zero, as asserted above. Both are live. `EquipWearable`
		// adds the wearable to the head of the list, which the caller guarantees
		// the game is not iterating, and runs its `Equip`, which deletes only
		// through `UTIL_Remove`.
		unsafe {
			vcall!(player as sys::CTFPlayer__bindgen_vtable => CTFPlayer_EquipWearable(
				wearable.entity.as_ptr().cast(),
			));
		}
	}

	/// Creates a wearable from its economy item definition, marks it validated
	/// for other players' clients, and equips it. Native item generation
	/// initializes the item's definition, schema attributes and model before
	/// spawning it, with Unique quality and level 1, as for
	/// [`PlayerWeapons::give_item`](crate::tf2::weapons::PlayerWeapons::give_item).
	///
	/// Fails before creating anything with [`WearableError::PlayerNotPlaying`]
	/// for a dead player or one not on a playing team, and with
	/// [`WearableError::Full`] if the player's list is full. A definition that
	/// native generation cannot create, such as a missing one, fails with
	/// [`WearableError::CreationFailedNative`], and one that creates no TF2
	/// wearable, such as a weapon, with [`WearableError::NotWearable`]. The
	/// game refuses items restricted to a holiday outside it, which fails with
	/// [`WearableError::Rejected`]. An item the player's class has no model
	/// for, such as an item restricted to other classes, fails with
	/// [`WearableError::MissingModel`]: equipped on TF2's 64-bit Windows
	/// server, such an item was observed to draw nothing on its owner's retail
	/// client while still hiding parts of the player's model. Whatever was
	/// created is deleted on failure.
	///
	/// See the [module documentation](crate::tf2::wearables) for what keeps
	/// the wearable, and who sees it.
	///
	/// # Safety
	///
	/// As for [`PlayerWeapons::give_item`]: the definition's constructor,
	/// spawn and activation, and the callbacks equipping runs, such as its
	/// attribute providers and upgrades, must uphold [`Server::new`]'s
	/// no-immediate-deletion contract. It must not be called while the game
	/// iterates this player's wearables, such as from a hook on
	/// `EquipWearable`, `RemoveWearable` or an item's `UnEquip`, as the
	/// [module documentation](crate::tf2::wearables#wearable-loops) describes.
	///
	/// [`PlayerWeapons::give_item`]: crate::tf2::weapons::PlayerWeapons::give_item
	#[doc(alias = "SpawnItem")]
	#[doc(alias = "EquipWearable")]
	pub unsafe fn give(
		self,
		definition: ItemDefinitionIndex,
	) -> Result<Wearable<'s>, WearableError> {
		// SAFETY: The caller vouches for the native creation path. The generator
		// initializes the item view before `Spawn` and `Activate` and returns a
		// fresh callback-live entity, which must not be spawned again.
		let wearable = unsafe {
			self.give_with(|origin| {
				sdk_raw::tf2::item_generation::spawn(
					self.server.game_server_factory().as_raw(),
					definition.get(),
					origin.into(),
					None,
				)
				.map_err(WearableError::CreationFailedNative)
			})
		}?;

		// Equipping chose the model for the player's class, which is missing for
		// a class the item has no model for.
		if wearable.has_error_model()? {
			// SAFETY: As above: the caller's guarantees cover removing the item
			// this call equipped.
			let _ = unsafe { self.remove(wearable) };

			return Err(WearableError::MissingModel);
		}

		Ok(wearable)
	}

	/// Creates a wearable with `create`, given the player's position, then
	/// validates and equips it.
	///
	/// # Safety
	///
	/// `create` must return a newly created entity, live through this
	/// callback. The guarantees of [`Self::give`] apply to it.
	unsafe fn give_with(
		self,
		create: impl FnOnce(Vector) -> Result<NonNull<sys::CBaseEntity>, WearableError>,
	) -> Result<Wearable<'s>, WearableError> {
		self.check_playing()?;

		let tools = self.server.server_tools()?;

		// Refused before creation, so that nothing is left to delete.
		if self.list()?.is_full() {
			return Err(WearableError::Full);
		}

		let origin = self.player.position().ok_or(WearableError::MissingOrigin)?;

		// SAFETY: The caller guarantees a newly created callback-live entity.
		let entity = unsafe { Entity::from_raw(create(origin)?) };

		let wearable = match Wearable::new(self.server, entity) {
			Ok(wearable) => wearable,

			Err(error) => {
				let _ = tools.remove(entity);
				return Err(error);
			}
		};

		// Clients cache whether they draw the item once it is attached, so the
		// flag must be set before the first snapshot that attaches it.
		if let Err(error) = check_live(entity).and_then(|()| wearable.set_validated(true)) {
			let _ = tools.remove(entity);
			return Err(error);
		}

		// SAFETY: The caller upholds `give`'s guarantees, which rule out the game
		// iterating the list.
		unsafe { self.attach(wearable) }?;
		Ok(wearable)
	}

	/// Whether the wearable is equipped as `EquipWearable` leaves it: not
	/// marked for deletion, owned by the player, below the player in the move
	/// hierarchy, and in the player's list.
	fn is_attached(self, wearable: Wearable<'s>) -> Result<bool, WearableError> {
		Ok(!wearable.entity.is_marked_for_deletion()
			&& wearable.owner()? == Some(self.player.handle())
			&& self.is_parented(wearable)?
			&& self.list()?.contains(wearable))
	}

	/// Whether the player is the wearable's move parent, or that of the view
	/// model a view model wearable follows. Clients draw an attached item only
	/// then.
	fn is_parented(self, wearable: Wearable<'s>) -> Result<bool, WearableError> {
		let tools = self.server.server_tools()?;
		let player = self.player.handle();
		let mut entity = wearable.entity;

		for _ in 0..MAX_PARENT_DEPTH {
			// SAFETY: `Wearable::new` found the 4-byte `EHANDLE` field in
			// `CBaseEntity`'s own datamap, whose fields every entity shares at the
			// same offsets.
			let parent = unsafe { read_handle(entity, wearable.parent_offset) };

			if parent == player {
				return Ok(true);
			}

			match tools.entity_by_handle(parent) {
				Some(found) => entity = found,
				None => return Ok(false),
			}
		}

		Ok(false)
	}

	/// The player's wearables as clients receive them: newest first, at most
	/// [`MAX_NETWORKED_WEARABLES`] entries.
	///
	/// The list is read through the game's own send proxies. An entry whose
	/// entity no longer exists, or is not a TF2 wearable, is counted by
	/// [`WearableList::networked_len`], but not listed.
	///
	/// Fails with [`WearableError::UnsupportedLayout`] if the game networks the
	/// list in another shape, or if [`Wearable::new`] does for a listed
	/// wearable, and with [`WearableError::NetProp`] if the player is not
	/// networked or its class networks no `m_hMyWearables`.
	#[doc(alias = "m_hMyWearables")]
	#[doc(alias = "GetWearable")]
	pub fn list(self) -> Result<WearableList<'s>, WearableError> {
		check_live(self.player)?;

		let tools = self.server.server_tools()?;
		let vector = self
			.server
			.server_game_dll()?
			.entity_net_prop(self.player, c"m_hMyWearables")?;

		// `SendPropUtlVector` nests a table holding the length first, then one
		// element per entry it can send, named by index.
		if vector.prop().kind() != PropKind::DataTable
			|| vector.element_count() != Some(MAX_NETWORKED_WEARABLES + 1)
		{
			return Err(WearableError::UnsupportedLayout);
		}

		let mut elements = [vector; MAX_NETWORKED_WEARABLES];

		for ((slot, name), element) in ELEMENT_NAMES.into_iter().enumerate().zip(&mut elements) {
			*element = vector.element(slot + 1).map_err(layout_error)?;

			if element.prop().name() != name || element.prop().kind() != PropKind::Int {
				return Err(WearableError::UnsupportedLayout);
			}
		}

		let mut list = WearableList {
			entries: [None; MAX_NETWORKED_WEARABLES],
			networked_len: 0,
		};

		for (slot, element) in elements.into_iter().enumerate() {
			let NetValue::Int(raw) = element.value(self.player).map_err(layout_error)? else {
				return Err(WearableError::UnsupportedLayout);
			};

			// `SendProxy_UtlVectorElement` sends zero past the end of the list.
			if raw == 0 {
				break;
			}

			list.networked_len += 1;

			let Some(entity) =
				NetworkedHandle::decode(raw).and_then(|handle| handle.resolve(tools))
			else {
				continue;
			};

			list.entries[slot] = match Wearable::new(self.server, entity) {
				Ok(wearable) => Some(wearable),

				// The list holds any `CEconWearable`, such as a bare `wearable_item`.
				Err(WearableError::NotWearable) => None,

				Err(error) => return Err(error),
			};
		}

		Ok(list)
	}

	/// The player whose wearables these are.
	pub const fn player(self) -> Entity<'s> {
		self.player
	}

	/// Unequips a wearable in this player's list and deletes it, deferred,
	/// through the player's `RemoveWearable`, as the game removes wearables
	/// itself. Its `UnEquip` undoes what equipping did, showing the player's
	/// bodygroups it covered again, and clearing a shield's equipped state or a
	/// canteen's effect.
	///
	/// Fails with [`WearableError::NotEquipped`] if the wearable is not in this
	/// player's list. `RemoveWearable` removes the first null entry it meets
	/// from the end of the list instead of the wearable, so it is called again
	/// until it reaches the wearable. If it never does, as when another plugin
	/// intercepts it, the wearable is deleted through [`ServerTools::remove`]
	/// instead and drops out of the list once freed. That skips its `UnEquip`,
	/// so the hidden bodygroups, shield state or canteen effect then stay until
	/// the game next resets them.
	///
	/// # Safety
	///
	/// As for [`Self::equip`]: it must not be called while the game iterates
	/// this player's wearables.
	#[doc(alias = "RemoveWearable")]
	pub unsafe fn remove(self, wearable: Wearable<'s>) -> Result<(), WearableError> {
		check_live(self.player)?;
		check_live(wearable.entity)?;

		if !self.list()?.contains(wearable) {
			return Err(WearableError::NotEquipped);
		}

		// SAFETY: The caller guarantees that the game is not iterating the list.
		unsafe { self.discard(wearable) }
	}

	/// Calls the player's `RemoveWearable`.
	///
	/// # Safety
	///
	/// As for [`Self::attach`].
	unsafe fn remove_native(self, wearable: Wearable<'s>) {
		let player = self.player.as_ptr().cast::<sys::CTFPlayer>();

		// SAFETY: As for `equip_native`. The wearable is never null, which the
		// game would match against null entries and unequip. `RemoveWearable`
		// changes the list, which the caller guarantees the game is not
		// iterating, and deletes through `UTIL_Remove`.
		unsafe {
			vcall!(player as sys::CTFPlayer__bindgen_vtable => CTFPlayer_RemoveWearable(
				wearable.entity.as_ptr().cast(),
			));
		}
	}

	/// Removes every listed wearable `filter` selects, as [`Self::remove`]
	/// does, and returns how many were removed.
	///
	/// `filter` sees the list as it was first read, and a wearable it selects
	/// that is no longer listed when its turn comes is skipped. Removal stops
	/// at the first failure, keeping the earlier removals.
	///
	/// No filter tells cosmetics apart: gameplay items worn in weapon slots,
	/// such as the Gunboats, are plain `tf_wearable`s too. Combine
	/// [`Wearable::definition`], [`Wearable::kind`] and
	/// [`Wearable::is_game_managed`] as needed.
	///
	/// # Safety
	///
	/// As for [`Self::equip`]: it must not be called while the game iterates
	/// this player's wearables.
	pub unsafe fn strip(
		self,
		mut filter: impl FnMut(Wearable<'s>) -> bool,
	) -> Result<usize, WearableError> {
		check_live(self.player)?;

		let mut selected = [None; MAX_NETWORKED_WEARABLES];

		for (slot, wearable) in selected.iter_mut().zip(self.list()?.iter()) {
			if filter(wearable) {
				*slot = Some(wearable);
			}
		}

		let mut removed = 0;

		for wearable in selected.into_iter().flatten() {
			if wearable.entity.is_marked_for_deletion() || !self.list()?.contains(wearable) {
				continue;
			}

			// SAFETY: The caller guarantees that the game is not iterating the
			// list.
			unsafe { self.discard(wearable) }?;
			removed += 1;
		}

		Ok(removed)
	}
}

/// A callback-scoped TF2 wearable. Keep its entity handle, or better its
/// definition, across callbacks.
#[doc(alias = "CTFWearable")]
#[derive(Debug, Clone, Copy)]
pub struct Wearable<'s> {
	server: Server<'s>,
	entity: Entity<'s>,
	owner_offset: usize,
	parent_offset: usize,
}

impl<'s> Wearable<'s> {
	/// Wraps a TF2 wearable. Returns [`WearableError::NotWearable`] unless the
	/// server runs TF2 and `entity`'s datamaps include `CTFWearable`, as every
	/// wearable in a player's list must, and
	/// [`WearableError::UnsupportedLayout`] if `CBaseEntity`'s datamap lacks a
	/// 4-byte `EHANDLE` `m_hOwnerEntity` or `m_hMoveParent` at a plausible
	/// offset.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, WearableError> {
		if server.game() != Game::TeamFortress2 || !entity.has_data_map_class(c"CTFWearable") {
			return Err(WearableError::NotWearable);
		}

		let handle_offset = |name| {
			entity
				.find_base_entity_field(name, sys::_fieldtypes_FIELD_EHANDLE, size_of::<u32>())
				.ok_or(WearableError::UnsupportedLayout)
		};

		Ok(Self {
			server,
			entity,
			owner_offset: handle_offset(c"m_hOwnerEntity")?,
			parent_offset: handle_offset(c"m_hMoveParent")?,
		})
	}

	/// The weapon this wearable belongs to (`m_hWeaponAssociatedWith`), such as
	/// the weapon whose extra wearable it is, or `None`.
	///
	/// Unlike [`Self::owner`], this finds the entity rather than returning its
	/// handle: the game networks only the low bits of the handle's serial
	/// number, so the full handle is that of the weapon found.
	#[doc(alias = "m_hWeaponAssociatedWith")]
	#[doc(alias = "GetWeaponAssociatedWith")]
	pub fn associated_weapon(self) -> Result<Option<Entity<'s>>, WearableError> {
		let tools = self.server.server_tools()?;

		Ok(self
			.associated_weapon_handle()?
			.and_then(|handle| handle.resolve(tools)))
	}

	/// `m_hWeaponAssociatedWith`, as networked.
	fn associated_weapon_handle(self) -> Result<Option<NetworkedHandle>, WearableError> {
		match self
			.net_prop(c"m_hWeaponAssociatedWith")?
			.value(self.entity)?
		{
			NetValue::Int(raw) => Ok(NetworkedHandle::decode(raw)),
			_ => Err(WearableError::UnsupportedLayout),
		}
	}

	/// The item's economy definition (`m_iItemDefinitionIndex`), or `None` if
	/// its item was never initialized, as for weapons' extra wearables.
	#[doc(alias = "m_iItemDefinitionIndex")]
	#[doc(alias = "GetItemDefIndex")]
	pub fn definition(self) -> Result<Option<ItemDefinitionIndex>, WearableError> {
		if !self.is_initialized()? {
			return Ok(None);
		}

		let index = self
			.net_prop(c"m_iItemDefinitionIndex")?
			.get::<u16>(self.entity)?;

		Ok(ItemDefinitionIndex::new(index))
	}

	/// The wearable's entity.
	pub const fn entity(self) -> Entity<'s> {
		self.entity
	}

	/// Whether the wearable's model is the engine's stand-in for a missing
	/// model file, as for an item the wearer's class has no model for. The
	/// wearer's own client was observed to draw nothing for such an item.
	pub fn has_error_model(self) -> Result<bool, WearableError> {
		check_live(self.entity)?;

		// The model index is a 16-bit `short`, read as clients receive it.
		let NetValue::Int(index) = self.net_prop(c"m_nModelIndex")?.value(self.entity)? else {
			return Err(WearableError::UnsupportedLayout);
		};

		Ok(self.server.model_info()?.is_error_model(index))
	}

	/// Whether this is one of a disguised Spy's disguise wearables
	/// (`m_bDisguiseWearable`).
	#[doc(alias = "m_bDisguiseWearable")]
	#[doc(alias = "IsDisguiseWearable")]
	pub fn is_disguise(self) -> Result<bool, WearableError> {
		Ok(self
			.net_prop(c"m_bDisguiseWearable")?
			.get::<bool>(self.entity)?)
	}

	/// Whether the game creates and removes this wearable itself, along with
	/// something else: a weapon's extra wearable, which has an
	/// [associated weapon](Self::associated_weapon) or an uninitialized item,
	/// or a [disguise wearable](Self::is_disguise).
	///
	/// Gameplay items worn in weapon slots, such as the Gunboats or a
	/// Demoman's shield, are not game managed.
	pub fn is_game_managed(self) -> Result<bool, WearableError> {
		Ok(self.associated_weapon_handle()?.is_some()
			|| self.is_disguise()?
			|| !self.is_initialized()?)
	}

	/// Whether the item view was initialized from a definition
	/// (`m_bInitialized`).
	fn is_initialized(self) -> Result<bool, WearableError> {
		Ok(self.net_prop(c"m_bInitialized")?.get::<bool>(self.entity)?)
	}

	/// Whether other players' clients draw the wearable on its owner even if
	/// the owner's inventory lacks it (`m_bValidatedAttachedEntity`).
	#[doc(alias = "m_bValidatedAttachedEntity")]
	pub fn is_validated(self) -> Result<bool, WearableError> {
		Ok(self
			.net_prop(c"m_bValidatedAttachedEntity")?
			.get::<bool>(self.entity)?)
	}

	/// The kind of wearable, by its entity's class name.
	pub fn kind(self) -> WearableKind {
		WearableKind::from_class_name(self.entity.class_name())
	}

	/// Resolves one of the wearable's networked variables.
	fn net_prop(self, name: &CStr) -> Result<NetProp<'s>, WearableError> {
		Ok(self
			.server
			.server_game_dll()?
			.entity_net_prop(self.entity, name)?)
	}

	/// The entity that owns the wearable (`m_hOwnerEntity`), which is the
	/// player wearing it, or `None` once it is unequipped. Fails with
	/// [`WearableError::MarkedForDeletion`] for a wearable marked for deletion.
	#[doc(alias = "m_hOwnerEntity")]
	#[doc(alias = "GetOwnerEntity")]
	pub fn owner(self) -> Result<Option<EntityHandle>, WearableError> {
		check_live(self.entity)?;

		// SAFETY: `new` found the 4-byte `EHANDLE` field in `CBaseEntity`'s own
		// datamap.
		let handle = unsafe { read_handle(self.entity, self.owner_offset) };

		Ok(handle.is_valid().then_some(handle))
	}

	/// Sets whether other players' clients draw the wearable on its owner
	/// even if the owner's inventory lacks it (`m_bValidatedAttachedEntity`),
	/// as the game does for weapons picked up from others.
	///
	/// Clients decide once, when they first see the wearable attached, so set
	/// this before equipping it, in the same callback.
	#[doc(alias = "m_bValidatedAttachedEntity")]
	#[doc(alias = "MarkAttachedEntityAsValidated")]
	pub fn set_validated(self, validated: bool) -> Result<(), WearableError> {
		let engine = self.server.valve_engine()?;
		let prop = self.net_prop(c"m_bValidatedAttachedEntity")?;

		// SAFETY: Only clients read the flag, to decide whether to draw the
		// item (`CEconEntity::ValidateEntityAttachedToPlayer`, compiled for the
		// client only), so the server accepts either value.
		unsafe { prop.set(engine, self.entity, validated) }?;

		Ok(())
	}
}

/// Why a wearable operation failed.
#[derive(Debug, thiserror::Error)]
pub enum WearableError {
	/// Native item generation, used by [`PlayerWearables::give`], failed, for
	/// example because the definition does not exist.
	#[error(transparent)]
	CreationFailedNative(#[from] WeaponCreationFailed),

	/// Another entity owns the wearable.
	#[error("the wearable is owned by another entity")]
	DifferentOwner,

	/// The game already networks [`MAX_NETWORKED_WEARABLES`] entries of the
	/// player's list, so equipping another would push the oldest out of them.
	#[error("the player already wears as many items as the game networks")]
	Full,

	/// A required engine interface is unavailable.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// The player or wearable is already marked for deletion.
	#[error("the entity is marked for deletion")]
	MarkedForDeletion,

	/// The item's model for the player's class is missing, as for an item
	/// restricted to other classes, so the player's own client was observed
	/// to draw nothing for it.
	#[error("the item has no model for the player's class")]
	MissingModel,

	/// The player's position, needed to create an item, is unavailable.
	#[error("the player's absolute position could not be read")]
	MissingOrigin,

	/// A networked variable could not be read or written.
	#[error(transparent)]
	NetProp(#[from] NetPropError),

	/// The wearable is not in this player's list.
	#[error("the wearable is not in this player's wearable list")]
	NotEquipped,

	/// The entity is not a connected TF2 player, or the server does not run
	/// TF2.
	#[error("wearable operations require a TF2 player")]
	NotTfPlayer,

	/// The entity is not a TF2 wearable (for example, it is a weapon), or the
	/// server does not run TF2.
	#[error("the entity is not a TF2 wearable")]
	NotWearable,

	/// The player is dead, or not on a playing team.
	#[error("the player is dead or not on a playing team")]
	PlayerNotPlaying,

	/// The wearable did not end up equipped: the game refused it, as it does
	/// items restricted to a holiday outside it, and the wearable was deleted.
	#[error("the game refused to equip the wearable")]
	Rejected,

	/// The player's networked wearable list, or `CBaseEntity`'s datamap, is
	/// not laid out as expected.
	#[error("the game does not describe wearables in the expected layout")]
	UnsupportedLayout,
}

/// The kind of a [`Wearable`], by its entity's class name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WearableKind {
	/// `tf_wearable_campaign_item`, a campaign's item.
	#[doc(alias = "CTFWearableCampaignItem")]
	Campaign,

	/// `tf_wearable_demoshield`, a Demoman's shield, such as the Chargin'
	/// Targe.
	#[doc(alias = "CTFWearableDemoShield")]
	DemoShield,

	/// `tf_wearable_levelable_item`, an item that levels up.
	#[doc(alias = "CTFWearableLevelableItem")]
	Levelable,

	/// Any other class name.
	Other,

	/// `tf_wearable`: cosmetics, and gameplay items worn in weapon slots, such
	/// as the Gunboats or the Mantreads.
	#[doc(alias = "CTFWearable")]
	Plain,

	/// `tf_powerup_bottle`, Mann vs. Machine's Power Up Canteen.
	#[doc(alias = "CTFPowerupBottle")]
	PowerupBottle,

	/// `tf_wearable_razorback`, the Sniper's Razorback.
	#[doc(alias = "CTFWearableRazorback")]
	Razorback,

	/// `tf_wearable_robot_arm`, the Engineer's Gunslinger arm.
	#[doc(alias = "CTFWearableRobotArm")]
	RobotArm,

	/// `tf_wearable_vm`, drawn on the player's view model, such as a weapon's
	/// extra view model wearable.
	#[doc(alias = "CTFWearableVM")]
	ViewModel,
}

impl WearableKind {
	/// The kind of a wearable whose entity has `class_name`.
	pub fn from_class_name(class_name: &CStr) -> Self {
		match class_name.to_bytes() {
			b"tf_powerup_bottle" => Self::PowerupBottle,
			b"tf_wearable" => Self::Plain,
			b"tf_wearable_campaign_item" => Self::Campaign,
			b"tf_wearable_demoshield" => Self::DemoShield,
			b"tf_wearable_levelable_item" => Self::Levelable,
			b"tf_wearable_razorback" => Self::Razorback,
			b"tf_wearable_robot_arm" => Self::RobotArm,
			b"tf_wearable_vm" => Self::ViewModel,
			_ => Self::Other,
		}
	}
}

/// A player's networked wearables, newest first, from
/// [`PlayerWearables::list`].
#[derive(Debug, Clone, Copy)]
pub struct WearableList<'s> {
	entries: [Option<Wearable<'s>>; MAX_NETWORKED_WEARABLES],
	networked_len: usize,
}

impl<'s> WearableList<'s> {
	/// Whether the list holds the wearable's entity.
	pub fn contains(&self, wearable: Wearable<'_>) -> bool {
		self.iter()
			.any(|listed| listed.entity.as_ptr() == wearable.entity.as_ptr())
	}

	/// Whether the list holds no wearable.
	pub fn is_empty(&self) -> bool {
		self.len() == 0
	}

	/// Whether the game already networks [`MAX_NETWORKED_WEARABLES`] entries.
	/// Equipping another adds it at the head of the list, so the oldest entry
	/// would stop being networked, and clients would stop hiding its
	/// bodygroups and applying its attributes.
	pub fn is_full(&self) -> bool {
		self.networked_len >= MAX_NETWORKED_WEARABLES
	}

	/// The wearables, newest first.
	pub fn iter(&self) -> impl Iterator<Item = Wearable<'s>> + '_ {
		self.entries.iter().flatten().copied()
	}

	/// The number of wearables listed.
	pub fn len(&self) -> usize {
		self.iter().count()
	}

	/// The number of entries the game networks, including those whose entity
	/// no longer exists, which still take up room until the game drops them.
	pub fn networked_len(&self) -> usize {
		self.networked_len
	}
}

fn check_live(entity: Entity<'_>) -> Result<(), WearableError> {
	if entity.is_marked_for_deletion() {
		Err(WearableError::MarkedForDeletion)
	} else {
		Ok(())
	}
}

/// Converts an error about how `m_hMyWearables` is networked, rather than about
/// the player, into [`WearableError::UnsupportedLayout`].
fn layout_error(error: NetPropError) -> WearableError {
	match error {
		NetPropError::ElementOutOfRange { .. }
		| NetPropError::InvalidOffset { .. }
		| NetPropError::NoProxy { .. }
		| NetPropError::NotAValue { .. }
		| NetPropError::NotAnArray { .. }
		| NetPropError::Relocated { .. } => WearableError::UnsupportedLayout,

		error => WearableError::NetProp(error),
	}
}

/// Reads an entity handle at `offset`.
///
/// # Safety
///
/// `offset` must be that of a 4-byte `EHANDLE` field `CBaseEntity`'s own
/// datamap declares, which every entity shares at the same offset.
unsafe fn read_handle(entity: Entity<'_>, offset: usize) -> EntityHandle {
	// SAFETY: The caller guarantees the field, and the entity is live for the
	// callback. It is read without forming a reference, as the game writes it.
	EntityHandle::from_raw(unsafe { entity.as_ptr().byte_add(offset).cast::<u32>().read() })
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::Module;
	use crate::datatables::PropFlags;

	use crate::datatables::test_support::{
		direct_table, int8_proxy, int16_proxy, pointer_table, prop, proxies, table, table_prop,
	};

	use crate::edicts::test_support::mock_edict;
	use crate::entities::test_support::{MOCK_EFLAGS_OFFSET, data_map, field, leak};
	use crate::interfaces::{PlayerInfoManager, ServerGameDll, ValveEngine};
	use crate::server::test_support::{export, mock_server};
	use sdk_raw::util::mock::{mock_vtable, unexpected_call};
	use std::cell::{Cell, RefCell};
	use std::ffi::{c_char, c_void};
	use std::mem::MaybeUninit;
	use std::ptr::null_mut;

	const EQUIP: usize =
		offset_of!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_EquipWearable) / size_of::<usize>();

	/// A null handle, in a mock entity's fields and wearable list.
	const NULL: u32 = u32::MAX;

	const REMOVE: usize =
		offset_of!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_RemoveWearable) / size_of::<usize>();

	/// A player or wearable, whose datamaps and send tables describe these
	/// fields.
	#[repr(C)]
	struct FakeEntity {
		vtable: *const *const (),
		map: *mut sys::datamap_t,
		networkable: sys::IServerNetworkable,
		collideable: sys::ICollideable,
		flags: c_int,
		handle: u32,
		owner: u32,
		move_parent: u32,
		validated: bool,
		initialized: bool,
		disguise: bool,
		definition: u16,
		associated: u32,
		origin: sys::Vector,
		class: *mut sys::ServerClass,
		edict: *mut sys::edict_t,
		class_name: *const c_char,
		/// A player's `m_hMyWearables`, as raw handles.
		wearables: Vec<u32>,
		equip_calls: usize,
		remove_calls: usize,
	}

	thread_local! {
		static DEAD: Cell<bool> = const { Cell::new(false) };
		static ENTITIES: RefCell<Vec<(c_int, *mut FakeEntity)>> = const { RefCell::new(Vec::new()) };
		/// `gEntList`, which `ServerTools::entity_by_handle` reads.
		static ENTITY_LIST: Cell<*mut sys::CGlobalEntityList> = const { Cell::new(null_mut()) };
		/// What `EquipWearable` makes wearables follow instead of the player.
		static FOLLOW: Cell<Option<u32>> = const { Cell::new(None) };
		static GLOBALS: Cell<*mut sys::CGlobalVars> = const { Cell::new(null_mut()) };
		/// Makes `RemoveWearable` leave the list alone, as if another plugin
		/// intercepted it.
		static IGNORE_REMOVE: Cell<bool> = const { Cell::new(false) };
		static PLAYER_INFO: Cell<*mut sys::IPlayerInfo> = const { Cell::new(null_mut()) };
		static PROXIES: Cell<*mut sys::CStandardSendProxies> = const { Cell::new(null_mut()) };
		/// Makes `EquipWearable` emulate `CanEquip` refusing the wearable.
		static REFUSE_EQUIP: Cell<bool> = const { Cell::new(false) };
		static TEAM: Cell<c_int> = const { Cell::new(FIRST_GAME_TEAM) };
		static TOOL_REMOVALS: Cell<usize> = const { Cell::new(0) };
		static VALIDATED_AT_EQUIP: Cell<Option<bool>> = const { Cell::new(None) };
	}

	/// Mock interfaces exported on this thread, the classes of its fake
	/// entities, and its player.
	struct World {
		vtable: *const *const (),
		networkable: *const sys::IServerNetworkable__bindgen_vtable,
		collideable: *const sys::ICollideable__bindgen_vtable,
		base_map: *mut sys::datamap_t,
		wearable_map: *mut sys::datamap_t,
		wearable_class: *mut sys::ServerClass,
		/// The player's `m_hMyWearables` property, nesting `elements_table`.
		wearables_prop: *mut sys::SendProp,
		elements_table: *mut sys::SendTable,
		/// The `m_hMyWearables` table's properties: the length table, then
		/// `000` to `007`.
		elements: *mut [sys::SendProp; MAX_NETWORKED_WEARABLES + 1],
		player: *mut FakeEntity,
	}

	impl World {
		fn new() -> Self {
			let slot = |field: usize| field / size_of::<usize>();
			let mut vtable = vec![
				unexpected_call as *const ();
				EQUIP
					.max(REMOVE)
					.max(sdk_raw::entities::GET_DATA_DESC_MAP_SLOT)
					+ 1
			];

			vtable[sdk_raw::entities::GET_DATA_DESC_MAP_SLOT] = datamap as *const ();
			vtable[slot(offset_of!(
				sys::IServerUnknown__bindgen_vtable,
				IServerUnknown_GetRefEHandle
			))] = handle as *const ();
			vtable[slot(offset_of!(
				sys::IServerUnknown__bindgen_vtable,
				IServerUnknown_GetNetworkable
			))] = networkable as *const ();
			vtable[slot(offset_of!(
				sys::IServerUnknown__bindgen_vtable,
				IServerUnknown_GetCollideable
			))] = collideable as *const ();
			vtable[EQUIP] = equip_wearable as *const ();
			vtable[REMOVE] = remove_wearable as *const ();

			let networkable = Box::leak(unsafe {
				mock_vtable::<sys::IServerNetworkable__bindgen_vtable>(
					unexpected_call as *const (),
					|vtable| {
						(&raw mut (*vtable).IServerNetworkable_GetClassName).write(class_name);
						(&raw mut (*vtable).IServerNetworkable_GetServerClass).write(server_class);
						(&raw mut (*vtable).IServerNetworkable_GetEdict).write(edict);
					},
				)
			});
			let collideable = Box::leak(unsafe {
				mock_vtable::<sys::ICollideable__bindgen_vtable>(
					unexpected_call as *const (),
					|vtable| {
						(&raw mut (*vtable).ICollideable_GetCollisionOrigin).write(origin);
					},
				)
			});

			let base_map = data_map(
				c"CBaseEntity",
				vec![
					flags_field(),
					handle_field(c"m_hOwnerEntity", offset_of!(FakeEntity, owner)),
					handle_field(c"m_hMoveParent", offset_of!(FakeEntity, move_parent)),
				],
				null_mut(),
			);
			let player_map = data_map(
				c"CTFPlayer",
				vec![],
				data_map(c"CBasePlayer", vec![], base_map),
			);
			let wearable_map = data_map(
				c"CTFWearable",
				vec![],
				data_map(c"CEconWearable", vec![], base_map),
			);

			let wearable_props = leak([
				int_prop(
					c"m_bValidatedAttachedEntity",
					offset_of!(FakeEntity, validated),
					PropFlags::UNSIGNED,
					Some(int8_proxy),
				),
				int_prop(
					c"m_bInitialized",
					offset_of!(FakeEntity, initialized),
					PropFlags::UNSIGNED,
					Some(int8_proxy),
				),
				int_prop(
					c"m_bDisguiseWearable",
					offset_of!(FakeEntity, disguise),
					PropFlags::UNSIGNED,
					Some(int8_proxy),
				),
				int_prop(
					c"m_iItemDefinitionIndex",
					offset_of!(FakeEntity, definition),
					PropFlags::UNSIGNED,
					Some(int16_proxy),
				),
				int_prop(
					c"m_hWeaponAssociatedWith",
					offset_of!(FakeEntity, associated),
					PropFlags::default(),
					Some(handle_proxy),
				),
			]);
			let wearable_table = leak(table(c"DT_TFWearable", unsafe { &mut *wearable_props }));

			let length_props = leak([int_prop(
				c"lengthprop8",
				0,
				PropFlags::UNSIGNED,
				Some(int8_proxy),
			)]);
			let length_table = leak(table(c"_LPT_m_hMyWearables_8", unsafe {
				&mut *length_props
			}));
			let elements: [sys::SendProp; MAX_NETWORKED_WEARABLES + 1] =
				std::array::from_fn(|index| match index.checked_sub(1) {
					None => table_prop(c"lengthproxy", 0, length_table, Some(pointer_table)),

					Some(element) => {
						let mut prop = int_prop(
							ELEMENT_NAMES[element],
							0,
							PropFlags::default(),
							Some(wearable_element),
						);

						// `SendPropUtlVector` stores each element's index here.
						prop.m_ElementStride = element as c_int;
						prop
					}
				});
			let elements = leak(elements);
			let elements_table = leak(table(c"_ST_m_hMyWearables_8", unsafe { &mut *elements }));
			let player_props = leak([table_prop(
				c"m_hMyWearables",
				0,
				elements_table,
				Some(direct_table),
			)]);
			let player_table = leak(table(c"DT_TFPlayer", unsafe { &mut *player_props }));

			let player_class = leak(sys::ServerClass {
				m_pNetworkName: c"CTFPlayer".as_ptr(),
				m_pTable: player_table,
				m_pNext: null_mut(),
				m_ClassID: 1,
				m_InstanceBaselineIndex: 0,
			});
			let wearable_class = leak(sys::ServerClass {
				m_pNetworkName: c"CTFWearable".as_ptr(),
				m_pTable: wearable_table,
				m_pNext: null_mut(),
				m_ClassID: 2,
				m_InstanceBaselineIndex: 0,
			});

			Self::export_interfaces();

			let mut world = Self {
				vtable: vtable.leak().as_ptr(),
				networkable,
				collideable,
				base_map,
				wearable_map,
				wearable_class,
				wearables_prop: player_props.cast(),
				elements_table,
				elements,
				player: null_mut(),
			};

			world.player = world.spawn(1, 1, player_map, player_class, c"player");
			world
		}

		/// Exports the engine and game interfaces the module uses.
		fn export_interfaces() {
			PROXIES.set(leak(proxies(null_mut())));

			let globals =
				leak(MaybeUninit::<sys::CGlobalVars>::zeroed()).cast::<sys::CGlobalVars>();
			unsafe { (&raw mut (*globals)._base.maxClients).write(8) };
			GLOBALS.set(globals);

			let info = Box::leak(unsafe {
				mock_vtable::<sys::IPlayerInfo__bindgen_vtable>(
					unexpected_call as *const (),
					|vtable| {
						(&raw mut (*vtable).IPlayerInfo_IsDead).write(is_dead);
						(&raw mut (*vtable).IPlayerInfo_GetTeamIndex).write(team_index);
					},
				)
			});
			PLAYER_INFO.set(leak(sys::IPlayerInfo { vtable_: info }));

			let manager = Box::leak(unsafe {
				mock_vtable::<sys::IPlayerInfoManager__bindgen_vtable>(
					unexpected_call as *const (),
					|vtable| {
						(&raw mut (*vtable).IPlayerInfoManager_GetGlobalVars).write(global_vars);
						(&raw mut (*vtable).IPlayerInfoManager_GetPlayerInfo).write(player_info);
					},
				)
			});
			export(
				Module::GameServer,
				PlayerInfoManager::VERSION,
				leak(sys::IPlayerInfoManager { vtable_: manager }),
			);

			let dll = Box::leak(unsafe {
				mock_vtable::<sys::IServerGameDLL__bindgen_vtable>(
					unexpected_call as *const (),
					|vtable| {
						(&raw mut (*vtable).IServerGameDLL_GetStandardSendProxies)
							.write(standard_proxies);
					},
				)
			});
			export(
				Module::GameServer,
				ServerGameDll::VERSION,
				leak(sys::IServerGameDLL { vtable_: dll }),
			);

			ENTITY_LIST.set(Box::leak(Box::<sys::CGlobalEntityList>::new_zeroed()).as_mut_ptr());

			let tools = Box::leak(unsafe {
				mock_vtable::<sys::IServerTools__bindgen_vtable>(
					unexpected_call as *const (),
					|vtable| {
						(&raw mut (*vtable).IServerTools_GetBaseEntityByEntIndex)
							.write(entity_by_index);
						(&raw mut (*vtable).IServerTools_GetEntityList).write(entity_list);
						(&raw mut (*vtable).IServerTools_RemoveEntity).write(remove_entity);
					},
				)
			});
			export(
				Module::GameServer,
				ServerTools::VERSION,
				leak(sys::IServerTools { vtable_: tools }),
			);

			let engine = Box::leak(unsafe {
				mock_vtable::<sys::IVEngineServer__bindgen_vtable>(
					unexpected_call as *const (),
					|vtable| {
						(&raw mut (*vtable).IVEngineServer_GetChangeAccessor)
							.write(change_accessor);
						(&raw mut (*vtable).IVEngineServer_GetSharedEdictChangeInfo)
							.write(shared_change_info);
					},
				)
			});
			export(
				Module::Engine,
				ValveEngine::VERSION,
				leak(sys::IVEngineServer { vtable_: engine }),
			);
		}

		/// Creates an entity in edict slot `index`, which
		/// `GetBaseEntityByEntIndex` and the entity list find.
		fn spawn(
			&self,
			index: c_int,
			serial: u32,
			map: *mut sys::datamap_t,
			class: *mut sys::ServerClass,
			class_name: &'static CStr,
		) -> *mut FakeEntity {
			let fake = leak(FakeEntity {
				vtable: self.vtable,
				map,
				networkable: sys::IServerNetworkable {
					vtable_: self.networkable,
				},
				collideable: sys::ICollideable {
					vtable_: self.collideable,
				},
				flags: 0,
				handle: index.cast_unsigned() | serial << 16,
				owner: NULL,
				move_parent: NULL,
				validated: false,
				initialized: true,
				disguise: false,
				definition: 378,
				associated: NULL,
				origin: sys::Vector {
					x: 1.0,
					y: 2.0,
					z: 3.0,
				},
				class,
				edict: leak(mock_edict(index, false)),
				class_name: class_name.as_ptr(),
				wearables: Vec::new(),
				equip_calls: 0,
				remove_calls: 0,
			});

			ENTITIES.with_borrow_mut(|entities| entities.push((index, fake)));

			unsafe {
				let info = (&raw mut (*ENTITY_LIST.get())._base.m_EntPtrArray)
					.cast::<sys::CEntInfo>()
					.add(usize::try_from(index).unwrap());

				(&raw mut (*info).m_pEntity).write(fake.cast());
				(&raw mut (*info).m_SerialNumber).write(c_int::try_from(serial).unwrap());
			}

			fake
		}

		/// A spawned, unowned `tf_wearable` in edict slot `index`.
		fn wearable(&self, index: c_int) -> *mut FakeEntity {
			self.spawn(
				index,
				1,
				self.wearable_map,
				self.wearable_class,
				c"tf_wearable",
			)
		}
	}

	unsafe extern "C" fn change_accessor(
		_: *mut sys::IVEngineServer,
		_: *const sys::edict_t,
	) -> *mut sys::IChangeInfoAccessor {
		null_mut()
	}

	unsafe extern "C" fn class_name(this: *const sys::IServerNetworkable) -> *const c_char {
		unsafe { (*fake_of(this, offset_of!(FakeEntity, networkable))).class_name }
	}

	unsafe extern "C" fn collideable(entity: *mut sys::IServerUnknown) -> *mut sys::ICollideable {
		unsafe { &raw mut (*entity.cast::<FakeEntity>()).collideable }
	}

	unsafe extern "C" fn datamap(entity: *mut sys::CBaseEntity) -> *mut sys::datamap_t {
		unsafe { (*entity.cast::<FakeEntity>()).map }
	}

	unsafe extern "C" fn edict(this: *const sys::IServerNetworkable) -> *mut sys::edict_t {
		unsafe { (*fake_of(this, offset_of!(FakeEntity, networkable))).edict }
	}

	/// Encodes a raw handle as `SendProxy_EHandleToInt` does.
	fn encode(handle: u32) -> c_int {
		if handle == NULL {
			INVALID_NETWORKED_HANDLE.cast_signed()
		} else {
			((handle & 0x7FF) | ((handle >> 16) & 0x3FF) << 11).cast_signed()
		}
	}

	/// A callback-scoped entity for a fake one.
	fn entity<'s>(fake: *mut FakeEntity) -> Entity<'s> {
		unsafe { Entity::from_raw(NonNull::new(fake).unwrap().cast()) }
	}

	unsafe extern "C" fn entity_by_index(
		_: *mut sys::IServerTools,
		index: c_int,
	) -> *mut sys::CBaseEntity {
		ENTITIES.with_borrow(|entities| {
			entities
				.iter()
				.find(|(slot, _)| *slot == index)
				.map_or(null_mut(), |&(_, fake)| fake.cast())
		})
	}

	unsafe extern "C" fn entity_list(_: *mut sys::IServerTools) -> *mut sys::CGlobalEntityList {
		ENTITY_LIST.get()
	}

	#[test]
	fn equip_remove_and_strip_go_through_the_players_list() {
		let world = World::new();
		let scope = ();
		let server = mock_server(&scope);
		let player = world.player;
		let wearables = PlayerWearables::new(server, entity(player)).unwrap();
		let player_handle = unsafe { (*player).handle };
		let wear = |index| Wearable::new(server, entity(world.wearable(index))).unwrap();
		let fake = |wearable: Wearable<'_>| wearable.entity().as_ptr().cast::<FakeEntity>();

		// The mock game never iterates the list while these run.
		let equip = |wearable| unsafe { wearables.equip(wearable) };
		let remove = |wearable| unsafe { wearables.remove(wearable) };

		let owned = wear(2);
		unsafe { (*fake(owned)).owner = 9 | 1 << 16 };
		assert!(matches!(equip(owned), Err(WearableError::DifferentOwner)));

		let hat = wear(3);
		assert!(matches!(remove(hat), Err(WearableError::NotEquipped)));
		assert_eq!(unsafe { (*player).remove_calls }, 0);

		equip(hat).unwrap();
		equip(hat).unwrap();
		unsafe {
			assert!((*fake(hat)).validated);
			assert_eq!((*fake(hat)).owner, player_handle);
			assert_eq!((*player).equip_calls, 1);
		}

		// The game's removal takes a null entry behind the wearable first.
		unsafe { (*player).wearables.push(NULL) };
		remove(hat).unwrap();
		unsafe {
			assert_eq!((*fake(hat)).flags, 1);
			assert_eq!(((*fake(hat)).owner, (*fake(hat)).move_parent), (NULL, NULL));
			assert_eq!((*player).remove_calls, 2);
			assert!((*player).wearables.is_empty());
		}
		assert!(matches!(remove(hat), Err(WearableError::MarkedForDeletion)));
		assert!(matches!(equip(hat), Err(WearableError::MarkedForDeletion)));

		// Behind more null entries, it takes one call per entry.
		let removals = TOOL_REMOVALS.get();
		let behind = wear(4);
		equip(behind).unwrap();
		unsafe { (*player).wearables.extend([NULL; 3]) };
		remove(behind).unwrap();
		unsafe {
			assert_eq!((*fake(behind)).flags, 1);
			assert_eq!((*player).remove_calls, 6);
			assert!((*player).wearables.is_empty());
		}
		assert_eq!(TOOL_REMOVALS.get(), removals);

		// If the game never removes it, it is deleted through `ServerTools`.
		let stubborn = wear(15);
		equip(stubborn).unwrap();
		IGNORE_REMOVE.set(true);
		remove(stubborn).unwrap();
		IGNORE_REMOVE.set(false);
		assert_eq!(TOOL_REMOVALS.get(), removals + 1);
		unsafe {
			assert_eq!((*fake(stubborn)).flags, 1);
			assert_eq!((*player).remove_calls, 6 + MAX_REMOVE_ATTEMPTS);
			(*player).wearables = vec![NULL; MAX_NETWORKED_WEARABLES];
		}

		assert!(matches!(equip(wear(5)), Err(WearableError::Full)));
		unsafe { (*player).wearables.clear() };

		// Only what the filter selects is stripped.
		let cosmetic = wear(6);
		let disguise = wear(7);
		let extra = wear(8);
		let boots = wear(9);
		for wearable in [cosmetic, disguise, extra, boots] {
			equip(wearable).unwrap();
		}
		unsafe {
			(*fake(disguise)).disguise = true;
			(*fake(extra)).initialized = false;
			(*fake(boots)).definition = 133;
		}
		let mut seen = 0;
		let removed = unsafe {
			wearables.strip(|wearable| {
				seen += 1;
				!wearable.is_game_managed().unwrap()
					&& wearable.definition().unwrap() != ItemDefinitionIndex::new(133)
			})
		}
		.unwrap();
		assert_eq!((removed, seen), (1, 4));
		unsafe {
			assert_eq!((*fake(cosmetic)).flags, 1);
			assert_eq!(
				[
					(*fake(disguise)).flags,
					(*fake(extra)).flags,
					(*fake(boots)).flags
				],
				[0; 3]
			);
			assert_eq!((*player).wearables.len(), 3);
		}

		// Dead players are given nothing, but can still lose wearables.
		DEAD.set(true);
		assert!(matches!(
			equip(wear(10)),
			Err(WearableError::PlayerNotPlaying)
		));
		assert_eq!(unsafe { wearables.strip(|_| true) }.unwrap(), 3);
		assert!(wearables.list().unwrap().is_empty());
		DEAD.set(false);

		// A view model wearable follows the view model, which follows the player.
		let view_model = world.spawn(11, 1, world.base_map, world.wearable_class, c"tf_viewmodel");
		unsafe { (*view_model).move_parent = player_handle };
		FOLLOW.set(Some(unsafe { (*view_model).handle }));
		let sleeve = wear(12);
		equip(sleeve).unwrap();

		// Following anything else is not attached, and is undone.
		FOLLOW.set(Some(13 | 1 << 16));
		let detached = wear(14);
		assert!(matches!(equip(detached), Err(WearableError::Rejected)));
		FOLLOW.set(None);
		unsafe {
			assert_eq!((*fake(detached)).flags, 1);
			assert_eq!((*player).wearables, [(*fake(sleeve)).handle]);
		}
	}

	/// Emulates `CBasePlayer::EquipWearable` and `CEconWearable::Equip`.
	unsafe extern "C" fn equip_wearable(
		player: *mut sys::CTFPlayer,
		item: *mut sys::CEconWearable,
	) {
		unsafe {
			let player = player.cast::<FakeEntity>();
			let item = item.cast::<FakeEntity>();

			(*player).equip_calls += 1;
			VALIDATED_AT_EQUIP.set(Some((*item).validated));
			(*player).wearables.insert(0, (*item).handle);

			if REFUSE_EQUIP.get() {
				// `CanEquip` failed: `RemoveFrom` calls `RemoveWearable`.
				remove_wearable(player.cast(), item.cast());
			} else {
				(*item).owner = (*player).handle;
				(*item).move_parent = FOLLOW.get().unwrap_or((*player).handle);
			}
		}
	}

	/// The fake entity containing an interface at `offset`.
	fn fake_of<T>(interface: *const T, offset: usize) -> *mut FakeEntity {
		unsafe { interface.byte_sub(offset) }.cast_mut().cast()
	}

	/// The `m_iEFlags` field of `CBaseEntity`'s datamap.
	fn flags_field() -> sys::typedescription_t {
		let mut flags = field();

		flags.fieldType = sys::_fieldtypes_FIELD_INTEGER;
		flags.fieldName = c"m_iEFlags".as_ptr();
		flags.fieldOffset[0] = MOCK_EFLAGS_OFFSET as c_int;
		flags.fieldSizeInBytes = size_of::<c_int>() as c_int;
		flags
	}

	#[test]
	fn give_validates_before_equipping_and_deletes_what_the_game_refuses() {
		let world = World::new();
		let scope = ();
		let server = mock_server(&scope);
		let wearables = PlayerWearables::new(server, entity(world.player)).unwrap();
		let player_handle = unsafe { (*world.player).handle };
		let created = |fake: *mut FakeEntity| {
			move |origin| {
				assert_eq!(origin, Vector::new(1.0, 2.0, 3.0));
				Ok(NonNull::new(fake).unwrap().cast())
			}
		};

		let hat = world.wearable(2);
		let given = unsafe { wearables.give_with(created(hat)) }.unwrap();

		assert_eq!(given.entity().as_ptr(), hat.cast());
		assert_eq!(VALIDATED_AT_EQUIP.get(), Some(true));
		unsafe {
			assert_eq!(
				((*hat).owner, (*hat).move_parent, (*hat).flags),
				(player_handle, player_handle, 0)
			);
			assert_eq!((*world.player).equip_calls, 1);
		}
		assert!(wearables.list().unwrap().contains(given));

		// Nothing is created for a player who could not wear it.
		DEAD.set(true);
		assert!(matches!(
			unsafe { wearables.give_with(|_| unreachable!()) },
			Err(WearableError::PlayerNotPlaying)
		));
		DEAD.set(false);
		TEAM.set(1);
		assert!(matches!(
			unsafe { wearables.give_with(|_| unreachable!()) },
			Err(WearableError::PlayerNotPlaying)
		));
		TEAM.set(FIRST_GAME_TEAM);

		unsafe { (*world.player).wearables = vec![(*hat).handle; MAX_NETWORKED_WEARABLES] };
		assert!(matches!(
			unsafe { wearables.give_with(|_| unreachable!()) },
			Err(WearableError::Full)
		));
		unsafe { (*world.player).wearables = vec![(*hat).handle] };

		// A definition that creates a weapon is deleted unequipped.
		let weapon = world.spawn(
			3,
			1,
			data_map(c"CTFWeaponBase", vec![], world.base_map),
			world.wearable_class,
			c"tf_weapon_bottle",
		);
		assert!(matches!(
			unsafe { wearables.give_with(created(weapon)) },
			Err(WearableError::NotWearable)
		));
		assert_eq!(unsafe { (*weapon).flags }, 1);
		assert_eq!(unsafe { (*world.player).equip_calls }, 1);

		// `CanEquip` refusing a holiday item makes the game delete it already.
		let removals = TOOL_REMOVALS.get();
		let holiday = world.wearable(4);
		REFUSE_EQUIP.set(true);
		assert!(matches!(
			unsafe { wearables.give_with(created(holiday)) },
			Err(WearableError::Rejected)
		));
		unsafe {
			assert_eq!((*holiday).flags, 1);
			assert_eq!((*world.player).remove_calls, 1);
			assert_eq!((*world.player).wearables, [(*hat).handle]);
		}
		assert_eq!(TOOL_REMOVALS.get(), removals);

		// With a null entry at the end, the game's own removal takes the null
		// entry instead, leaving the refused item listed but unowned.
		let quirk = world.wearable(5);
		unsafe { (*world.player).wearables.push(NULL) };
		assert!(matches!(
			unsafe { wearables.give_with(created(quirk)) },
			Err(WearableError::Rejected)
		));
		REFUSE_EQUIP.set(false);
		unsafe {
			assert_eq!((*quirk).flags, 1);
			assert_eq!((*world.player).remove_calls, 3);
			assert_eq!((*world.player).wearables, [(*hat).handle]);
		}
		assert_eq!(TOOL_REMOVALS.get(), removals);

		unsafe { (*world.player).flags = 1 };
		assert!(matches!(
			unsafe { wearables.give_with(|_| unreachable!()) },
			Err(WearableError::MarkedForDeletion)
		));
		assert!(matches!(
			wearables.list(),
			Err(WearableError::MarkedForDeletion)
		));
	}

	unsafe extern "C" fn global_vars(_: *mut sys::IPlayerInfoManager) -> *mut sys::CGlobalVars {
		GLOBALS.get()
	}

	unsafe extern "C" fn handle(entity: *const sys::IServerUnknown) -> *const sys::CBaseHandle {
		unsafe { (&raw const (*entity.cast::<FakeEntity>()).handle).cast() }
	}

	/// An `EHANDLE` field of `CBaseEntity`'s datamap.
	fn handle_field(name: &'static CStr, offset: usize) -> sys::typedescription_t {
		let mut handle = field();

		handle.fieldType = sys::_fieldtypes_FIELD_EHANDLE;
		handle.fieldName = name.as_ptr();
		handle.fieldOffset[0] = offset as c_int;
		handle.fieldSize = 1;
		handle.fieldSizeInBytes = size_of::<u32>() as c_int;
		handle
	}

	/// Stands in for `SendProxy_EHandleToInt`.
	unsafe extern "C" fn handle_proxy(
		_: *const sys::SendProp,
		_: *const c_void,
		data: *const c_void,
		out: *mut sys::DVariant,
		_: c_int,
		_: c_int,
	) {
		unsafe { (*out).__bindgen_anon_1.m_Int = encode(data.cast::<u32>().read()) };
	}

	/// An integer property at `offset` into a fake entity.
	fn int_prop(
		name: &'static CStr,
		offset: usize,
		flags: PropFlags,
		proxy: sys::SendVarProxyFn,
	) -> sys::SendProp {
		prop(
			name,
			sys::SendPropType_DPT_Int,
			c_int::try_from(offset).unwrap(),
			flags,
			proxy,
		)
	}

	unsafe extern "C" fn is_dead(_: *mut sys::IPlayerInfo) -> bool {
		DEAD.get()
	}

	#[test]
	fn lists_decode_networked_handles_and_wearables_read_their_variables() {
		assert_eq!(offset_of!(FakeEntity, flags), MOCK_EFLAGS_OFFSET);

		let world = World::new();
		let scope = ();
		let server = mock_server(&scope);
		let player = entity(world.player);
		let first = world.wearable(2);
		let second = world.wearable(3);
		let stale = world.wearable(4);

		// Newest first, with a null entry and one whose serial number is stale.
		unsafe {
			(*world.player).wearables = vec![
				(*second).handle,
				NULL,
				(*stale).handle + (1 << 16),
				(*first).handle,
			];
		}

		let wearables = PlayerWearables::new(server, player).unwrap();
		let list = wearables.list().unwrap();

		assert_eq!((list.networked_len(), list.len()), (4, 2));
		assert!(!list.is_empty() && !list.is_full());
		assert_eq!(
			list.iter()
				.map(|wearable| wearable.entity().as_ptr())
				.collect::<Vec<_>>(),
			[second.cast(), first.cast()]
		);
		assert!(!list.contains(Wearable::new(server, entity(stale)).unwrap()));

		unsafe { (*world.player).wearables = vec![(*first).handle; MAX_NETWORKED_WEARABLES + 1] };
		let list = wearables.list().unwrap();
		assert!(list.is_full());
		assert_eq!(list.networked_len(), MAX_NETWORKED_WEARABLES);

		unsafe { (*world.player).wearables.clear() };
		assert!(wearables.list().unwrap().is_empty());

		// A list the game sends in another shape is refused.
		let refused = || matches!(wearables.list(), Err(WearableError::UnsupportedLayout));

		unsafe { (*world.elements)[4].m_pVarName = c"bad".as_ptr() };
		assert!(refused());
		unsafe { (*world.elements)[4].m_pVarName = ELEMENT_NAMES[3].as_ptr() };

		unsafe { (*world.elements)[4].m_Type = sys::SendPropType_DPT_Float };
		assert!(refused());
		unsafe { (*world.elements)[4].m_Type = sys::SendPropType_DPT_Int };

		unsafe { (*world.elements)[1].m_ProxyFn = None };
		assert!(refused());
		unsafe { (*world.elements)[1].m_ProxyFn = Some(wearable_element) };

		unsafe { (*world.elements_table).m_nProps -= 1 };
		assert!(refused());
		unsafe { (*world.elements_table).m_nProps += 1 };

		unsafe { (*world.wearables_prop).m_Type = sys::SendPropType_DPT_Int };
		assert!(refused());
		unsafe { (*world.wearables_prop).m_Type = sys::SendPropType_DPT_DataTable };

		// A table proxy that moves the elements elsewhere hides where they are.
		unsafe { (*world.wearables_prop).m_DataTableProxyFn = Some(pointer_table) };
		assert!(refused());
		unsafe { (*world.wearables_prop).m_DataTableProxyFn = Some(direct_table) };

		// Classes are checked, down to `CTFWearable`, and so is the game.
		let econ_only = world.spawn(
			5,
			1,
			data_map(c"CEconWearable", vec![], world.base_map),
			world.wearable_class,
			c"wearable_item",
		);

		// An entry that is no TF2 wearable takes room but is not listed, while
		// one whose entity data cannot be read fails the whole list.
		let no_handles = world.spawn(
			7,
			1,
			data_map(
				c"CTFWearable",
				vec![],
				data_map(c"CBaseEntity", vec![flags_field()], null_mut()),
			),
			world.wearable_class,
			c"tf_wearable",
		);
		unsafe { (*world.player).wearables = vec![(*econ_only).handle, (*first).handle] };
		let list = wearables.list().unwrap();
		assert_eq!((list.networked_len(), list.len()), (2, 1));
		unsafe { (*world.player).wearables.push((*no_handles).handle) };
		assert!(refused());
		unsafe { (*world.player).wearables.clear() };
		assert!(matches!(
			PlayerWearables::new(server, entity(first)),
			Err(WearableError::NotTfPlayer)
		));
		assert!(matches!(
			Wearable::new(server, player),
			Err(WearableError::NotWearable)
		));
		assert!(matches!(
			Wearable::new(server, entity(econ_only)),
			Err(WearableError::NotWearable)
		));
		let sdk_server = unsafe {
			Server::new(
				server.engine_factory(),
				server.game_server_factory(),
				Game::SourceSdk2013,
				&scope,
			)
		};
		assert!(matches!(
			PlayerWearables::new(sdk_server, player),
			Err(WearableError::NotTfPlayer)
		));
		assert!(matches!(
			Wearable::new(sdk_server, entity(first)),
			Err(WearableError::NotWearable)
		));

		let wearable = Wearable::new(server, entity(first)).unwrap();
		assert_eq!(wearable.owner().unwrap(), None);
		assert_eq!(wearable.kind(), WearableKind::Plain);
		assert_eq!(
			wearable.definition().unwrap(),
			ItemDefinitionIndex::new(378)
		);
		assert!(!wearable.is_game_managed().unwrap());
		assert!(!wearable.is_validated().unwrap());

		wearable.set_validated(true).unwrap();
		assert!(unsafe { (*first).validated });
		assert!(wearable.is_validated().unwrap());
		assert_ne!(unsafe { (*(*first).edict)._base.m_fStateFlags } & 1, 0);

		unsafe { (*first).definition = u16::MAX };
		assert_eq!(wearable.definition().unwrap(), None);

		// Weapons' extra wearables never initialize their item.
		unsafe {
			(*first).definition = 378;
			(*first).initialized = false;
		}
		assert_eq!(wearable.definition().unwrap(), None);
		assert!(wearable.is_game_managed().unwrap());

		unsafe {
			(*first).initialized = true;
			(*first).disguise = true;
		}
		assert!(wearable.is_disguise().unwrap());
		assert!(wearable.is_game_managed().unwrap());

		let weapon = world.spawn(
			6,
			3,
			world.base_map,
			world.wearable_class,
			c"tf_weapon_rocketlauncher",
		);
		unsafe {
			(*first).disguise = false;
			(*first).associated = (*weapon).handle;
		}
		assert_eq!(
			wearable.associated_weapon().unwrap().map(Entity::as_ptr),
			Some(weapon.cast())
		);
		assert!(wearable.is_game_managed().unwrap());

		unsafe { (*first).associated = (*weapon).handle + (1 << 16) };
		assert!(wearable.associated_weapon().unwrap().is_none());

		unsafe {
			(*first).associated = NULL;
			(*first).owner = (*world.player).handle;
		}
		assert!(!wearable.is_game_managed().unwrap());
		assert_eq!(wearable.owner().unwrap(), Some(player.handle()));

		unsafe { (*first).flags = 1 };
		assert!(matches!(
			wearable.owner(),
			Err(WearableError::MarkedForDeletion)
		));

		for (class_name, kind) in [
			(c"tf_wearable", WearableKind::Plain),
			(c"tf_wearable_vm", WearableKind::ViewModel),
			(c"tf_wearable_demoshield", WearableKind::DemoShield),
			(c"tf_wearable_razorback", WearableKind::Razorback),
			(c"tf_powerup_bottle", WearableKind::PowerupBottle),
			(c"tf_wearable_levelable_item", WearableKind::Levelable),
			(c"tf_wearable_campaign_item", WearableKind::Campaign),
			(c"tf_wearable_robot_arm", WearableKind::RobotArm),
			(c"wearable_item", WearableKind::Other),
		] {
			assert_eq!(WearableKind::from_class_name(class_name), kind);
		}
	}

	unsafe extern "C" fn networkable(
		entity: *mut sys::IServerUnknown,
	) -> *mut sys::IServerNetworkable {
		unsafe { &raw mut (*entity.cast::<FakeEntity>()).networkable }
	}

	#[test]
	fn networked_handles_keep_the_index_and_low_serial_bits() {
		let handle = NetworkedHandle::decode(5 | 3 << 11).unwrap();

		assert_eq!(handle.index(), 5);
		assert!(handle.matches(EntityHandle::from_raw(5 | 3 << 16)));
		assert!(handle.matches(EntityHandle::from_raw(5 | (3 + 1024) << 16)));
		assert!(!handle.matches(EntityHandle::from_raw(5 | 4 << 16)));
		assert!(!handle.matches(EntityHandle::from_raw(6 | 3 << 16)));
		assert!(!handle.matches(EntityHandle::INVALID));
		assert_eq!(encode(5 | (3 + 1024) << 16), 5 | 3 << 11);

		assert_eq!(
			NetworkedHandle::decode(2047 | 1022 << 11).map(NetworkedHandle::index),
			Some(2047)
		);

		for raw in [0, INVALID_NETWORKED_HANDLE.cast_signed(), 1 << 21, -1] {
			assert_eq!(NetworkedHandle::decode(raw), None);
		}
	}

	unsafe extern "C" fn origin(this: *const sys::ICollideable) -> *const sys::Vector {
		unsafe { &raw const (*fake_of(this, offset_of!(FakeEntity, collideable))).origin }
	}

	unsafe extern "C" fn player_info(
		_: *mut sys::IPlayerInfoManager,
		_: *mut sys::edict_t,
	) -> *mut sys::IPlayerInfo {
		PLAYER_INFO.get()
	}

	unsafe extern "C" fn remove_entity(_: *mut sys::IServerTools, entity: *mut sys::CBaseEntity) {
		unsafe { (*entity.cast::<FakeEntity>()).flags |= 1 };
		TOOL_REMOVALS.set(TOOL_REMOVALS.get() + 1);
	}

	/// Emulates `CBasePlayer::RemoveWearable`, which removes the first null
	/// entry it meets from the end instead of the wearable.
	unsafe extern "C" fn remove_wearable(
		player: *mut sys::CTFPlayer,
		item: *mut sys::CEconWearable,
	) {
		assert!(!item.is_null(), "RemoveWearable unequips null entries");

		unsafe {
			let player = player.cast::<FakeEntity>();
			let item = item.cast::<FakeEntity>();

			(*player).remove_calls += 1;

			if IGNORE_REMOVE.get() {
				return;
			}

			let wearables = &mut (*player).wearables;

			for index in (0..wearables.len()).rev() {
				let entry = wearables[index];

				if entry == (*item).handle {
					(*item).owner = NULL;
					(*item).move_parent = NULL;
					(*item).flags |= 1;
					wearables.remove(index);
					break;
				}

				if entry == NULL {
					wearables.remove(index);
					break;
				}
			}
		}
	}

	unsafe extern "C" fn server_class(this: *mut sys::IServerNetworkable) -> *mut sys::ServerClass {
		unsafe { (*fake_of(this, offset_of!(FakeEntity, networkable))).class }
	}

	unsafe extern "C" fn shared_change_info(
		_: *mut sys::IVEngineServer,
	) -> *mut sys::CSharedEdictChangeInfo {
		null_mut()
	}

	unsafe extern "C" fn standard_proxies(
		_: *mut sys::IServerGameDLL,
	) -> *mut sys::CStandardSendProxies {
		PROXIES.get()
	}

	unsafe extern "C" fn team_index(_: *mut sys::IPlayerInfo) -> c_int {
		TEAM.get()
	}

	/// Stands in for `SendProxy_UtlVectorElement` over a fake player's list.
	unsafe extern "C" fn wearable_element(
		prop: *const sys::SendProp,
		structure: *const c_void,
		_: *const c_void,
		out: *mut sys::DVariant,
		_: c_int,
		_: c_int,
	) {
		unsafe {
			let index = usize::try_from((*prop).m_ElementStride).unwrap();
			let wearables = &(*structure.cast::<FakeEntity>()).wearables;

			(*out).__bindgen_anon_1.m_Int =
				wearables.get(index).map_or(0, |&handle| encode(handle));
		}
	}
}
