//! TF2 gunfire hooks: a call each time a player fires a weapon, as the game
//! tells its bots.
//!
//! [`MetamodApi::hook_gunfire`] runs a callback after each
//! `NextBotManager::OnWeaponFired` of the game's bot manager, which
//! [`source_sdk_2013::raw::tf2::gunfire`] describes: as a player's gun,
//! projectile weapon, melee weapon or flame thrower attacks, as a spy's
//! invisibility watch cloaks or uncloaks them, and as a bot bumps into an
//! enemy. The callback gets the player and the weapon. The game tells its bots
//! whether or not any play, but only while a round runs (`GR_STATE_RND_RUNNING`,
//! setup included): not in its pre-game, between rounds, or in a stalemate.
//!
//! The hook calls back only for a call whose player is one of the server's
//! players, and whose weapon is one they carry, comparing addresses before it
//! reads anything of either. A call it cannot match is ignored, so a game whose
//! bot manager changed fails closed. As with other Metamod hooks, the callback
//! stops running while the plugin is paused.

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::entities::Entity;

use source_sdk_2013::raw::tf2::gunfire::{
	BOT_MANAGER_CLASS, ON_WEAPON_FIRED_SLOT, OnWeaponFiredFn as OnWeaponFired,
};

use source_sdk_2013::tf2::class_targets::{ClassTargetError, ClassTargets};
use source_sdk_2013::tf2::weapons::PlayerWeapons;
use source_sdk_2013::{Server, ServerBinding};
use std::cell::Cell;
use std::ffi::c_void;

/// A callback-scoped server, a player who fired, and the weapon they fired.
/// A panic is contained by the hook dispatcher.
pub type GunfireFn = for<'s> fn(Server<'s>, Entity<'s>, Entity<'s>);

/// `OnWeaponFired` in the bot manager's primary vtable.
const ON_WEAPON_FIRED_FUNCTION: VirtualFunction<OnWeaponFired> =
	VirtualFunction::new(ON_WEAPON_FIRED_SLOT);

static ROUTE: GunfireRoute = GunfireRoute {
	state: Cell::new(None),
};

/// Why a gunfire hook could not be installed.
#[derive(Debug, thiserror::Error)]
pub enum GunfireHookError {
	/// The game's bot manager class was not found.
	#[error(transparent)]
	Class(#[from] ClassTargetError),

	/// Metamod refused the hook. [`HookError::AlreadyInstalled`] means it is
	/// installed, which callers installing more than once can ignore.
	#[error(transparent)]
	Hook(#[from] HookError),
}

struct GunfireRoute {
	state: Cell<Option<Routed>>,
}

impl Handler<OnWeaponFired> for GunfireRoute {
	fn call(&self, call: &HookCall<'_, OnWeaponFired>) -> HookAction<()> {
		let Some(route) = self.state.get() else {
			return HookAction::Ignore;
		};

		let (who_fired, weapon) = call.args();

		if who_fired.is_null() || weapon.is_null() {
			return HookAction::Ignore;
		}

		let scope = ();

		// SAFETY: The hook dispatcher runs on the main thread during one live
		// engine invocation. Binding was supplied during plugin integration.
		let server = unsafe { route.binding.server(&scope) };

		let Some((player, weapon)) = fired(server, who_fired.cast(), weapon.cast()) else {
			return HookAction::Ignore;
		};

		(route.callback)(server, player, weapon);
		HookAction::Ignore
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl Sync for GunfireRoute {}

#[derive(Clone, Copy)]
struct Routed {
	binding: ServerBinding,
	callback: GunfireFn,
	hook: HookId,
}

impl MetamodApi<'_> {
	/// Runs `callback` after each time the game tells its bots that a player
	/// fired a weapon; see the [module documentation](crate::hooks::tf2::gunfire).
	///
	/// Finds the bot manager's class in `targets`, which must come from the
	/// server `binding` describes. Installing again while the hook is installed
	/// returns [`HookError::AlreadyInstalled`]. The callback runs for every
	/// shot of every player, so it should be cheap, and must not delete
	/// entities immediately, as [`Server::new`] requires.
	///
	/// With Metamod 2.0, KHook can queue the hook's activation on its worker,
	/// so a returned ID means registration was accepted, not that the next
	/// shot will be seen.
	pub fn hook_gunfire(
		self,
		targets: &ClassTargets<'_>,
		binding: ServerBinding,
		callback: GunfireFn,
	) -> Result<HookId, GunfireHookError> {
		if ROUTE
			.state
			.get()
			.is_some_and(|state| self.has_hook(state.hook))
		{
			return Err(HookError::AlreadyInstalled.into());
		}

		let manager = targets
			.find(BOT_MANAGER_CLASS, ON_WEAPON_FIRED_SLOT)
			.ok_or(ClassTargetError::NotFound(BOT_MANAGER_CLASS))?;

		// SAFETY: TF2's bot manager class derives from `NextBotManager`, which
		// has no base, so its vtable holds `OnWeaponFired` at its slot. The game
		// module stays loaded until Metamod unloads the plugin. The handler
		// reads the arguments only once it found them among live entities.
		let hook = unsafe {
			self.add_hook(
				ON_WEAPON_FIRED_FUNCTION,
				HookTarget::vtable(manager.as_ptr()),
				HookTiming::Post,
				&ROUTE,
			)
		}?;

		ROUTE.state.set(Some(Routed {
			binding,
			callback,
			hook,
		}));
		Ok(hook)
	}
}

/// The player at `who_fired` and their weapon at `weapon`, found by address
/// among the server's players and the weapons they carry, or `None`.
fn fired<'s>(
	server: Server<'s>,
	who_fired: *mut c_void,
	weapon: *mut c_void,
) -> Option<(Entity<'s>, Entity<'s>)> {
	let tools = server.server_tools().ok()?;
	let max_clients = server
		.player_info_manager()
		.ok()?
		.global_vars()
		.map_or(0, |globals| globals.max_clients());

	let player = (1..=max_clients)
		.filter_map(|index| tools.entity_by_index(index))
		.find(|player| player.as_ptr().cast() == who_fired)?;

	let weapon = PlayerWeapons::new(server, player)
		.ok()?
		.all()
		.ok()?
		.into_iter()
		.map(|carried| carried.entity())
		.find(|carried| carried.as_ptr().cast() == weapon)?;

	Some((player, weapon))
}
