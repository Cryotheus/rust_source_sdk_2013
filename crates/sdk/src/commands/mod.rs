//! Console commands implemented in Rust.
//!
//! A [`ConsoleCommand`] is laid out as the engine's `ConCommand`, so the
//! engine lists, finds, and runs it like any command a game module declares.
//! Registering one takes it pinned and `'static`, since the engine keeps its
//! address.
//!
//! # Who runs a command
//!
//! [`Invoker`] tells a handler where an invocation came from:
//!
//! - [`Invoker::Server`]: the server console, rcon, a config file,
//!   [`ValveEngine::server_command`], or other server-side code. The engine
//!   delivers these through the command's `Dispatch`.
//! - [`Invoker::Client`]: a string command a connected client sent. The
//!   engine only dispatches a client's command directly when the command is
//!   flagged `FCVAR_GAMEDLL` or is on its short list of engine client commands,
//!   and it cannot tell which client ran it then. Commands from this module
//!   never report that flag, and [`validate_name`] rejects the listed names, so
//!   the engine hands a client's command to `IServerGameClients::ClientCommand`,
//!   together with the client's edict, instead. A plugin hooks that method and passes each call to
//!   [`route_client_command`], which runs the command if it is one of ours and
//!   clients may run it.
//!
//! Attribution therefore follows the path an invocation took, so nested
//! commands and commands run during level changes are never attributed to the
//! wrong invoker. [`CommandAccess`] decides which invokers a command accepts.
//!
//! [`ValveEngine::server_command`]: crate::interfaces::ValveEngine::server_command

mod args;
mod error;
mod object;
mod registrar;
mod route;

#[cfg(test)]
mod tests;

pub use args::{ArgError, CommandArgs};
pub use error::{
	CommandBaseKind, InvalidCommandName, RegisterCommandError, RegisterCommandErrorKind,
	UnregisterCommandError, UnregisterCommandErrorKind, validate_name,
};
pub use object::ConsoleCommand;
pub use registrar::{CommandRegistrar, UnlinksBeforeUnload};
pub use route::{ClientRoute, route_client_command};

use crate::edicts::Edict;
use crate::ffi::NotThreadSafe;
use crate::server::{InterfaceError, Server};
use std::borrow::Cow;
use std::ffi::{CStr, CString, c_int};
use std::fmt::Display;
use std::marker::PhantomData;

/// Runs a console command.
///
/// Commands can run inside one another, for example when a handler makes a
/// client run a command, so state behind `&self` belongs in a `Cell` or
/// `RefCell` whose borrows are not held across calls into the engine.
///
/// A panic is caught, logged to the server console, and reported to the
/// invoker as a failure. It is never an unload, so a client cannot unload the
/// plugin by making a handler panic.
pub trait CommandHandler: 'static {
	fn dispatch(&self, command: &CommandContext<'_>) -> CommandResult;
}

/// A plain function handler, which keeps `ConsoleCommand<CommandFn>` nameable
/// in a `static`.
pub type CommandFn = for<'a, 'd> fn(&'a CommandContext<'d>) -> CommandResult;

impl<F> CommandHandler for F
where
	F: Fn(&CommandContext<'_>) -> CommandResult + 'static,
{
	fn dispatch(&self, command: &CommandContext<'_>) -> CommandResult {
		self(command)
	}
}

/// Who may run a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CommandAccess {
	/// Server-side invokers only (see [`Invoker::Server`]). Clients that run the
	/// command are told it is unknown, as for any command a client cannot run.
	#[default]
	Server,

	/// Connected clients only. Server-side invokers are told the command is for
	/// players.
	///
	/// Clients' commands reach the game only if no command of ours takes them.
	/// The game also handles some client commands by name without registering
	/// them, such as TF2's `build` and `jointeam`, which the registry cannot
	/// report as taken; a client-runnable command with such a name replaces the
	/// game's handling of it.
	Clients,

	/// Server-side invokers and connected clients. As for
	/// [`Clients`](Self::Clients), clients' invocations never reach the game.
	Everyone,
}

impl CommandAccess {
	const fn allows_server(self) -> bool {
		matches!(self, Self::Server | Self::Everyone)
	}

	const fn allows_clients(self) -> bool {
		matches!(self, Self::Clients | Self::Everyone)
	}
}

/// The `FCVAR_*` flags a command may carry, from `public/tier1/iconvar.h`.
///
/// `FCVAR_GAMEDLL` cannot be expressed: the engine would then dispatch
/// clients' invocations without saying who ran them (see the
/// [module documentation](self)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CommandFlags(c_int);

impl CommandFlags {
	pub const NONE: Self = Self(0);

	/// `FCVAR_HIDDEN`: left out of `find`, `cvarlist`, and completion.
	pub const HIDDEN: Self = Self(1 << 4);

