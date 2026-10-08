//! The arguments of one command invocation (`CCommand`).

use sdk_raw::commands::CommandLine;
use std::ffi::CStr;
use std::str::FromStr;

/// An argument is missing or cannot be read as requested.
///
/// Positions count the command name as 0, as the console shows them.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ArgError {
	/// The invocation has no argument at the requested position.
	#[error("missing argument {position}")]
	Missing {
		/// The requested argument's position.
		position: usize,
	},

	/// The argument is not valid UTF-8.
	#[error("argument {position} is not valid UTF-8")]
	NotUtf8 {
		/// The argument's position.
		position: usize,
	},

	/// The argument does not parse as the requested type.
	#[error("argument {position} (`{value}`) is not a valid {expected}")]
	Invalid {
		/// The argument's position.
		position: usize,

		/// The argument's text, as tokenized, without quotes.
		value: String,

		/// The requested type's name, as [`std::any::type_name`] gives it.
		expected: &'static str,
	},
}

/// The arguments of one invocation.
///
/// Argument 0, the command name, is [`Self::name`]; [`Self::get`] counts the
/// arguments after it from 0.
#[doc(alias("CCommand"))]
#[derive(Clone, Copy)]
pub struct CommandArgs<'d> {
	/// A copy owned by a single dispatch, so commands run in the meantime
	/// cannot change it.
	line: &'d CommandLine,
}

impl<'d> CommandArgs<'d> {
	pub(crate) const fn new(line: &'d CommandLine) -> Self {
		Self { line }
	}

	/// The whole command line.
	#[doc(alias("GetCommandString"))]
	pub fn command_line(self) -> &'d CStr {
		self.line.line()
	}

	/// The argument at `index` after the name, or `None` past the last.
	#[doc(alias("Arg"))]
	pub fn get(self, index: usize) -> Option<&'d CStr> {
		self.line.arg(index.checked_add(1)?)
	}

	/// The argument at `index` after the name, as UTF-8. Fails if it is
	/// missing or not UTF-8.
	pub fn get_str(self, index: usize) -> Result<&'d str, ArgError> {
		let position = index.saturating_add(1);

		self.get(index)
			.ok_or(ArgError::Missing { position })?
			.to_str()
			.map_err(|_| ArgError::NotUtf8 { position })
	}

	/// Whether no arguments follow the name.
	pub fn is_empty(self) -> bool {
		self.len() == 0
	}

	/// The arguments after the name.
	pub fn iter(self) -> impl Iterator<Item = &'d CStr> + 'd {
		(0..self.len()).filter_map(move |index| self.get(index))
	}

	/// The number of arguments after the name.
	#[doc(alias("ArgC"))]
	pub fn len(self) -> usize {
		// A copy always holds the name.
		self.line.argc() - 1
	}

	/// The command name as typed. Its case may differ from the registered name.
	pub fn name(self) -> &'d CStr {
		self.line.arg(0).unwrap_or_default()
	}

	/// Parses the argument at `index` after the name. Fails as
	/// [`Self::get_str`] does, or if [`FromStr`] rejects the argument.
	pub fn parse<T: FromStr>(self, index: usize) -> Result<T, ArgError> {
		let value = self.get_str(index)?;

		value.parse().map_err(|_| ArgError::Invalid {
			position: index + 1,
			value: value.to_owned(),
			expected: std::any::type_name::<T>(),
		})
	}

	/// Everything after the name, exactly as typed, quotes included.
	#[doc(alias("ArgS"))]
	pub fn raw_args(self) -> &'d CStr {
		self.line.raw_args()
	}
}

impl std::fmt::Debug for CommandArgs<'_> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("CommandArgs")
			.field("name", &self.name())
			.field("args", &self.iter().collect::<Vec<_>>())
			.finish()
	}
}
