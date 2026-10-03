//! Gives plugin-chosen wearables and weapon attributes again each time TF2
//! rebuilds a player's loadout.
//!
//! TF2 rebuilds a player's loadout when they spawn, through
//! `CTFPlayer::InitClass`, and when they touch a resupply locker, through
//! `CTFPlayer::Regenerate`, which calls `InitClass` too. `InitClass` hands out
//! the player's items (`GiveDefaultItems` and `ManageRegularWeapons`): it
//! keeps the weapons of the player's own loadout, replaces any weapon it did
//! not hand out itself, which loses the attributes a plugin set on it, and
//! removes every wearable their loadout lacks (`CTFPlayer::ValidateWearables`),
//! which includes those a plugin gave with [`PlayerWearables::give`]. It then
//! fires [`GameEventId::PostInventoryApplication`] for the player, bots
//! included.
//!
//! A [`LoadoutReapplier`] remembers a [`Loadout`] per player, by [`UserId`],
//! and gives it again from that event: each wanted wearable the player does
//! not already wear, and each weapon slot's [`AttributeSet`] on the weapon
//! the player has in that slot. Like the loadouts, it holds plain data, which
//! a plugin keeps between callbacks.
//!
//! This is a convenience behind the `tf2_loadout` feature. A plugin that
//! wants other rules, such as items chosen per class or weapons given again,
//! can do the same from its own listener with the APIs this module uses:
//! [`PlayerWearables::list`] and [`PlayerWearables::give`] for wearables, and
//! [`PlayerWeapons::get_slot`], [`Weapon::attributes`] and
//! [`AttributeSet::apply`] for attributes.
//!
//! # Wiring
//!
//! - Pass every [`GameEventId::PostInventoryApplication`] event, from a
//!   [`GameEventListener`] registered for it, to
//!   [`LoadoutReapplier::on_game_event`]. It ignores other events, and
//!   players without a loadout.
//! - Call [`LoadoutReapplier::forget`] when a client disconnects, from a
//!   [`GameEventId::PlayerDisconnect`] listener.
//! - Call [`LoadoutReapplier::retain_connected`] at least once each level,
//!   such as from a [`GameEventId::PlayerActivate`] listener. Human clients
//!   keep their user IDs across a level change, and their players spawn again
//!   on the new level, which fires the event again, so their loadouts carry
//!   over; [`LoadoutReapplier::clear`] at level shutdown would forget them.
//!   Bots, however, are expected to be dropped at a level change without
//!   `player_disconnect` firing, and to join the next level with new user
//!   IDs. `retain_connected` forgets the loadouts of user IDs no connected
//!   client has, so that the engine reusing such a user ID for a later client
//!   does not give that client a stale loadout.
//! - To give a loadout at once, without waiting for the player's next spawn
//!   or resupply, call [`LoadoutReapplier::apply`] after
//!   [`LoadoutReapplier::set`].
//! - Once a call that applied returns, apply again for each user ID its
//!   report lists in [`ApplyReport::deferred`].
//!
//! # What it does
//!
//! - It only adds. It never removes or replaces the player's own items, and
//!   gives no wearable whose definition the player already wears, from their
//!   own loadout or given before. A disguised Spy's disguise wearables, which
//!   carry the definitions of the disguise target's items and outlast a
//!   resupply, do not count as worn.
//! - It never gives weapons. A weapon a plugin gave, such as with
//!   [`PlayerWeapons::give_item_with`], is replaced at the next resupply, and
//!   a loadout's attributes go on whichever weapon the player then has in the
//!   slot, which may be one from their own loadout.
//! - It never removes attributes either. Attributes it set stay on a weapon
//!   until the game replaces the weapon, and the game keeps the weapons of
//!   the player's own loadout across spawns and resupplies, so on those they
//!   outlast [`Loadout::remove_weapon_attributes`], [`LoadoutReapplier::set`],
//!   [`LoadoutReapplier::forget`] and [`LoadoutReapplier::clear`]. A plugin
//!   that wants them gone removes them with [`ItemAttributes::remove`],
//!   through [`Weapon::attributes`].
//! - Each item is applied on its own, and the [`ApplyReport`] tells what
//!   happened to each, rather than stopping at the first failure.
//! - A wanted wearable is a new entity each time it is given: the game
//!   removes the previous one whenever it rebuilds the player's loadout. Each
//!   new one takes an edict and is sent in full to the clients that see the
//!   player, which may briefly draw the player without it.
//! - Player attributes ([`PlayerAttributes`]), which the game clears whenever
//!   the player spawns, are not covered.
//!
//! The [limits of wearables](crate::tf2::wearables#limits) apply to each one
//! given, such as the [`MAX_NETWORKED_WEARABLES`] a player can wear, which
//! the player's own items count towards.
//!
//! # Re-entrancy
//!
//! Giving a wearable runs the game's and other plugins' code. Should that
//! regenerate a player, the game fires the event again while the reapplier is
//! still applying. [`LoadoutReapplier::apply`] and
//! [`LoadoutReapplier::on_game_event`] therefore take `&self`, so that the
//! nested listener can reach the reapplier, and refuse to apply with
//! [`LoadoutError::Reentrant`] until the outer call returns
//! ([`LoadoutReapplier::is_applying`]), whichever player the nested call is
//! for. The outer call's report lists the user IDs it refused in
//! [`ApplyReport::deferred`], to apply again once it returns. If the report's
//! own player is among them, a nested regeneration may have removed
//! wearables the report lists as given.
//!
//! A plugin that keeps the reapplier in a `RefCell` must borrow it shared to
//! apply for this to work: a nested mutable borrow panics, which aborts the
//! server at the engine boundary. Changing loadouts takes `&mut self`, so it
//! cannot happen while applying, and code that applying may reach, such as a
//! [`GameEventId::PlayerDisconnect`] listener calling
//! [`LoadoutReapplier::forget`] for a client another plugin kicks from an
//! equip hook, must borrow with `try_borrow_mut`, and put the change off
//! while the reapplier is borrowed, rather than with `borrow_mut`.
//!
//! # Unverified
//!
//! On TF2's 64-bit Windows server, a reapplier passed each
//! [`GameEventId::PostInventoryApplication`] has been observed, with one bot
//! and a loadout of a wearable and one slot's attribute, to:
//!
//! - give both through [`LoadoutReapplier::apply`], and report the wearable
//!   as [`WearableOutcome::AlreadyWorn`], without giving it twice, when
//!   applying again;
//! - give a new wearable, and set the attribute on a new weapon, after the
//!   bot respawned as another class, whose loadout replaced both;
//! - give a new wearable after the bot respawned as the same class, and after
//!   VScript's `CTFPlayer::Regenerate`, which a resupply locker calls, ran
//!   for it, while the game kept the weapon and its attribute;
//! - give nothing after [`LoadoutReapplier::forget`], when the game removed
//!   the wearable at the next regeneration and the kept weapon kept the
//!   attribute.
//!
//! Human players, what clients draw in between, touching a resupply locker,
//! [`ApplyReport::deferred`] and [`LoadoutError::Reentrant`], whether bots
//! dropped at a level change fire `player_disconnect` and get new user IDs,
//! whether the engine counts human clients as connected while they load the
//! next level, which [`LoadoutReapplier::retain_connected`] relies on to keep
//! their loadouts, Mann vs. Machine, and Linux servers have not been tested.
//!
//! [`GameEventId::PlayerActivate`]: crate::tf2::game_events::GameEventId::PlayerActivate
//! [`GameEventId::PlayerDisconnect`]: crate::tf2::game_events::GameEventId::PlayerDisconnect
//! [`GameEventId::PostInventoryApplication`]: crate::tf2::game_events::GameEventId::PostInventoryApplication
//! [`GameEventListener`]: crate::interfaces::game_event::GameEventListener
//! [`ItemAttributes::remove`]: crate::tf2::attributes::ItemAttributes::remove
//! [`PlayerAttributes`]: crate::tf2::attributes::PlayerAttributes
//! [`PlayerWeapons::give_item_with`]: crate::tf2::weapons::PlayerWeapons::give_item_with
//! [`Weapon::attributes`]: crate::tf2::weapons::Weapon::attributes

