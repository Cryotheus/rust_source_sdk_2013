//! Callbacks with the bots each run of TF2's `tf_bot_add` command adds,
//! through hooks before and after the command's `Dispatch`.
//!
//! `tf_bot_add` (`game/server/tf/bot/tf_bot.cpp:323-449`) adds its bots while
//! it runs: each joins as a fake client, whose player the game creates in
//! `IServerGameClients::ClientPutInServer`, and is then put on its team and
//! class, with its skill and attributes. The hook before the command notes the
//! user IDs of the players in the server, and the hook after it passes the
//! callback the players with other user IDs that are TF2's bots, in the order
//! of their slots. The callback so sees each bot on its team and class, as
//! [`TfBot`], whose wrappers can change it further.
//!
//! Only `tf_bot_add` is watched: the bots the bot quota (`tf_bot_quota`) adds
//! on its own, Mann vs. Machine's robots, and the bots of `bot_generator`
//! entities join otherwise. A run that adds no bot, such as one the game
//! refuses for not coming from the server, calls nothing.

#[cfg(test)]
#[path = "../../tests/hooks/tf2/bot.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::commands::CommandBaseKind;
use source_sdk_2013::edicts::Edict;
use source_sdk_2013::interfaces::Cvar;
use source_sdk_2013::players::UserId;
use source_sdk_2013::raw::commands::{DISPATCH_SLOT, DispatchFn as Dispatch};
use source_sdk_2013::tf2::bots::TfBot;
use source_sdk_2013::{Server, ServerBinding, sys};
use std::cell::{Cell, RefCell};
use std::ffi::CStr;
use std::ptr::NonNull;

/// A callback-scoped server and the bots a run of `tf_bot_add` added, at least
/// one, in the order of their player slots. A panic is contained by the hook
/// dispatcher.
pub type BotsAddedFn = for<'s> fn(Server<'s>, &[TfBot<'s>]);

/// The command the hooks watch.
const COMMAND: &CStr = c"tf_bot_add";

/// `ConCommand::Dispatch`.
const DISPATCH: VirtualFunction<Dispatch> = VirtualFunction::new(DISPATCH_SLOT);

static ROUTE: BotRoute = BotRoute {
	state: Cell::new(None),
	before: RefCell::new(None),
};

/// The hooks [`MetamodApi::hook_tf_bot_add`] installs.
///
/// Metamod disables them while the plugin is paused, and removes them before
/// it unloads. [`Self::remove`] stops them earlier.
#[must_use = "retain the hooks to remove them"]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BotAddHooks {
	pre: HookId,
	post: HookId,
}

impl BotAddHooks {
	/// Stops the hooks, so no further run of `tf_bot_add` is reported.
	pub fn remove(self, api: MetamodApi<'_>) {
		api.remove_hook(self.pre);
		api.remove_hook(self.post);
		ROUTE.state.set(None);
		ROUTE.before.take();
	}
}

struct BotRoute {
	state: Cell<Option<RoutedBots>>,

	/// The user IDs of the players in the server before the running command,
	/// or `None` outside one.
	before: RefCell<Option<Vec<UserId>>>,
}

impl Handler<Dispatch> for BotRoute {
	fn call(&self, call: &HookCall<'_, Dispatch>) -> HookAction<()> {
		let Some(routed) = self.state.get() else {
			return HookAction::Ignore;
		};

		let scope = ();

		// SAFETY: The hooks run on the main thread, during the engine's call of
		// the command, for the server the binding describes.
		let server = unsafe { routed.binding.server(&scope) };

		match call.timing() {
			HookTiming::Pre => {
				let before = players(server)
					.into_iter()
					.map(|(user_id, _)| user_id)
					.collect();

				self.before.replace(Some(before));
			}

			HookTiming::Post => {
				// The hook was installed during the command.
				let Some(before) = self.before.take() else {
					return HookAction::Ignore;
				};

				let added = players(server)
					.into_iter()
					.filter(|(user_id, _)| !before.contains(user_id))
					.filter_map(|(_, edict)| TfBot::new(server, edict.entity()?).ok())
					.collect::<Vec<_>>();

				if !added.is_empty() {
					(routed.callback)(server, &added);
				}
			}
		}

		HookAction::Ignore
	}
}

