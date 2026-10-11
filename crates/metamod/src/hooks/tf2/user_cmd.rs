//! TF2 player command hooks, which run before and after the game's
//! `CTFPlayer::PlayerRunCommand` for the players of the classes they cover,
//! as the game runs one of the commands a player's client sent, and may change
//! the command.
//!
//! The game runs a player's commands as it simulates the player once a tick,
//! those of bots included, each from their own client's [`UserCmd`]: the
//! command's weapon switch, then its movement, between the player's
//! `PreThink` and `PostThink`, in which their weapon attacks. TF2's own
//! method changes the command first in a few cases, such as to hold a
//! taunting player still, so the hooks before it see the command as the client
//! sent it, and those after it as the game ran it. The hooks cover classes as
//! [`crate::hooks::tf2::class`] describes: cover the players' classes, those of
//! [`ClassTargets::players`](source_sdk_2013::tf2::class_targets::ClassTargets::players),
//! to hook every player's commands.
//!
//! Under SourceHook, `PlayerRunCommand` takes a hook manager, which every
//! handle shares.

#[cfg(test)]
#[path = "../../tests/hooks/tf2/user_cmd.rs"]
mod tests;

use crate::MetamodApi;
use crate::hooks::tf2::class::ClassHooks;
use crate::hook::{HookAction, HookCall, HookTiming, VirtualFunction};
use source_sdk_2013::entities::Entity;
use source_sdk_2013::raw::tf2::virtuals::{PLAYER_RUN_COMMAND_SLOT, RunCommandFn as RunCommand};
use source_sdk_2013::tf2::class_targets::TfPlayer;
use source_sdk_2013::user_cmd::UserCmd;
use source_sdk_2013::{Server, ServerBinding};
use std::ptr::NonNull;

/// A callback-scoped server, whether the game's method is about to run the
/// command or ran it, the player whose command it is, and the command, to
/// read or change. A panic is contained by the hook dispatcher.
pub type RunCommandFn = for<'s> fn(Server<'s>, HookTiming, Entity<'s>, &mut UserCmd);

/// Before the game's method, then after it.
const AROUND: &[HookTiming] = &[HookTiming::Post, HookTiming::Pre];

/// `PlayerRunCommand` in a TF2 player's primary vtable.
const RUN_COMMAND: VirtualFunction<RunCommand> = VirtualFunction::new(PLAYER_RUN_COMMAND_SLOT);

impl MetamodApi<'_> {
	/// Runs `callback` before and after each `CTFPlayer::PlayerRunCommand`
	/// of the players of the classes the returned hooks cover, which are none
	/// until [`ClassHooks::cover`] covers them.
	///
	/// Changes to the command before the method change what the game runs:
	/// clearing its [`Buttons::ATTACK`](source_sdk_2013::user_cmd::Buttons::ATTACK)
	/// keeps the player's weapon from attacking, and zeroing its moves keeps
	/// them still. The player's client predicted the command as it sent it,
	/// and corrects to what the server ran. Changes after the method come too
	/// late for the game's run of the command.
	///
	/// `binding` must describe the running server. The callback must not
	/// delete entities immediately, as [`Server::new`] requires.
	pub fn hook_run_commands(
		self,
		binding: ServerBinding,
		callback: RunCommandFn,
	) -> ClassHooks<TfPlayer> {
		ClassHooks::new(binding, callback, RUN_COMMAND, AROUND, run)
	}
}

/// Gives `callback` the command, which runs on.
fn run<'s>(
	server: Server<'s>,
	callback: RunCommandFn,
	player: Entity<'s>,
	call: &HookCall<'_, RunCommand>,
) -> HookAction<()> {
	let (command, _helper) = call.args();

	let Some(command) = NonNull::new(command) else {
		return HookAction::Ignore;
	};

	// SAFETY: The game passes the command it runs, which it keeps live through
	// the call, and does not touch while its hooks run.
	let command = unsafe { UserCmd::from_raw_mut(command) };

	callback(server, call.timing(), player, command);
	HookAction::Ignore
}