#[cfg(test)]
#[path = "../tests/tf2/loadout.rs"]
mod tests;

use crate::entities::{Entity, EntityHandle};
use crate::interfaces::GameClient;
use crate::interfaces::game_event::GameEvent;
use crate::players::UserId;
use crate::tf2::attributes::{AttributeSet, SchemaToken};
use crate::tf2::game_events::GameEventId;
use crate::tf2::weapons::{IntoWeaponSlot, ItemDefinitionIndex, PlayerWeapons, WeaponError};
use crate::tf2::wearables::{MAX_NETWORKED_WEARABLES, PlayerWearables, Wearable, WearableError};
use crate::{Game, InterfaceError, Server};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{CStr, c_int};

/// The key of [`GameEventId::PostInventoryApplication`] naming the player.
///
/// [`GameEventId::PostInventoryApplication`]: crate::tf2::game_events::GameEventId::PostInventoryApplication
const USER_ID_KEY: &CStr = c"userid";

/// Marks a [`LoadoutReapplier`] as applying until it is dropped, then forgets
/// the user IDs it deferred that no report took.
#[derive(Debug)]
struct ApplyGuard<'r>(&'r LoadoutReapplier);

impl Drop for ApplyGuard<'_> {
	fn drop(&mut self) {
		self.0.applying.set(false);
		self.0.deferred.take();
	}
}

