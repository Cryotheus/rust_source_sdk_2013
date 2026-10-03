//! Console commands and variables implemented in Rust.
//!
//! A [`ConsoleCommand`] is laid out as the engine's `ConCommand`, so the
//! engine lists, finds, and runs it like any command a game module declares.
//! A [`ConsoleVariable`] is laid out as its `ConVar` likewise. Registering
//! either takes it pinned and `'static`, since the engine keeps its address.
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
mod variable;

#[cfg(test)]
mod tests;

use crate::edicts::Edict;
use crate::ffi::NotThreadSafe;
use crate::server::{InterfaceError, Server};
use std::borrow::Cow;
use std::ffi::{CStr, CString, c_int};
use std::fmt::Display;
use std::marker::PhantomData;

pub use args::{ArgError, CommandArgs};

pub use error::{
	CommandBaseKind, InvalidCommandName, RegisterCommandError, RegisterCommandErrorKind,
	UnregisterCommandError, UnregisterCommandErrorKind, validate_name,
};

pub use object::ConsoleCommand;
pub use registrar::{CommandRegistrar, UnlinksBeforeUnload};
pub use route::{ClientRoute, route_client_command};
pub use source_sdk_2013_declmacros::commands;
pub use variable::ConsoleVariable;

/// A plain function handler, which keeps `ConsoleCommand<CommandFn>` nameable
/// in a `static`.
pub type CommandFn = for<'a, 'd> fn(&'a CommandContext<'d>) -> CommandResult;

/// What a handler returns.
pub type CommandResult = Result<(), CommandError>;

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
	/// game's handling of it, except for the invocations its handler leaves
	/// [unhandled](CommandError::Unhandled).
	Clients,

	/// Server-side invokers and connected clients. As for
	/// [`Clients`](Self::Clients), clients' invocations never reach the game.
	Everyone,
}

impl CommandAccess {
	const fn allows_clients(self) -> bool {
		matches!(self, Self::Clients | Self::Everyone)
	}

	const fn allows_server(self) -> bool {
		matches!(self, Self::Server | Self::Everyone)
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
	/// The arguments, copied from the engine before the handler runs, so
	/// commands run in the meantime cannot change them.
	pub const fn args(&self) -> CommandArgs<'d> {
		self.args
	}

	/// Where the invocation came from.
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
	/// NUL characters, which C strings cannot hold, are dropped. Only a reply
	/// to a client can fail, if the engine does not export `IVEngineServer` at
	/// the version the bindings expect.
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

	/// The server, scoped to this invocation.
	pub const fn server(&self) -> Server<'d> {
		self.server
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

impl std::fmt::Debug for CommandContext<'_> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("CommandContext")
			.field("name", &self.name)
			.field("args", &self.args)
			.field("invoker", &self.invoker)
			.finish_non_exhaustive()
	}
}

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

	/// An argument is missing or cannot be read as requested, as
	/// [`CommandArgs::get_str`] and [`CommandArgs::parse`] report.
	#[error(transparent)]
	Argument(#[from] ArgError),

	/// A module, the engine or the game server, does not export an interface
	/// the handler needs at the version the bindings expect.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// Any other error, shown by its message.
	#[error(transparent)]
	Other(Box<dyn std::error::Error>),

	/// The handler leaves the invocation to whoever would have handled it had
	/// the command not been registered, and nothing is printed.
	///
	/// A client's invocation goes on to the game, so a client-runnable command
	/// named after one the game handles itself, such as TF2's `jointeam`, can
	/// take over only some invocations. A server-side invocation has nothing
	/// to go on to, so it just ends.
	#[error("the command was left unhandled")]
	Unhandled,
}

impl CommandError {
	/// A [`Denied`](Self::Denied) error, which shows `reason`.
	pub fn denied(reason: impl Into<Cow<'static, str>>) -> Self {
		Self::Denied(reason.into())
	}

	/// Boxes any error as [`Other`](Self::Other), for use with
	/// [`Result::map_err`].
	pub fn other(error: impl std::error::Error + 'static) -> Self {
		Self::Other(Box::new(error))
	}

	/// A [`Usage`](Self::Usage) error, which shows `usage` after `usage: `.
	pub fn usage(usage: impl Into<Cow<'static, str>>) -> Self {
		Self::Usage(usage.into())
	}
}

