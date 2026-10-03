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

#[cfg(test)]
#[path = "../tests/tf2/wearables.rs"]
mod tests;

use crate::datatables::{NetProp, NetPropError, NetValue, PropKind};
use crate::entities::{Entity, EntityHandle};
use crate::interfaces::ServerTools;
use crate::math::Vector;
use crate::tf2::weapons::{self, ItemDefinitionIndex, ItemGenerationError};
use crate::{Game, InterfaceError, Server};
use sdk_raw::edicts::MAX_EDICT_BITS;

use sdk_raw::entities::{
	INVALID_NETWORKED_EHANDLE_VALUE, NUM_NETWORKED_EHANDLE_BITS,
	NUM_NETWORKED_EHANDLE_SERIAL_NUMBER_BITS,
};

use sdk_raw::players::FIRST_GAME_TEAM;
use sdk_raw::vcall;
use std::ffi::{CStr, c_int};
use std::ptr::NonNull;

/// The names `SendPropUtlVector` gives the networked elements of
/// `m_hMyWearables`, in order (`DT_ArrayElementNameForIdx`).
const ELEMENT_NAMES: [&CStr; MAX_NETWORKED_WEARABLES] = [
	c"000", c"001", c"002", c"003", c"004", c"005", c"006", c"007",
];

/// `MAX_WEARABLES_SENT_FROM_SERVER` (TF2's `LOADOUT_MAX_WEARABLES_COUNT`): the
/// most entries of a player's wearable list the game networks.
#[doc(alias("MAX_WEARABLES_SENT_FROM_SERVER", "LOADOUT_MAX_WEARABLES_COUNT"))]
pub const MAX_NETWORKED_WEARABLES: usize = sdk_raw::tf2::wearables::MAX_WEARABLES_SENT_FROM_SERVER;

/// How far up the move hierarchy a wearable's player is looked for: a view
/// model wearable follows the player's view model, which follows the player.
const MAX_PARENT_DEPTH: usize = 8;

/// The most times `RemoveWearable` is called for one wearable before it is
/// deleted through [`ServerTools`] instead. While the wearable is listed, each
/// call removes one entry, the wearable or the first null entry it meets from
/// the end of the list, so it reaches the wearable within the list's length.
const MAX_REMOVE_ATTEMPTS: usize = 32;

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
			&& raw != INVALID_NETWORKED_EHANDLE_VALUE
			&& raw >> NUM_NETWORKED_EHANDLE_BITS == 0)
			.then_some(Self(raw))
	}

	/// The entity's slot in the entity list, which is its edict index.
	const fn index(self) -> usize {
		(self.0 & ((1 << MAX_EDICT_BITS) - 1)) as usize
	}

	/// Whether `handle` has this slot and the serial number bits sent.
	const fn matches(self, handle: EntityHandle) -> bool {
		let serial = handle.serial_number() & ((1 << NUM_NETWORKED_EHANDLE_SERIAL_NUMBER_BITS) - 1);

		matches!(handle.index(), Some(index) if index == self.index())
			&& serial == self.0 >> MAX_EDICT_BITS
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
	#[doc(alias("EquipWearable"))]
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

		// SAFETY: `new` found `CTFPlayer` in the player's datamaps, so it is
		// one, whose entity base `sdk_raw::tf2` asserts is at offset zero, and
		// `Wearable::new` found `CTFWearable` in the wearable's, whose entity
		// and `CEconWearable` bases `sdk_raw::tf2::wearables` asserts are at
		// offset zero. Both are live.
		// `EquipWearable` adds the wearable to the head of the list, which the
		// caller guarantees the game is not iterating, and runs its `Equip`,
		// which deletes only through `UTIL_Remove`.
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
	/// native generation cannot create, such as one the running schema lacks,
	/// fails with [`WearableError::CreationFailedNative`], and one that creates
	/// no TF2 wearable, such as a weapon, with [`WearableError::NotWearable`].
	/// The game refuses items restricted to a holiday outside it, which fails
	/// with [`WearableError::Rejected`]. An item the player's class has no
	/// model for, such as an item restricted to other classes, fails with
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
	#[doc(alias("SpawnItem", "EquipWearable"))]
	pub unsafe fn give(
		self,
		definition: ItemDefinitionIndex,
	) -> Result<Wearable<'s>, WearableError> {
		// SAFETY: The caller vouches for the native creation path. The generator
		// initializes the item view before `Spawn` and `Activate` and returns a
		// fresh callback-live entity, which must not be spawned again.
		let wearable = unsafe {
			self.give_with(|origin| {
				weapons::generate_item(self.server, definition, origin, None)
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
	#[doc(alias("m_hMyWearables", "GetWearable"))]
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
	#[doc(alias("RemoveWearable"))]
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
#[doc(alias("CTFWearable"))]
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
	#[doc(alias("m_hWeaponAssociatedWith", "GetWeaponAssociatedWith"))]
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
	#[doc(alias("m_iItemDefinitionIndex", "GetItemDefIndex"))]
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
	#[doc(alias("m_bDisguiseWearable", "IsDisguiseWearable"))]
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
	#[doc(alias("m_bValidatedAttachedEntity"))]
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
	#[doc(alias("m_hOwnerEntity", "GetOwnerEntity"))]
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
	#[doc(alias("m_bValidatedAttachedEntity", "MarkAttachedEntityAsValidated"))]
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
	/// example because the definition does not exist
	/// ([`ItemGenerationError::UnknownDefinition`]).
	#[error(transparent)]
	CreationFailedNative(#[from] ItemGenerationError),

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
	#[doc(alias("CTFWearableCampaignItem"))]
	Campaign,

	/// `tf_wearable_demoshield`, a Demoman's shield, such as the Chargin'
	/// Targe.
	#[doc(alias("CTFWearableDemoShield"))]
	DemoShield,

	/// `tf_wearable_levelable_item`, an item that levels up.
	#[doc(alias("CTFWearableLevelableItem"))]
	Levelable,

	/// Any other class name.
	Other,

	/// `tf_wearable`: cosmetics, and gameplay items worn in weapon slots, such
	/// as the Gunboats or the Mantreads.
	#[doc(alias("CTFWearable"))]
	Plain,

	/// `tf_powerup_bottle`, Mann vs. Machine's Power Up Canteen.
	#[doc(alias("CTFPowerupBottle"))]
	PowerupBottle,

	/// `tf_wearable_razorback`, the Sniper's Razorback.
	#[doc(alias("CTFWearableRazorback"))]
	Razorback,

	/// `tf_wearable_robot_arm`, the Engineer's Gunslinger arm.
	#[doc(alias("CTFWearableRobotArm"))]
	RobotArm,

	/// `tf_wearable_vm`, drawn on the player's view model, such as a weapon's
	/// extra view model wearable.
	#[doc(alias("CTFWearableVM"))]
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