	/// `FCVAR_CHEAT`: runnable only while `sv_cheats` is set. The engine checks
	/// this for server-side invokers, and [`route_client_command`] for clients.
	pub const CHEAT: Self = Self(1 << 14);

	/// `FCVAR_DONTRECORD`: left out of demo recordings.
	pub const DONT_RECORD: Self = Self(1 << 17);

	/// `FCVAR_GAMEDLL`, which commands never report.
	pub(crate) const GAME_DLL: c_int = 1 << 2;

	pub const fn union(self, other: Self) -> Self {
		Self(self.0 | other.0)
	}

	pub const fn contains(self, other: Self) -> bool {
		self.0 & other.0 == other.0
	}

	pub const fn bits(self) -> c_int {
		self.0
	}
}

impl std::ops::BitOr for CommandFlags {
	type Output = Self;

	fn bitor(self, other: Self) -> Self {
		self.union(other)
	}
}

/// Where an invocation came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Invoker<'d> {
	/// Not a client's string command: the console, rcon, a config file,
	/// `ServerCommand`, a map's `point_servercommand`, or server-side code
	/// dispatching the command directly.
	Server,

	/// A string command from a connected client, including one server code
	/// made the client run.
	Client(Client<'d>),
}

/// The client that ran a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Client<'d> {
	edict: Edict<'d>,
}

impl<'d> Client<'d> {
	/// The edict of the client's player.
	pub const fn edict(self) -> Edict<'d> {
		self.edict
	}

	/// The client's slot, one less than its edict's index.
	pub fn slot(self) -> c_int {
		self.edict.index() - 1
	}
}

/// One invocation of a command, passed to its handler.
pub struct CommandContext<'d> {
	server: Server<'d>,
	args: CommandArgs<'d>,
	invoker: Invoker<'d>,
	name: &'static CStr,
	_not_thread_safe: NotThreadSafe,
}

impl<'d> CommandContext<'d> {
	pub const fn server(&self) -> Server<'d> {
		self.server
	}

	pub const fn args(&self) -> CommandArgs<'d> {
		self.args
	}

	pub const fn invoker(&self) -> Invoker<'d> {
		self.invoker
	}

	/// The name the command was registered under.
	///
	/// [`CommandArgs::name`] is the name as typed, whose case may differ.
	pub const fn name(&self) -> &'static CStr {
		self.name
	}

	/// Prints a line to whoever ran the command: the server console, which
	/// rcon also receives, or the client's console.
	///
	/// NUL characters, which C strings cannot hold, are dropped.
	pub fn reply(&self, message: impl Display) -> Result<(), InterfaceError> {
		let line = line_from(message);

		match self.invoker {
			Invoker::Server => self.server.console_print(&line),
			Invoker::Client(client) => self
				.server
				.valve_engine()?
				.client_print(client.edict, &line),
		}

		Ok(())
	}
}

impl std::fmt::Debug for CommandContext<'_> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("CommandContext")
			.field("name", &self.name)
			.field("args", &self.args)
			.field("invoker", &self.invoker)
			.finish_non_exhaustive()
	}
}

/// Formats a message as a terminated line for the engine's print functions.
fn line_from(message: impl Display) -> CString {
	let mut bytes = message.to_string().into_bytes();

	bytes.retain(|&byte| byte != 0);
	bytes.push(b'\n');

	// SAFETY: Every NUL was removed.
	unsafe { CString::from_vec_unchecked(bytes) }
}

/// What a handler returns.
pub type CommandResult = Result<(), CommandError>;

/// Why an invocation failed. The invoker is shown the command's name and this
/// error's message.
#[derive(Debug, thiserror::Error)]
pub enum CommandError {
	/// The arguments did not match the command's usage, shown as given.
	#[error("usage: {0}")]
	Usage(Cow<'static, str>),

	/// The invoker may not do what it asked.
	#[error("{0}")]
	Denied(Cow<'static, str>),

	#[error(transparent)]
	Argument(#[from] ArgError),

	#[error(transparent)]
	Interface(#[from] InterfaceError),

	#[error(transparent)]
	Other(Box<dyn std::error::Error>),
}

impl CommandError {
	pub fn usage(usage: impl Into<Cow<'static, str>>) -> Self {
		Self::Usage(usage.into())
	}

	pub fn denied(reason: impl Into<Cow<'static, str>>) -> Self {
		Self::Denied(reason.into())
	}

	pub fn other(error: impl std::error::Error + 'static) -> Self {
		Self::Other(Box::new(error))
	}
}

impl<'d> CommandContext<'d> {
	pub(crate) const fn new(
		server: Server<'d>,
		args: CommandArgs<'d>,
		invoker: Invoker<'d>,
		name: &'static CStr,
	) -> Self {
		Self {
			server,
			args,
			invoker,
			name,
			_not_thread_safe: PhantomData,
		}
	}
}
