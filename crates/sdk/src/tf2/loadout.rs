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

#[cfg(test)]
mod tests {
	use super::*;
	use crate::interfaces::ValveEngine;
	use crate::net::cheats::test_support::{MockClient, MockEngine};
	use crate::server::test_support::{export, mock_server};
	use crate::tf2::attributes::{Multiplier, catalog, trust_shipped_schema};
	use crate::tf2::weapons::WeaponSlot;
	use crate::{InterfaceFactory, Module};
	use sdk_raw::util::mock::{mock_vtable, unexpected_call};
	use std::ffi::{c_char, c_void};
	use std::ptr::{NonNull, null_mut};

	/// A game event with a name and, optionally, a `userid`.
	#[repr(C)]
	struct MockEvent {
		raw: sys::IGameEvent,
		name: &'static CStr,
		user_id: Option<c_int>,
	}

	impl MockEvent {
		fn new(name: &'static CStr, user_id: Option<c_int>) -> Self {
			let vtable = Box::leak(unsafe {
				mock_vtable::<sys::IGameEvent__bindgen_vtable>(
					unexpected_call as *const (),
					|vtable| {
						(&raw mut (*vtable).IGameEvent_GetName).write(event_name);
						(&raw mut (*vtable).IGameEvent_IsEmpty).write(event_is_empty);
						(&raw mut (*vtable).IGameEvent_GetInt).write(event_get_int);
					},
				)
			});

			Self {
				raw: sys::IGameEvent { vtable_: vtable },
				name,
				user_id,
			}
		}

