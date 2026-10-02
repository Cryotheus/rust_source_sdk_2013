//! Registration errors and command name validation.

use crate::server::InterfaceError;
use std::ffi::CStr;
use std::fmt::{self, Display, Formatter};

/// Names the engine runs for clients itself, through `Dispatch` as though the
/// server had run them, before the game or any hook sees them: the list
/// `CGameClient::ExecuteStringCommand` checks in TF2's 64-bit engine. The
/// `rpt` names are not registered, so the registry cannot report them taken.
const ENGINE_CLIENT_COMMANDS: [&[u8]; 12] = [
	b"status",
	b"pause",
	b"setpause",
	b"unpause",
	b"ping",
	b"rpt",
	b"rpt_server_enable",
	b"rpt_client_enable",
	b"rpt_connect",
	b"rpt_password",
	b"rpt_screenshot",
	b"rpt_download_log",
];

/// The longest command name [`validate_name`] accepts, in bytes.
const MAX_NAME_LENGTH: usize = 63;

/// What already uses a name in the engine's registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandBaseKind {
	/// A console command (`ConCommand`).
	Command,

	/// A console variable (`ConVar`).
	Variable,
}

impl Display for CommandBaseKind {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		f.write_str(match self {
			Self::Command => "command",
			Self::Variable => "variable",
		})
	}
}

/// A command name [`validate_name`] rejects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InvalidCommandName {
	/// The name has no bytes.
	#[error("the name is empty")]
	Empty,

	/// The name is longer than 63 bytes.
	#[error("the name is longer than {MAX_NAME_LENGTH} bytes")]
	TooLong,

	/// A byte is not an ASCII letter, digit, or `_`. Only the first is reported.
	#[error("byte {index} ({byte:#04x}) is not an ASCII letter, digit, or `_`")]
	InvalidByte {
		/// Where the byte is in the name.
		index: usize,

		/// The byte itself.
		byte: u8,
	},

	/// The engine runs commands with this name for clients as though the
	/// server ran them, so their invoker could not be told apart.
	#[error("the engine reserves the name for its own client commands")]
	Reserved,
}

/// A command or variable could not be registered.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("could not register console {base} `{}`: {kind}", .name.to_string_lossy())]
pub struct RegisterCommandError {
	name: &'static CStr,
	base: CommandBaseKind,
	kind: RegisterCommandErrorKind,
}

impl RegisterCommandError {
	pub(crate) const fn new(
		name: &'static CStr,
		base: CommandBaseKind,
		kind: RegisterCommandErrorKind,
	) -> Self {
		Self { name, base, kind }
	}

	/// Whether a command or a variable could not be registered.
	pub const fn base(&self) -> CommandBaseKind {
		self.base
	}

	/// Why it could not be registered.
	pub const fn kind(&self) -> &RegisterCommandErrorKind {
		&self.kind
	}

	/// The name of the command or variable.
	pub const fn name(&self) -> &'static CStr {
		self.name
	}
}

/// Why a command or variable could not be registered.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegisterCommandErrorKind {
	/// The engine lists it already. Registering it again would cut the
	/// engine's list short, since it is relinked in place.
	#[error("it is already registered")]
	AlreadyRegistered,

	/// The engine lists another command or variable, of the given kind, under
	/// the name, ignoring case.
	#[error("the name is already used by a console {0}")]
	NameTaken(CommandBaseKind),

	/// The registrar returned without the engine listing it under its name.
	/// It was unlinked again if the engine marked it registered.
	#[error("the engine did not list it")]
	NotLinked,

	/// The engine does not export `ICvar` at the version the bindings expect.
	#[error(transparent)]
	Interface(#[from] InterfaceError),
}

/// A command or variable could not be unregistered.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("could not unregister console {base} `{}`: {kind}", .name.to_string_lossy())]
pub struct UnregisterCommandError {
	name: &'static CStr,
	base: CommandBaseKind,
	kind: UnregisterCommandErrorKind,
}

impl UnregisterCommandError {
	pub(crate) const fn new(
		name: &'static CStr,
		base: CommandBaseKind,
		kind: UnregisterCommandErrorKind,
	) -> Self {
		Self { name, base, kind }
	}

	/// Whether a command or a variable could not be unregistered.
	pub const fn base(&self) -> CommandBaseKind {
		self.base
	}

	/// Why it could not be unregistered.
	pub const fn kind(&self) -> &UnregisterCommandErrorKind {
		&self.kind
	}

	/// The name of the command or variable.
	pub const fn name(&self) -> &'static CStr {
		self.name
	}
}

/// Why a command or variable could not be unregistered.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UnregisterCommandErrorKind {
	/// The engine does not mark it registered, so there is nothing to unlink.
	#[error("it is not registered")]
	NotRegistered,

	/// The registrar returned with the engine still listing it.
	#[error("the engine still lists it")]
	StillLinked,

	/// The engine does not export `ICvar` at the version the bindings expect.
	#[error(transparent)]
	Interface(#[from] InterfaceError),
}

/// Checks that a name is 1 to 63 ASCII letters, digits, and underscores, and
/// not one the engine reserves for its own client commands, in any case.
///
/// That is stricter than the engine, but every such name can be typed in a
/// console and tokenizes as a single argument.
pub const fn validate_name(name: &CStr) -> Result<(), InvalidCommandName> {
	let bytes = name.to_bytes();

	if bytes.is_empty() {
		return Err(InvalidCommandName::Empty);
	}

	if bytes.len() > MAX_NAME_LENGTH {
		return Err(InvalidCommandName::TooLong);
	}

	let mut index = 0;

	while index < bytes.len() {
		let byte = bytes[index];

		if !byte.is_ascii_alphanumeric() && byte != b'_' {
			return Err(InvalidCommandName::InvalidByte { index, byte });
		}

		index += 1;
	}

	let mut reserved = 0;

	while reserved < ENGINE_CLIENT_COMMANDS.len() {
		if bytes.eq_ignore_ascii_case(ENGINE_CLIENT_COMMANDS[reserved]) {
			return Err(InvalidCommandName::Reserved);
		}

		reserved += 1;
	}

	Ok(())
}