/// The `FCVAR_*` flags a command or variable may carry, from
/// `public/tier1/iconvar.h`.
///
/// Flags read from the engine, such as [`ConVar::flags`], keep every bit,
/// including those without a constant here.
///
/// A [`ConsoleCommand`] is never registered with [`GAME_DLL`](Self::GAME_DLL),
/// even if its flags contain it: the engine would then dispatch clients'
/// invocations without saying who ran them (see the
/// [module documentation](self)). A [`ConsoleVariable`] keeps it, since the
/// engine only dispatches commands that way.
///
/// [`ConVar::flags`]: crate::interfaces::cvar::ConVar::flags
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CommandFlags(c_int);

impl CommandFlags {
	/// `FCVAR_ARCHIVE`: a variable the engine saves when it writes its
	/// configuration.
	#[doc(alias = "FCVAR_ARCHIVE")]
	pub const ARCHIVE: Self = Self(1 << 7);

	/// `FCVAR_CHEAT`: runnable only while `sv_cheats` is set. The engine checks
	/// this for server-side invokers, and [`route_client_command`] for clients.
	#[doc(alias = "FCVAR_CHEAT")]
	pub const CHEAT: Self = Self(1 << 14);

	/// `FCVAR_CLIENTCMD_CAN_EXECUTE`: a client command or variable that the
	/// client library's `IVEngineClient::ClientCmd` may run, which it refuses
	/// otherwise. `ClientCmd_Unrestricted` runs either.
	#[doc(alias = "FCVAR_CLIENTCMD_CAN_EXECUTE")]
	pub const CLIENT_CMD_CAN_EXECUTE: Self = Self(1 << 30);

	/// `FCVAR_CLIENTDLL`: declared by the client library, which shares a listen
	/// server's registry.
	#[doc(alias = "FCVAR_CLIENTDLL")]
	pub const CLIENT_DLL: Self = Self(1 << 3);

	/// `FCVAR_DEMO`: a variable recorded when a demo recording starts.
	#[doc(alias = "FCVAR_DEMO")]
	pub const DEMO: Self = Self(1 << 16);

	/// `FCVAR_DEVELOPMENTONLY`: hidden from the console in released builds of
	/// the engine.
	#[doc(alias = "FCVAR_DEVELOPMENTONLY")]
	pub const DEVELOPMENT_ONLY: Self = Self(1 << 1);

	/// `FCVAR_DONTRECORD`: left out of demo recordings.
	#[doc(alias = "FCVAR_DONTRECORD")]
	pub const DONT_RECORD: Self = Self(1 << 17);

	/// `FCVAR_GAMEDLL`: declared by the game server library.
	///
	/// A [`ConsoleCommand`] registered from Rust never carries or reports it,
	/// while a [`ConsoleVariable`] keeps it.
	#[doc(alias = "FCVAR_GAMEDLL")]
	pub const GAME_DLL: Self = Self(1 << 2);

	/// `FCVAR_HIDDEN`: left out of `find`, `cvarlist`, and completion.
	#[doc(alias = "FCVAR_HIDDEN")]
	pub const HIDDEN: Self = Self(1 << 4);

	/// `FCVAR_NEVER_AS_STRING`: a variable whose string is never updated from
	/// its value, so it keeps the one it was declared with.
	#[doc(alias = "FCVAR_NEVER_AS_STRING")]
	pub const NEVER_AS_STRING: Self = Self(1 << 12);

	/// `FCVAR_NONE`: no flags, the default.
	#[doc(alias = "FCVAR_NONE")]
	pub const NONE: Self = Self(0);

	/// `FCVAR_NOT_CONNECTED`: a client variable that clients cannot change while
	/// connected to a server, unless the game's
	/// `CGameRules::IsConnectedUserInfoChangeAllowed` allows it.
	#[doc(alias = "FCVAR_NOT_CONNECTED")]
	pub const NOT_CONNECTED: Self = Self(1 << 22);

	/// `FCVAR_NOTIFY`: changes to a variable are announced to players and
	/// written to the server log.
	#[doc(alias = "FCVAR_NOTIFY")]
	pub const NOTIFY: Self = Self(1 << 8);