/// What [`LoadoutReapplier::apply`] did for one player, item by item.
///
/// A failed item did not stop the others.
#[derive(Debug)]
#[non_exhaustive]
pub struct ApplyReport {
	/// The player's user ID.
	pub user_id: UserId,

	/// Each wanted wearable, in the order of [`Loadout::wearables`], and
	/// whether it was given.
	pub wearables: Vec<(ItemDefinitionIndex, Result<WearableOutcome, WearableError>)>,

	/// Each native weapon slot of [`Loadout::weapons`], in slot order, and
	/// whether its attributes were set.
	pub weapons: Vec<(c_int, Result<WeaponOutcome, WeaponError>)>,

	/// The user IDs the reapplier refused to apply for while this call
	/// applied, each once, in the order they were refused, as for a nested
	/// event listener that got [`LoadoutError::Reentrant`]. Apply again for
	/// each once this call returns.
	///
	/// If [`Self::user_id`] is among them, a nested regeneration of the player
	/// may have removed wearables [`Self::wearables`] reports as given.
	pub deferred: Vec<UserId>,
}

impl ApplyReport {
	/// Whether no item failed. [`Self::deferred`] is not considered.
	pub fn is_ok(&self) -> bool {
		self.wearables.iter().all(|(_, outcome)| outcome.is_ok())
			&& self.weapons.iter().all(|(_, outcome)| outcome.is_ok())
	}
}

/// The wearables and weapon attributes a plugin wants a player to have, which
/// a [`LoadoutReapplier`] gives the player each time the game rebuilds their
/// loadout.
///
/// It holds plain data: wearables by [`ItemDefinitionIndex`], each at most
/// once and at most [`MAX_NETWORKED_WEARABLES`] of them, in the order they
/// are given, and a non-empty [`AttributeSet`] per native weapon slot.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Loadout {
	weapons: BTreeMap<c_int, AttributeSet>,
	wearables: Vec<ItemDefinitionIndex>,
}

