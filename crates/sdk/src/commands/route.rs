//! Running commands for the engine and for clients.

use super::args::{CommandArgs, CommandLine};
use super::object::{CommandHeader, RegisteredCommand};
use super::{Client, CommandContext, CommandFlags, Invoker, line_from};
use crate::edicts::Edict;
use crate::interfaces::Cvar;
use crate::players::ABSOLUTE_PLAYER_LIMIT;
use crate::server::{Server, ServerBinding};
use std::any::Any;
use std::mem;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;

/// What [`route_client_command`] did with a client's command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[must_use]
pub enum ClientRoute {
	/// The command is one of ours and ran, or was refused with a reply. The
	/// game must not handle it too, or it would report an unknown command.
	Handled,

	/// Not a command clients may run here: let the game handle it as usual.
	NotRouted,
}

impl ClientRoute {
	pub const fn is_handled(self) -> bool {
		matches!(self, Self::Handled)
	}
}

/// Runs a client's string command if it names a command registered through
/// this crate that clients may run.
///
/// Call this from a hook on `IServerGameClients::ClientCommand` that runs
/// before the game's own handler, and stop the call from reaching the game
/// when this returns [`ClientRoute::Handled`].
///
/// # Safety
///
/// Call it only from such a hook, on the server's main thread, passing the
/// arguments of the engine's call: a slot of the edict table and the command
/// the engine tokenized.
pub unsafe fn route_client_command(
	binding: &ServerBinding,
	edict: NonNull<sys::edict_t>,
	command: NonNull<sys::CCommand>,
) -> ClientRoute {
	let routed = catch_unwind(AssertUnwindSafe(|| {
		let scope = ();

		// SAFETY: The caller runs this inside the engine's call to the game, on
		// the main thread, which `scope` does not outlive.
		let server = unsafe { binding.server(&scope) };

		// SAFETY: The caller passes the engine's arguments.
		unsafe { route(server, edict, command) }
	}));

	routed.unwrap_or_else(|payload| {
		drop_payload(payload);
		ClientRoute::NotRouted
	})
}

/// # Safety
///
/// As for [`route_client_command`].
unsafe fn route(
	server: Server<'_>,
	edict: NonNull<sys::edict_t>,
	command: NonNull<sys::CCommand>,
) -> ClientRoute {
	let Ok(cvar) = server.cvar() else {
		return ClientRoute::NotRouted;
	};

	// SAFETY: The engine's command is live and unchanged for the call.
	// Malformed input is the game's to reject.
	let Ok(line) = (unsafe { CommandLine::copy(command) }) else {
		return ClientRoute::NotRouted;
	};

	let args = CommandArgs::new(&line);

	// The engine looks commands up the same way, ignoring case, so a command
	// another module registered later under the same name is theirs.
	let Some(header) = cvar
		.find_command_base(args.name())
		// SAFETY: The registry holds live commands and variables.
		.and_then(|found| unsafe { CommandHeader::from_registered(found) })
		.filter(|header| header.access().allows_clients())
	else {
		return ClientRoute::NotRouted;
	};

	// SAFETY: The engine passes a slot of its edict table, which stays
	// allocated for the call.
	let edict = unsafe { Edict::from_raw(edict) };

	if edict.is_free() || !(1..=ABSOLUTE_PLAYER_LIMIT).contains(&edict.index()) {
		return ClientRoute::NotRouted;
	}

	let context = CommandContext::new(
		server,
		args,
		Invoker::Client(Client { edict }),
		header.name(),
	);

	// The engine checks this itself only for commands it dispatches directly.
	if header.current_flags() & CommandFlags::CHEAT.bits() != 0 && !cheats_allowed(cvar) {
		let _ = context.reply(format_args!(
			"Can't use cheat command {} in multiplayer, unless the server has sv_cheats set to 1.",
			header.name().to_string_lossy(),
		));

		return ClientRoute::Handled;
	}

	run(header, &context);
	ClientRoute::Handled
}

/// Runs a command the engine dispatched: an invocation from the server side.
///
/// # Safety
///
/// `command` must be null or the live command the engine is running, and this
/// must run inside the engine's call on the main thread.
pub(super) unsafe fn dispatch_from_engine(
	header: RegisteredCommand,
	command: *const sys::CCommand,
) {
	let outcome = catch_unwind(AssertUnwindSafe(|| {
		let (Some(binding), Some(command)) = (header.binding(), NonNull::new(command.cast_mut()))
		else {
			return;
		};

		let scope = ();

		// SAFETY: This runs inside the engine's call to the command, on the
		// main thread, which `scope` does not outlive.
		let server = unsafe { binding.server(&scope) };
		let name = header.name().to_string_lossy();

		// SAFETY: The caller passes the engine's live command.
		let line = match unsafe { CommandLine::copy(command) } {
			Ok(line) => line,
			Err(error) => {
				print_to_console(server, format_args!("{name}: {error}"));
				return;
			}
		};

		let context = CommandContext::new(
			server,
			CommandArgs::new(&line),
			Invoker::Server,
			header.name(),
		);

		if !header.access().allows_server() {
			let _ = context.reply(format_args!("{name}: only players can use this command"));
			return;
		}

		run(header, &context);
	}));

	if let Err(payload) = outcome {
		drop_payload(payload);
	}
}

/// Runs the handler, reporting its error or panic to the invoker.
fn run(header: RegisteredCommand, context: &CommandContext<'_>) {
	let name = header.name().to_string_lossy();

	// The error's message is the handler's code too, so it is guarded as well.
	let outcome = catch_unwind(AssertUnwindSafe(|| {
		if let Err(error) = header.dispatch(context) {
			let _ = context.reply(format_args!("{name}: {error}"));
		}
	}));

	let Err(payload) = outcome else {
		return;
	};

	print_to_console(
		context.server(),
		format_args!("{name}: the command panicked: {}", panic_message(&*payload)),
	);

	if matches!(context.invoker(), Invoker::Client(_)) {
		let _ = context.reply(format_args!("{name}: the command failed"));
	}

	drop_payload(payload);
}

fn cheats_allowed(cvar: Cvar<'_>) -> bool {
	cvar.find_var(c"sv_cheats")
		.is_some_and(|cheats| cheats.int() != 0)
}

fn print_to_console(server: Server<'_>, message: std::fmt::Arguments<'_>) {
	server.console_print(&line_from(message));
}

fn panic_message(payload: &(dyn Any + Send)) -> &str {
	payload
		.downcast_ref::<&str>()
		.copied()
		.or_else(|| payload.downcast_ref::<String>().map(String::as_str))
		.unwrap_or("a non-string payload")
}

/// Drops a panic's payload, whose own drop may panic too.
fn drop_payload(payload: Box<dyn Any + Send>) {
	if let Err(nested) = catch_unwind(AssertUnwindSafe(|| drop(payload))) {
		mem::forget(nested);
	}
}