		fn event(&mut self) -> GameEvent<'_> {
			// SAFETY: The mock outlives the borrow, and is used on this thread.
			// The pointer covers the whole mock, which the callbacks read.
			unsafe { GameEvent::from_raw(NonNull::from(self).cast()) }
		}
	}

	#[test]
	fn applying_needs_a_connected_tf2_player() {
		let scope = ();
		let mut reapplier = LoadoutReapplier::new();
		let loadout = Loadout::new().with_weapon_attributes(WeaponSlot::Melee, damage_bonus());

		// No loadout: an empty report, without asking the engine, which the
		// mock server does not export yet.
		let server = mock_server(&scope);
		let token = unsafe { trust_shipped_schema(server) };
		let report = unsafe { reapplier.apply(server, user(2), token) }.unwrap();

		assert_eq!(report.user_id, user(2));
		assert!(report.wearables.is_empty() && report.weapons.is_empty() && report.is_ok());
		assert!(report.deferred.is_empty());

		// An empty loadout too.
		reapplier.set(user(2), Loadout::new());

		let report = unsafe { reapplier.apply(server, user(2), token) }.unwrap();

		assert!(report.wearables.is_empty() && report.weapons.is_empty());

		// Another game.
		reapplier.set(user(2), loadout);

		let factory = InterfaceFactory::new(no_interfaces);
		let other = unsafe { Server::new(factory, factory, Game::SourceSdk2013, &scope) };
		let other_token = unsafe { trust_shipped_schema(other) };

		assert!(matches!(
			unsafe { reapplier.apply(other, user(2), other_token) },
			Err(LoadoutError::NotTfPlayer)
		));

		// No client has the user ID.
		let vtable = unsafe {
			mock_vtable::<sys::IVEngineServer__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IVEngineServer_PEntityOfEntIndex).write(no_edict);
				},
			)
		};
		let mut engine = sys::IVEngineServer {
			vtable_: &raw const *vtable,
		};

		export(Module::Engine, ValveEngine::VERSION, &raw mut engine);
		assert!(matches!(
			unsafe { reapplier.apply(server, user(2), token) },
			Err(LoadoutError::NoPlayer)
		));
		assert!(!reapplier.is_applying());
	}

	#[test]
	fn applying_refuses_nested_calls_and_records_them() {
		let scope = ();
		let server = mock_server(&scope);
		let token = unsafe { trust_shipped_schema(server) };
		let mut reapplier = LoadoutReapplier::new();
		let mut inventory = MockEvent::new(c"post_inventory_application", Some(2));
		let mut spawn = MockEvent::new(c"player_spawn", Some(2));
		let mut unknown = MockEvent::new(c"post_inventory_application", Some(4));

		reapplier.set(user(2), Loadout::new().with_wearable(def(106)).unwrap());
		assert!(!reapplier.is_applying());

		// The mock server exports no engine interface, so applying fails, and
		// the reapplier is no longer applying afterwards.
		assert!(matches!(
			unsafe { reapplier.apply(server, user(2), token) },
			Err(LoadoutError::Interface(_))
		));
		assert!(!reapplier.is_applying());

		// As seen from a nested listener, while an outer call applies.
		let outer = reapplier.begin(user(2)).unwrap();

		assert!(reapplier.is_applying());
		assert_eq!(
			reapplier.begin(user(5)).unwrap_err(),
			LoadoutError::Reentrant
		);
		assert!(matches!(
			unsafe { reapplier.apply(server, user(2), token) },
			Err(LoadoutError::Reentrant)
		));
		assert!(matches!(
			unsafe { reapplier.apply(server, user(3), token) },
			Err(LoadoutError::Reentrant)
		));
		assert!(matches!(
			unsafe { reapplier.on_game_event(server, inventory.event(), token) },
			Err(LoadoutError::Reentrant)
		));

		// Events the reapplier ignores are not refused, nor recorded.
		assert!(matches!(
			unsafe { reapplier.on_game_event(server, spawn.event(), token) },
			Ok(None)
		));
		assert!(matches!(
			unsafe { reapplier.on_game_event(server, unknown.event(), token) },
			Ok(None)
		));
		assert!(reapplier.is_applying(), "refusals keep the outer mark");

		// Each refused user ID is recorded once, in order, for the outer call.
		assert_eq!(*reapplier.deferred.borrow(), [5, 2, 3].map(user));

		// The guard forgets what no report took.
		drop(outer);
		assert!(!reapplier.is_applying());
		assert!(reapplier.deferred.borrow().is_empty());
	}

	fn damage_bonus() -> AttributeSet {
		AttributeSet::new()
			.with(&catalog::DAMAGE_BONUS, Multiplier::new(2.0).unwrap())
			.unwrap()
	}

	fn def(index: u16) -> ItemDefinitionIndex {
		ItemDefinitionIndex::new(index).unwrap()
	}

	unsafe extern "C" fn event_get_int(
		event: *const sys::IGameEvent,
		key: *const c_char,
		default: c_int,
	) -> c_int {
		assert_eq!(unsafe { CStr::from_ptr(key) }, USER_ID_KEY);

		// SAFETY: Every mock event's vtable belongs to a `MockEvent`.
		unsafe { (&raw const (*event.cast::<MockEvent>()).user_id).read() }.unwrap_or(default)
	}

	unsafe extern "C" fn event_is_empty(event: *mut sys::IGameEvent, key: *const c_char) -> bool {
		// SAFETY: As for `event_get_int`.
		let user_id = unsafe { (&raw const (*event.cast::<MockEvent>()).user_id).read() };
		let key = unsafe { CStr::from_ptr(key) };

		key != USER_ID_KEY || user_id.is_none()
	}

	unsafe extern "C" fn event_name(event: *const sys::IGameEvent) -> *const c_char {
		// SAFETY: As for `event_get_int`.
		unsafe { (&raw const (*event.cast::<MockEvent>()).name).read() }.as_ptr()
	}

	#[test]
	fn loadouts_keep_each_wearable_once_up_to_the_networked_limit() {
		let mut loadout = Loadout::new().with_wearable(def(106)).unwrap();

		assert_eq!(loadout.insert_wearable(def(106)), Ok(false));
		assert_eq!(loadout.wearables(), [def(106)]);

		// Index zero is a valid definition.
		for index in [0, 378, 116, 30, 31, 32, 33] {
			assert_eq!(loadout.insert_wearable(def(index)), Ok(true));
		}

		assert_eq!(loadout.wearables().len(), MAX_NETWORKED_WEARABLES);
		assert_eq!(
			loadout.insert_wearable(def(34)),
			Err(LoadoutError::TooManyWearables)
		);
		assert_eq!(
			loadout.clone().with_wearable(def(34)),
			Err(LoadoutError::TooManyWearables)
		);

		// A wearable the loadout has is still reported as present when full.
		assert_eq!(loadout.insert_wearable(def(378)), Ok(false));

		// Removal keeps the order of the others, and frees room.
		assert!(loadout.remove_wearable(def(378)));
		assert!(!loadout.remove_wearable(def(378)));
		assert_eq!(loadout.wearables(), [106, 0, 116, 30, 31, 32, 33].map(def));
		assert_eq!(loadout.insert_wearable(def(34)), Ok(true));
		assert_eq!(loadout.wearables().last(), Some(&def(34)));
		assert!(!loadout.is_empty());
	}

	#[test]
	fn loadouts_keep_weapon_attributes_by_native_slot() {
		let mut loadout = Loadout::new();

		assert!(loadout.is_empty());
		assert_eq!(
			loadout.insert_weapon_attributes(WeaponSlot::Melee, damage_bonus()),
			None
		);
		assert!(!loadout.is_empty());

		// A named slot and its raw number are the same slot.
		assert_eq!(loadout.weapon_attributes(2), Some(&damage_bonus()));

		// An empty set removes the slot's, so the loadout wants nothing again.
		assert_eq!(
			loadout.insert_weapon_attributes(2, AttributeSet::new()),
			Some(damage_bonus())
		);
		assert_eq!(loadout.weapon_attributes(WeaponSlot::Melee), None);
		assert!(loadout.is_empty());
		assert_eq!(
			loadout.insert_weapon_attributes(WeaponSlot::Melee, AttributeSet::new()),
			None
		);
		assert!(loadout.is_empty());

		// Slots iterate in native order, whatever the insertion order.
		let mut loadout = loadout
			.with_weapon_attributes(7, damage_bonus())
			.with_weapon_attributes(WeaponSlot::Melee, damage_bonus())
			.with_weapon_attributes(WeaponSlot::Secondary, AttributeSet::new())
			.with_weapon_attributes(WeaponSlot::Primary, damage_bonus());
		let slots: Vec<c_int> = loadout.weapons().map(|(slot, _)| slot).collect();

		assert_eq!(slots, [0, 2, 7]);
		assert_eq!(
			loadout.remove_weapon_attributes(WeaponSlot::Primary),
			Some(damage_bonus())
		);
		assert_eq!(loadout.remove_weapon_attributes(WeaponSlot::Primary), None);
		assert_eq!(loadout.weapon_attributes(WeaponSlot::Primary), None);
		assert_eq!(loadout.weapons().count(), 2);
	}

	unsafe extern "C" fn no_edict(_: *mut sys::IVEngineServer, _: c_int) -> *mut sys::edict_t {
		null_mut()
	}

	unsafe extern "C" fn no_interfaces(_: *const c_char, _: *mut c_int) -> *mut c_void {
		null_mut()
	}

	#[test]
	fn only_post_inventory_application_for_known_players_is_handled() {
		let scope = ();
		let server = mock_server(&scope);
		let token = unsafe { trust_shipped_schema(server) };
		let mut reapplier = LoadoutReapplier::default();
		let handle = |reapplier: &LoadoutReapplier, name, user_id| {
			let mut event = MockEvent::new(name, user_id);

			unsafe { reapplier.on_game_event(server, event.event(), token) }
		};

		// No loadout at all: nothing is asked of the engine, which the mock
		// server does not export.
		assert!(matches!(
			handle(&reapplier, c"post_inventory_application", Some(2)),
			Ok(None)
		));

		reapplier.set(user(2), Loadout::new().with_wearable(def(106)).unwrap());
		reapplier.set(user(3), Loadout::new());

		// Other events are ignored, even for players with a loadout.
		for name in [c"player_spawn", c"player_regenerate", c"post_inventory"] {
			assert!(matches!(handle(&reapplier, name, Some(2)), Ok(None)));
		}

		// Players without a loadout, or with an empty one, are ignored.
		assert!(matches!(
			handle(&reapplier, c"post_inventory_application", Some(4)),
			Ok(None)
		));
		assert!(matches!(
			handle(&reapplier, c"post_inventory_application", Some(3)),
			Ok(None)
		));

		// A malformed `userid` is an error.
		for user_id in [None, Some(0), Some(-1), Some(65_536)] {
			assert!(matches!(
				handle(&reapplier, c"post_inventory_application", user_id),
				Err(LoadoutError::InvalidUserId)
			));
		}

		// A player with a loadout goes on to be resolved, which needs the
		// engine's interface.
		assert!(matches!(
			handle(&reapplier, c"post_inventory_application", Some(2)),
			Err(LoadoutError::Interface(_))
		));
		assert!(!reapplier.is_applying());
	}

	#[test]
	fn pruning_keeps_the_loadouts_of_connected_clients() {
		let scope = ();
		let mut reapplier = LoadoutReapplier::new();
		let loadout = Loadout::new().with_wearable(def(106)).unwrap();
		let kept = |reapplier: &LoadoutReapplier| -> Vec<UserId> {
			reapplier.iter().map(|(user_id, _)| user_id).collect()
		};

		for id in [2, 3, 4, 5, 6] {
			reapplier.set(user(id), loadout.clone());
		}

		// Without the engine's interface, nothing is forgotten.
		assert!(matches!(
			reapplier.retain_connected(mock_server(&scope)),
			Err(LoadoutError::Interface(_))
		));
		assert_eq!(kept(&reapplier), [2, 3, 4, 5, 6].map(user));

		// A player, a client still loading the level, a bot and an empty
		// slot. User IDs 3 and 6 left without `player_disconnect`, as bots
		// are expected to at a level change.
		let loading = MockClient {
			active: false,
			..MockClient::active(4)
		};
		let bot = MockClient {
			fake: true,
			..MockClient::active(5)
		};
		let mock = MockEngine::new(
			&[MockClient::active(2), loading, MockClient::default(), bot],
			c"0",
			&[],
		);

		reapplier.retain_connected(mock.server()).unwrap();
		assert_eq!(kept(&reapplier), [2, 4, 5].map(user));

		// Loadouts can also be kept, and changed, by any rule.
		reapplier.retain(|user_id, loadout| {
			if user_id == user(5) {
				loadout.remove_wearable(def(106));
			}

			user_id != user(4)
		});
		assert_eq!(kept(&reapplier), [2, 5].map(user));
		assert_eq!(reapplier.get(user(5)), Some(&Loadout::new()));
		assert_eq!(reapplier.get(user(2)), Some(&loadout));
	}

	#[test]
	fn reappliers_keep_loadouts_by_user_id() {
		let mut reapplier = LoadoutReapplier::new();
		let first = Loadout::new().with_wearable(def(106)).unwrap();
		let second = Loadout::new().with_wearable(def(116)).unwrap();

		assert_eq!(reapplier.get(user(2)), None);
		assert_eq!(reapplier.set(user(2), first.clone()), None);
		assert_eq!(reapplier.set(user(3), first.clone()), None);
		assert_eq!(reapplier.get(user(2)), Some(&first));

		// Setting again replaces, and returns the previous loadout.
		assert_eq!(reapplier.set(user(2), second.clone()), Some(first.clone()));
		assert_eq!(reapplier.get(user(2)), Some(&second));

		// Loadouts change in place.
		reapplier
			.get_mut(user(3))
			.unwrap()
			.insert_weapon_attributes(WeaponSlot::Secondary, damage_bonus());
		assert_eq!(
			reapplier
				.get(user(3))
				.and_then(|loadout| loadout.weapon_attributes(WeaponSlot::Secondary)),
			Some(&damage_bonus())
		);
		assert_eq!(reapplier.get_mut(user(4)), None);

		// Forgetting one player leaves the others.
		assert_eq!(reapplier.forget(user(2)), Some(second));
		assert_eq!(reapplier.forget(user(2)), None);
		assert_eq!(reapplier.get(user(2)), None);
		assert!(reapplier.get(user(3)).is_some());

		reapplier.clear();
		assert_eq!(reapplier.get(user(3)), None);
		assert_eq!(reapplier.iter().count(), 0);
		assert!(!reapplier.is_applying());
	}

	fn user(id: u16) -> UserId {
		UserId::new(id).unwrap()
	}
}
