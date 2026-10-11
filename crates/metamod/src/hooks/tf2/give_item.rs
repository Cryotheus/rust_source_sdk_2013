//! TF2 item hooks, which run before the game's `CTFPlayer::GiveNamedItem` for
//! the players of the classes they cover, as the game creates an item for a
//! player, and may refuse it.
//!
//! The game gives a player their loadout's weapons and cosmetics through it
//! as they spawn and resupply, from the economy items it was given, and the
//! classnames' stock items for what the player has not equipped. Commands
//! and plugins giving items by classname go through it too, such as
//! [`PlayerWeapons::give`](source_sdk_2013::tf2::weapons::PlayerWeapons::give).
//! [`PlayerWeapons::give_item`](source_sdk_2013::tf2::weapons::PlayerWeapons::give_item)
//! and its siblings do not: they create items through the game's item
//! generation directly. The hooks cover classes as [`crate::hooks::tf2::class`]
//! describes: cover the players' classes, those of
//! [`ClassTargets::players`](source_sdk_2013::tf2::class_targets::ClassTargets::players),
//! to hook every item given to players.
//!
//! Under SourceHook, `GiveNamedItem` takes a hook manager, which every handle
//! shares.

#[cfg(test)]
#[path = "../../tests/hooks/tf2/give_item.rs"]
mod tests;

use crate::MetamodApi;
use crate::hooks::tf2::class::ClassHooks;
use crate::hook::{HookAction, HookCall, HookTiming, VirtualFunction};
use source_sdk_2013::entities::Entity;
use source_sdk_2013::raw::tf2::virtuals::{GIVE_NAMED_ITEM_SLOT, GiveNamedItemFn as GiveNamedItem};
use source_sdk_2013::tf2::class_targets::TfPlayer;
use source_sdk_2013::tf2::weapons::ItemDefinitionIndex;
use source_sdk_2013::{Server, ServerBinding, sys};
use std::ffi::{CStr, c_int};
use std::ptr;

/// A callback-scoped server, the player, and the item the game is about to
/// give them, deciding whether it may. A panic is contained by the hook
/// dispatcher, and lets the game give the item.
pub type GiveItemFn = for<'s> fn(Server<'s>, Entity<'s>, &GiveItem<'_>) -> GiveItemAction;

/// `GiveNamedItem` in a TF2 player's primary vtable.
const GIVE_NAMED_ITEM: VirtualFunction<GiveNamedItem> = VirtualFunction::new(GIVE_NAMED_ITEM_SLOT);

/// An item the game is about to give a player, as `CTFPlayer::GiveNamedItem`
/// is asked for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GiveItem<'a> {
	/// The classname of the entity to create, such as
	/// `tf_weapon_rocketlauncher`, which the game translates for the player's
	/// class unless [`Self::force`] is set, as `tf_weapon_shotgun` is to the
	/// class's own shotgun.
	pub class_name: &'a CStr,

	/// The economy item's definition, such as one of the player's loadout, or
	/// `None` for the classname's stock item.
	pub definition: Option<ItemDefinitionIndex>,

	/// Whether the game creates the classname as named, and even if the player
	/// already carries a weapon of it, rather than translating it for their
	/// class, and giving nothing then.
	pub force: bool,

	/// The subtype of the item, which tells weapons of one classname apart,
	/// such as TF2's builders of each building.
	pub subtype: c_int,
}

/// Whether the game may give a player an item.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum GiveItemAction {
	/// Lets the game give the item.
	#[default]
	Continue,

	/// Refuses the item, skipping the game's method, and the hooks of other
	/// plugins that would run after this one: the game gives nothing in its
	/// place, as when it fails to create an item.
	Refuse,
}

impl MetamodApi<'_> {
	/// Runs `callback` before each `CTFPlayer::GiveNamedItem` of the players
	/// of the classes the returned hooks cover, which are none until
	/// [`ClassHooks::cover`] covers them: as the game creates an item for a
	/// player, and has them pick it up.
	///
	/// [`GiveItemAction::Refuse`] leaves the item's loadout slot empty, which
	/// can leave a player without any weapon.
	///
	/// `binding` must describe the running server. The callback must not
	/// delete entities immediately, as [`Server::new`] requires.
	pub fn hook_give_items(
		self,
		binding: ServerBinding,
		callback: GiveItemFn,
	) -> ClassHooks<TfPlayer> {
		ClassHooks::new(
			binding,
			callback,
			GIVE_NAMED_ITEM,
			&[HookTiming::Pre],
			decide,
		)
	}
}

/// Asks `callback` whether the game may give `player` the item.
fn decide<'s>(
	server: Server<'s>,
	callback: GiveItemFn,
	player: Entity<'s>,
	call: &HookCall<'_, GiveNamedItem>,
) -> HookAction<*mut sys::CBaseEntity> {
	// An earlier hook already decided.
	if call.superseded() == Some(true) {
		return HookAction::Ignore;
	}

	let (class_name, subtype, item, force) = call.args();

	if class_name.is_null() {
		return HookAction::Ignore;
	}

	// SAFETY: The game names the classname as a C string, which stays live
	// through the call.
	let class_name = unsafe { CStr::from_ptr(class_name) };

	// SAFETY: The game passes the economy item to give, or none, which stays
	// live through the call.
	let definition = unsafe { item.as_ref() }
		.and_then(|item| ItemDefinitionIndex::new(item.m_iItemDefinitionIndex.m_Value));

	let item = GiveItem {
		class_name,
		definition,
		force,
		subtype,
	};

	match callback(server, player, &item) {
		GiveItemAction::Continue => HookAction::Ignore,
		GiveItemAction::Refuse => HookAction::Supersede(ptr::null_mut()),
	}
}