impl Loadout {
	/// A loadout that wants nothing, which a `const` or `static` can hold.
	pub const fn new() -> Self {
		Self {
			weapons: BTreeMap::new(),
			wearables: Vec::new(),
		}
	}

	/// Sets the attributes to apply to the weapon in a native slot, such as
	/// [`WeaponSlot::Melee`] or a raw slot number, replacing the slot's
	/// previous set, which is returned. An empty set removes the slot's set,
	/// as [`Self::remove_weapon_attributes`] does.
	///
	/// [`WeaponSlot::Melee`]: crate::tf2::weapons::WeaponSlot::Melee
	pub fn insert_weapon_attributes(
		&mut self,
		slot: impl IntoWeaponSlot,
		attributes: AttributeSet,
	) -> Option<AttributeSet> {
		let slot = slot.into_weapon_slot();

		if attributes.is_empty() {
			return self.weapons.remove(&slot);
		}

		self.weapons.insert(slot, attributes)
	}

	/// Adds a wearable to give, after those already added. Returns `Ok(false)`
	/// without changing anything if the loadout already has it.
	///
	/// Fails with [`LoadoutError::TooManyWearables`] once the loadout has
	/// [`MAX_NETWORKED_WEARABLES`], the most the game networks for a player.
	/// The player's own items count towards that limit too, such as their
	/// cosmetics and their weapons' extra wearables, so giving may still fail
	/// with [`WearableError::Full`] for fewer.
	pub fn insert_wearable(
		&mut self,
		definition: ItemDefinitionIndex,
	) -> Result<bool, LoadoutError> {
		if self.wearables.contains(&definition) {
			return Ok(false);
		}

		if self.wearables.len() >= MAX_NETWORKED_WEARABLES {
			return Err(LoadoutError::TooManyWearables);
		}

		self.wearables.push(definition);

		Ok(true)
	}

	/// Whether the loadout wants no wearable, and attributes for no slot.
	pub fn is_empty(&self) -> bool {
		self.weapons.is_empty() && self.wearables.is_empty()
	}

	/// Stops applying attributes to the weapon in a native slot, returning
	/// the slot's set, if it had one.
	///
	/// Attributes already set stay on the player's weapon until the game
	/// replaces it, which it does not for a weapon of the player's own
	/// loadout, as the
	/// [module documentation](crate::tf2::loadout#what-it-does) describes.
	pub fn remove_weapon_attributes(&mut self, slot: impl IntoWeaponSlot) -> Option<AttributeSet> {
		self.weapons.remove(&slot.into_weapon_slot())
	}

	/// Stops giving a wearable, returning whether the loadout had it. Wearables
	/// already given stay until the game next removes them.
	pub fn remove_wearable(&mut self, definition: ItemDefinitionIndex) -> bool {
		let len = self.wearables.len();

		self.wearables.retain(|&wanted| wanted != definition);
		self.wearables.len() != len
	}

	/// The attributes to apply to the weapon in a native slot, if any.
	pub fn weapon_attributes(&self, slot: impl IntoWeaponSlot) -> Option<&AttributeSet> {
		self.weapons.get(&slot.into_weapon_slot())
	}

	/// Each native weapon slot with attributes to apply, in slot order.
	pub fn weapons(&self) -> impl Iterator<Item = (c_int, &AttributeSet)> {
		self.weapons
			.iter()
			.map(|(&slot, attributes)| (slot, attributes))
	}

	/// The wearables to give, in the order they are given.
	pub fn wearables(&self) -> &[ItemDefinitionIndex] {
		&self.wearables
	}

	/// As [`Self::insert_weapon_attributes`], by value, for chaining.
	pub fn with_weapon_attributes(
		mut self,
		slot: impl IntoWeaponSlot,
		attributes: AttributeSet,
	) -> Self {
		self.insert_weapon_attributes(slot, attributes);
		self
	}

	/// As [`Self::insert_wearable`], by value, for chaining.
	pub fn with_wearable(mut self, definition: ItemDefinitionIndex) -> Result<Self, LoadoutError> {
		self.insert_wearable(definition)?;

		Ok(self)
	}
}