	/// `FCVAR_PRINTABLEONLY`: a variable whose string may only contain
	/// printable characters, such as a player's name. The variable's own
	/// `SetValue` does not check this, so [`ConVar::set_string`] does not
	/// either.
	///
	/// [`ConVar::set_string`]: crate::interfaces::cvar::ConVar::set_string
	#[doc(alias = "FCVAR_PRINTABLEONLY")]
	pub const PRINTABLE_ONLY: Self = Self(1 << 10);

	/// `FCVAR_PROTECTED`: a server variable whose value is withheld, such as a
	/// password. Queries of the server's rules learn only whether it is set.
	#[doc(alias = "FCVAR_PROTECTED")]
	pub const PROTECTED: Self = Self(1 << 5);

	/// `FCVAR_REPLICATED`: a variable whose server value is sent to every
	/// client, whose own copy follows it.
	#[doc(alias = "FCVAR_REPLICATED")]
	pub const REPLICATED: Self = Self(1 << 13);

	/// `FCVAR_SERVER_CAN_EXECUTE`: a client command clients run when the
	/// server sends it to them, such as through `IVEngineServer::ClientCommand`.
	#[doc(alias = "FCVAR_SERVER_CAN_EXECUTE")]
	pub const SERVER_CAN_EXECUTE: Self = Self(1 << 28);

	/// `FCVAR_SERVER_CANNOT_QUERY`: a client variable whose value clients refuse
	/// to report to the server, as
	/// [`PluginHelpers::start_query_cvar_value`] asks them to.
	///
	/// [`PluginHelpers::start_query_cvar_value`]: crate::interfaces::PluginHelpers::start_query_cvar_value
	#[doc(alias = "FCVAR_SERVER_CANNOT_QUERY")]
	pub const SERVER_CANNOT_QUERY: Self = Self(1 << 29);

	/// `FCVAR_SPONLY`: meant for single-player games; clients connected to a
	/// multiplayer server cannot change a variable marked with it.
	#[doc(alias = "FCVAR_SPONLY")]
	pub const SP_ONLY: Self = Self(1 << 6);

	/// `FCVAR_UNLOGGED`: changes to a variable are not written to the server
	/// log, even if it is marked [`NOTIFY`](Self::NOTIFY).
	#[doc(alias = "FCVAR_UNLOGGED")]
	pub const UNLOGGED: Self = Self(1 << 11);

	/// `FCVAR_USERINFO`: a client variable whose value clients send to the
	/// server, which [`ValveEngine::client_convar_value`] reads.
	///
	/// [`ValveEngine::client_convar_value`]: crate::interfaces::ValveEngine::client_convar_value
	#[doc(alias = "FCVAR_USERINFO")]
	pub const USERINFO: Self = Self(1 << 9);

	/// The flags the engine stores as `bits` in `ConCommandBase::m_nFlags`,
	/// keeping bits without a constant here.
	pub const fn from_bits_retain(bits: c_int) -> Self {
		Self(bits)
	}

	/// The flags as the engine stores them in `ConCommandBase::m_nFlags`.
	pub const fn bits(self) -> c_int {
		self.0
	}

	/// Whether every flag set in `other` is also set in `self`.
	pub const fn contains(self, other: Self) -> bool {
		self.0 & other.0 == other.0
	}

	/// The flags set in either `self` or `other`, as `|` gives, in `const`
	/// contexts too.
	pub const fn union(self, other: Self) -> Self {
		Self(self.0 | other.0)
	}
}

impl std::ops::BitOr for CommandFlags {
	type Output = Self;

	fn bitor(self, other: Self) -> Self {
		self.union(other)
	}
}

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
	/// Runs one invocation. An error is shown to the invoker, except
	/// [`CommandError::Unhandled`], which passes the invocation on.
	fn dispatch(&self, command: &CommandContext<'_>) -> CommandResult;
}

impl<F> CommandHandler for F
where
	F: Fn(&CommandContext<'_>) -> CommandResult + 'static,
{
	fn dispatch(&self, command: &CommandContext<'_>) -> CommandResult {
		self(command)
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

/// Formats a message as a terminated line for the engine's print functions.
fn line_from(message: impl Display) -> CString {
	let mut bytes = message.to_string().into_bytes();

	bytes.retain(|&byte| byte != 0);
	bytes.push(b'\n');

	// SAFETY: Every NUL was removed.
	unsafe { CString::from_vec_unchecked(bytes) }
}
