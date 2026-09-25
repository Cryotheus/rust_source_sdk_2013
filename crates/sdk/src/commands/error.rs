//! Registration errors and command name validation.

use crate::server::InterfaceError;
use std::ffi::CStr;
use std::fmt::{self, Display, Formatter};

/// The longest command name [`validate_name`] accepts, in bytes.
const MAX_NAME_LENGTH: usize = 63;

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

/// A command name [`validate_name`] rejects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InvalidCommandName {
	#[error("the name is empty")]
	Empty,

	#[error("the name is longer than {MAX_NAME_LENGTH} bytes")]
	TooLong,

	#[error("byte {index} ({byte:#04x}) is not an ASCII letter, digit, or `_`")]
	InvalidByte { index: usize, byte: u8 },

	/// The engine runs commands with this name for clients as though the
	/// server ran them, so their invoker could not be told apart.
	#[error("the engine reserves the name for its own client commands")]
	Reserved,
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

/// What already uses a name in the engine's registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandBaseKind {
	Command,
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

/// A command could not be registered.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("could not register console command `{}`: {kind}", .name.to_string_lossy())]
pub struct RegisterCommandError {
	name: &'static CStr,
	kind: RegisterCommandErrorKind,
}

impl RegisterCommandError {
	pub(crate) const fn new(name: &'static CStr, kind: RegisterCommandErrorKind) -> Self {
		Self { name, kind }
	}

	pub const fn name(&self) -> &'static CStr {
		self.name
	}

	pub const fn kind(&self) -> &RegisterCommandErrorKind {
		&self.kind
	}
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegisterCommandErrorKind {
	/// The engine lists the command already. Registering it again would cut
	/// the engine's list short, since the command is relinked in place.
	#[error("the command is already registered")]
	AlreadyRegistered,

	#[error("the name is already used by a console {0}")]
	NameTaken(CommandBaseKind),

	/// The registrar returned without the engine listing the command under its
	/// name. It was unlinked again if the engine marked it registered.
	#[error("the engine did not list the command")]
	NotLinked,

	#[error(transparent)]
	Interface(#[from] InterfaceError),
}

/// A command could not be unregistered.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("could not unregister console command `{}`: {kind}", .name.to_string_lossy())]
pub struct UnregisterCommandError {
	name: &'static CStr,
	kind: UnregisterCommandErrorKind,
}

impl UnregisterCommandError {
	pub(crate) const fn new(name: &'static CStr, kind: UnregisterCommandErrorKind) -> Self {
		Self { name, kind }
	}

	pub const fn name(&self) -> &'static CStr {
		self.name
	}

	pub const fn kind(&self) -> &UnregisterCommandErrorKind {
		&self.kind
	}
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UnregisterCommandErrorKind {
	#[error("the command is not registered")]
	NotRegistered,

	/// The registrar returned with the engine still listing the command.
	#[error("the engine still lists the command")]
	StillLinked,

	#[error(transparent)]
	Interface(#[from] InterfaceError),
}