/// Why a loadout could not be changed or applied.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LoadoutError {
	/// A required engine interface is unavailable.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// A [`GameEventId::PostInventoryApplication`] event has no `userid`, or
	/// one that is not a user ID.
	///
	/// [`GameEventId::PostInventoryApplication`]: crate::tf2::game_events::GameEventId::PostInventoryApplication
	#[error("the event has no valid `userid`")]
	InvalidUserId,

	/// No connected client has the user ID, or its player has no entity yet.
	#[error("no connected client with the user id has a player")]
	NoPlayer,

	/// The engine returned no game server.
	#[error("the engine has no game server")]
	NoServer,

	/// The player is not a TF2 player, or the server does not run TF2.
	#[error("loadouts require a TF2 player")]
	NotTfPlayer,

	/// The reapplier was called while it applies a loadout, by code applying
	/// it ran, such as a nested event listener. The outer call lists the user
	/// ID in its [`ApplyReport::deferred`].
	#[error("the reapplier is already applying a loadout")]
	Reentrant,

	/// The loadout already has [`MAX_NETWORKED_WEARABLES`] wearables.
	#[error("the loadout already has as many wearables as the game networks")]
	TooManyWearables,
}

/// Each player's [`Loadout`], by user ID, given again whenever the game
/// rebuilds the player's loadout, as the
/// [module documentation](crate::tf2::loadout) describes.
///
/// It holds plain data, and is kept between callbacks. Applying takes
/// `&self` and marks the reapplier as applying, for the reasons the
/// [module documentation](crate::tf2::loadout#re-entrancy) gives, so a
/// reapplier is not `Sync`: keep it where only the server's main thread
/// reaches it.
#[doc(alias("Regenerate", "ValidateWearables"))]
#[derive(Debug, Default)]
pub struct LoadoutReapplier {
	applying: Cell<bool>,
	deferred: RefCell<Vec<UserId>>,
	loadouts: BTreeMap<UserId, Loadout>,
}

impl LoadoutReapplier {
	/// A reapplier that knows no player.
	pub const fn new() -> Self {
		Self {
			applying: Cell::new(false),
			deferred: RefCell::new(Vec::new()),
			loadouts: BTreeMap::new(),
		}
	}