// SAFETY: Only the server's main thread reaches it: hooks only run their
// handlers there, and `hook_tf_bot_add` takes a `MetamodApi`, which is
// confined to it. No borrow of `before` is held over a call out.
unsafe impl Sync for BotRoute {}

#[derive(Clone, Copy)]
struct RoutedBots {
	hooks: BotAddHooks,
	binding: ServerBinding,
	callback: BotsAddedFn,
}

impl MetamodApi<'_> {
	/// Passes `callback` the bots each run of `tf_bot_add` adds, after the
	/// run; see the [module documentation](crate::hooks::tf2::bot).
	///
	/// This finds `tf_bot_add` in `cvar`'s registry, which lists it once the
	/// game DLL registered its commands, and hooks its object's
	/// `ConCommand::Dispatch` before and after the call. It fails with
	/// [`HookError::InvalidArgument`] when the registry lists no such command,
	/// as outside TF2. `binding` must describe the server `cvar` belongs to.
	/// Installing again while the hooks are installed returns
	/// [`HookError::AlreadyInstalled`]; removing them allows replacement.
	///
	/// The hooks run even if another plugin's hook superseded the command,
	/// which then adds no bot. With Metamod 2.0, KHook can queue native
	/// activation on its worker when the function already has a detour, so
	/// returned hooks mean registration was accepted, not that the next run
	/// will be seen (see [`crate::hook`]).
	pub fn hook_tf_bot_add(
		self,
		cvar: Cvar<'_>,
		binding: ServerBinding,
		callback: BotsAddedFn,
	) -> Result<BotAddHooks, HookError> {
		if ROUTE
			.state
			.get()
			.is_some_and(|state| self.has_hook(state.hooks.pre))
		{
			return Err(HookError::AlreadyInstalled);
		}

		let command = cvar
			.command_bases()
			.find(|base| {
				base.kind() == CommandBaseKind::Command
					&& base
						.name()
						.to_bytes()
						.eq_ignore_ascii_case(COMMAND.to_bytes())
			})
			.ok_or(HookError::InvalidArgument)?;

		// A command's `ConCommandBase` is at the start of its `ConCommand`.
		let command = NonNull::new(command.as_ptr().cast::<sys::ConCommand>())
			.ok_or(HookError::InvalidArgument)?;

		let target = HookTarget::instance(command);

		ROUTE.state.set(None);
		ROUTE.before.take();

		// SAFETY: A `MetamodApi` only exists during a callback, on the main
		// thread. The registry lists the object as a command, so it is a
		// `ConCommand`, whose vtable has `Dispatch` at the slot. The game DLL
		// registered it, and it lives as long as the game DLL, which outlives
		// the plugin.
		let pre = unsafe { self.add_hook(DISPATCH, target, HookTiming::Pre, &ROUTE) }?;

		// SAFETY: As above.
		let post = unsafe { self.add_hook(DISPATCH, target, HookTiming::Post, &ROUTE) }
			.inspect_err(|_| {
				self.remove_hook(pre);
			})?;

		let hooks = BotAddHooks { pre, post };

		ROUTE.state.set(Some(RoutedBots {
			hooks,
			binding,
			callback,
		}));

		Ok(hooks)
	}
}

/// The user ID and edict of each player slot, up to the client limit, that a
/// client occupies.
fn players(server: Server<'_>) -> Vec<(UserId, Edict<'_>)> {
	let Ok(engine) = server.valve_engine() else {
		return Vec::new();
	};

	let max_clients = server
		.player_info_manager()
		.ok()
		.and_then(|players| players.global_vars())
		.map_or(0, |globals| globals.max_clients());

	(1..=max_clients)
		.filter_map(|index| {
			let edict = engine.edict_of_index(index)?;

			Some((engine.user_id_of_edict(edict)?, edict))
		})
		.collect()
}