	/// Gives the player with `user_id` their loadout now: each wanted wearable
	/// they do not already wear, through [`PlayerWearables::give`], then each
	/// slot's attributes to the weapon [`PlayerWeapons::get_slot`] finds, through
	/// [`AttributeSet::apply`] and [`ItemAttributes::reapply_provision`], as
	/// [`PlayerWeapons::give_item_with`] does.
	///
	/// A wearable is already worn if the player's list holds one of its
	/// definition that is not marked for deletion and is not one of a
	/// disguised Spy's disguise wearables. Nothing the player has is removed
	/// or replaced. The report tells what happened to each item, and a failed
	/// item does not stop the others, though a slot whose attributes failed
	/// keeps those set before the failure, as [`AttributeSet::apply`] leaves
	/// them. A player without a loadout, or with an empty one, gets an empty
	/// report, without the engine being asked anything.
	///
	/// Fails, without applying anything, with [`LoadoutError::Reentrant`]
	/// while this reapplier is applying, which the outer call reports in
	/// [`ApplyReport::deferred`], [`LoadoutError::NotTfPlayer`] outside TF2 or
	/// for a player that is not a TF2 player, [`LoadoutError::Interface`]
	/// without the engine's interface, and [`LoadoutError::NoPlayer`] if no
	/// connected client has the user ID, or its player has no entity yet.
	///
	/// # Safety
	///
	/// As for [`PlayerWearables::give`], for each wearable the loadout wants:
	/// the definition's constructor, spawn and activation, and the callbacks
	/// equipping runs, must uphold [`Server::new`]'s no-immediate-deletion
	/// contract, and it must not be called while the game iterates the
	/// player's wearables, such as from a hook on `EquipWearable`,
	/// `RemoveWearable` or an item's `UnEquip`. The attribute writes rest on
	/// `token`, for which the caller of
	/// [`trust_shipped_schema`](crate::tf2::attributes::trust_shipped_schema)
	/// vouched.
	///
	/// [`ItemAttributes::reapply_provision`]: crate::tf2::attributes::ItemAttributes::reapply_provision
	/// [`PlayerWeapons::give_item_with`]: crate::tf2::weapons::PlayerWeapons::give_item_with
	pub unsafe fn apply<'s>(
		&self,
		server: Server<'s>,
		user_id: UserId,
		token: SchemaToken<'s>,
	) -> Result<ApplyReport, LoadoutError> {
		let _applying = self.begin(user_id)?;

		let mut report = ApplyReport {
			user_id,
			wearables: Vec::new(),
			weapons: Vec::new(),
			deferred: Vec::new(),
		};

		let Some(loadout) = self
			.loadouts
			.get(&user_id)
			.filter(|loadout| !loadout.is_empty())
		else {
			return Ok(report);
		};

		let player = find_player(server, user_id)?;

		// Both wrappers only refuse a player that is not a TF2 player, or a
		// server not running TF2.
		let wearables =
			PlayerWearables::new(server, player).map_err(|_| LoadoutError::NotTfPlayer)?;
		let weapons = PlayerWeapons::new(server, player).map_err(|_| LoadoutError::NotTfPlayer)?;

		// Wearables come first: should giving one regenerate the player, the
		// weapons found afterwards are the ones the player then has.
		for &definition in &loadout.wearables {
			// SAFETY: The caller upholds `give`'s contract for every wearable
			// the loadout wants.
			let outcome = unsafe { give_missing(wearables, definition) };

			report.wearables.push((definition, outcome));
		}

		for (&slot, attributes) in &loadout.weapons {
			report
				.weapons
				.push((slot, apply_attributes(token, weapons, slot, attributes)));
		}

		// Calls are only refused while this one applies, so every user ID
		// recorded was refused for it.
		report.deferred = self.deferred.take();

		Ok(report)
	}

	/// Marks the reapplier as applying until the guard is dropped. If it
	/// already is, records `user_id` for the outer call's report, once, and
	/// fails with [`LoadoutError::Reentrant`].
	fn begin(&self, user_id: UserId) -> Result<ApplyGuard<'_>, LoadoutError> {
		if self.applying.replace(true) {
			// No borrow of the list outlives the statement that takes it, and
			// nothing here runs other code.
			let mut deferred = self.deferred.borrow_mut();

			if !deferred.contains(&user_id) {
				deferred.push(user_id);
			}

			return Err(LoadoutError::Reentrant);
		}

		Ok(ApplyGuard(self))
	}

	/// Forgets every player's loadout. What was already given stays, as the
	/// [module documentation](crate::tf2::loadout#what-it-does) describes.
	///
	/// A level change needs no `clear`, which would also forget the loadouts
	/// of human clients, who keep their user IDs across it. Call
	/// [`Self::retain_connected`] each level instead, for the bots the level
	/// change drops.
	pub fn clear(&mut self) {
		self.loadouts.clear();
	}

	/// Forgets the loadout of the player with `user_id`, and returns it. Call
	/// it once the client disconnects, from a
	/// [`GameEventId::PlayerDisconnect`] listener. What was already given
	/// stays, as for [`Self::clear`].
	///
	/// [`GameEventId::PlayerDisconnect`]: crate::tf2::game_events::GameEventId::PlayerDisconnect
	pub fn forget(&mut self, user_id: UserId) -> Option<Loadout> {
		self.loadouts.remove(&user_id)
	}

	/// The loadout of the player with `user_id`, if any.
	pub fn get(&self, user_id: UserId) -> Option<&Loadout> {
		self.loadouts.get(&user_id)
	}

	/// The loadout of the player with `user_id`, to change it in place, if
	/// any.
	pub fn get_mut(&mut self, user_id: UserId) -> Option<&mut Loadout> {
		self.loadouts.get_mut(&user_id)
	}

	/// Whether [`Self::apply`] or [`Self::on_game_event`] is applying a
	/// loadout, as code they run sees it, such as a nested event listener.
	pub fn is_applying(&self) -> bool {
		self.applying.get()
	}

	/// Each player's loadout, by user ID, in user ID order.
	pub fn iter(&self) -> impl Iterator<Item = (UserId, &Loadout)> {
		self.loadouts
			.iter()
			.map(|(&user_id, loadout)| (user_id, loadout))
	}

	/// Applies, as [`Self::apply`] does, the loadout of the player a
	/// [`GameEventId::PostInventoryApplication`] event names, for a listener
	/// of that event to call.
	///
	/// Returns `Ok(None)`, without asking the engine anything, for any other
	/// event, and for a player without a loadout or with an empty one, such as
	/// most bots. Fails with [`LoadoutError::InvalidUserId`] if the event's
	/// `userid` is missing or not a user ID, and otherwise as [`Self::apply`]
	/// does.
	///
	/// # Safety
	///
	/// As for [`Self::apply`]. The game itself fires the event outside its
	/// loops over the player's wearables; the caller must still rule out the
	/// event being fired by other code, such as another plugin, from inside
	/// such a loop.
	///
	/// [`GameEventId::PostInventoryApplication`]: crate::tf2::game_events::GameEventId::PostInventoryApplication
	#[doc(alias("post_inventory_application"))]
	pub unsafe fn on_game_event<'s>(
		&self,
		server: Server<'s>,
		event: GameEvent<'_>,
		token: SchemaToken<'s>,
	) -> Result<Option<ApplyReport>, LoadoutError> {
		if event.name() != GameEventId::PostInventoryApplication.name_cstr() {
			return Ok(None);
		}

		let user_id = event
			.get_int(USER_ID_KEY)
			.and_then(|raw| UserId::from_raw(raw).ok())
			.ok_or(LoadoutError::InvalidUserId)?;

		if self.loadouts.get(&user_id).is_none_or(Loadout::is_empty) {
			return Ok(None);
		}

		// SAFETY: The caller upholds `apply`'s contract.
		unsafe { self.apply(server, user_id, token) }.map(Some)
	}

	/// Keeps only the loadouts for which `keep` returns `true`, which may
	/// change them, visiting players in user ID order.
	pub fn retain(&mut self, mut keep: impl FnMut(UserId, &mut Loadout) -> bool) {
		self.loadouts
			.retain(|&user_id, loadout| keep(user_id, loadout));
	}

	/// Forgets the loadouts of user IDs no connected client has, such as those
	/// of bots dropped at a level change, for which `player_disconnect` is not
	/// expected to fire.
	///
	/// Every client the engine's server counts as connected is kept
	/// (`IClient::IsConnected`), which is expected, though not observed, to
	/// include human clients still loading the level, so call it at least
	/// once each level, such as from a
	/// [`GameEventId::PlayerActivate`] listener, as the
	/// [module documentation](crate::tf2::loadout#wiring) describes.
	///
	/// Fails, forgetting nothing, with [`LoadoutError::Interface`] without the
	/// engine's interface, and [`LoadoutError::NoServer`] if the engine has no
	/// game server.
	///
	/// [`GameEventId::PlayerActivate`]: crate::tf2::game_events::GameEventId::PlayerActivate
	pub fn retain_connected(&mut self, server: Server<'_>) -> Result<(), LoadoutError> {
		let connected: BTreeSet<UserId> = server
			.valve_engine()?
			.game_server()
			.ok_or(LoadoutError::NoServer)?
			.clients()
			.filter(|client| client.is_connected())
			.filter_map(GameClient::user_id)
			.collect();

		self.loadouts
			.retain(|user_id, _| connected.contains(user_id));

		Ok(())
	}

	/// Sets the loadout of the player with `user_id`, returning the previous
	/// one. It is given from the player's next spawn or resupply; call
	/// [`Self::apply`] to give it at once.
	///
	/// What the previous loadout gave stays: its wearables until the game
	/// next removes them, and its attributes on a weapon until the game
	/// replaces it, as the
	/// [module documentation](crate::tf2::loadout#what-it-does) describes.
	pub fn set(&mut self, user_id: UserId, loadout: Loadout) -> Option<Loadout> {
		self.loadouts.insert(user_id, loadout)
	}
}

/// What applying a slot's attributes did, when it did not fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WeaponOutcome {
	/// The attributes were set on the weapon with this handle.
	Applied(EntityHandle),

	/// The player has no weapon in the slot, so nothing was set.
	NoWeapon,
}

/// What giving a wanted wearable did, when it did not fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WearableOutcome {
	/// The player already wears a wearable of the definition, with this
	/// handle, so none was given.
	AlreadyWorn(EntityHandle),

	/// A wearable was given, with this handle.
	Given(EntityHandle),
}

/// Sets a slot's attributes, which a [`Loadout`] never leaves empty, on the
/// weapon the player has in it, then reapplies its provision, which the game
/// decided when equipping it.
fn apply_attributes<'s>(
	token: SchemaToken<'s>,
	weapons: PlayerWeapons<'s>,
	slot: c_int,
	attributes: &AttributeSet,
) -> Result<WeaponOutcome, WeaponError> {
	let Some(weapon) = weapons.get_slot(slot)? else {
		return Ok(WeaponOutcome::NoWeapon);
	};

	let item = weapon.attributes()?;

	attributes.apply(token, item)?;
	item.reapply_provision(token)?;

	Ok(WeaponOutcome::Applied(weapon.entity().handle()))
}

/// The player entity of the connected client with `user_id`.
fn find_player(server: Server<'_>, user_id: UserId) -> Result<Entity<'_>, LoadoutError> {
	if server.game() != Game::TeamFortress2 {
		return Err(LoadoutError::NotTfPlayer);
	}

	server
		.valve_engine()?
		.edict_of_user_id(user_id)
		.and_then(|edict| edict.entity())
		.ok_or(LoadoutError::NoPlayer)
}

/// A wearable of `definition` the player wears. One marked for deletion stays
/// listed until the game frees it, and a disguised Spy's disguise wearables,
/// which carry the definitions of the disguise target's items, are the
/// disguise's, so neither counts.
fn find_worn<'s>(
	wearables: PlayerWearables<'s>,
	definition: ItemDefinitionIndex,
) -> Result<Option<Wearable<'s>>, WearableError> {
	let list = wearables.list()?;

	for wearable in list.iter() {
		if wearable.entity().is_marked_for_deletion() || wearable.is_disguise()? {
			continue;
		}

		if wearable.definition()? == Some(definition) {
			return Ok(Some(wearable));
		}
	}

	Ok(None)
}

/// Gives a wearable of `definition` unless the player already wears one. The
/// list is read again for each wearable, since giving changes it.
///
/// # Safety
///
/// As for [`PlayerWearables::give`].
unsafe fn give_missing(
	wearables: PlayerWearables<'_>,
	definition: ItemDefinitionIndex,
) -> Result<WearableOutcome, WearableError> {
	if let Some(worn) = find_worn(wearables, definition)? {
		return Ok(WearableOutcome::AlreadyWorn(worn.entity().handle()));
	}

	// SAFETY: The caller upholds `give`'s contract.
	let given = unsafe { wearables.give(definition) }?;

	Ok(WearableOutcome::Given(given.entity().handle()))
}
