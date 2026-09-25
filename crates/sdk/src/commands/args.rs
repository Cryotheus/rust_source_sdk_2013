//! The arguments of one command invocation (`CCommand`).

use std::ffi::{CStr, c_char, c_int};
use std::mem::offset_of;
use std::ptr::NonNull;
use std::str::FromStr;

/// `CCommand::COMMAND_MAX_ARGC`.
const MAX_ARGC: usize = 64;

/// `CCommand::COMMAND_MAX_LENGTH`.
const MAX_LENGTH: usize = 512;

/// A `CCommand` the engine passed could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MalformedCommand {
	#[error("the command has {0} arguments, outside 1 to {MAX_ARGC}")]
	ArgCount(c_int),

	#[error("the command line is not terminated within {MAX_LENGTH} bytes")]
	Unterminated,

	#[error("argument {index} does not point into the argument buffer")]
	ArgOutsideBuffer { index: usize },

	#[error("the arguments start at byte {0}, beyond the end of the line")]
	ArgsOffset(c_int),
}

/// A validated copy of one `CCommand`, owned by a single dispatch.
///
/// The engine re-tokenizes its command buffer in place, so a command executed
/// while a handler runs would rewrite the engine's copy. The tail of each of
/// its buffers is also uninitialized, so only the bytes up to each terminator
/// are copied.
pub(crate) struct CommandLine {
	/// `m_pArgSBuffer` up to and including its terminator: the whole line.
	line: [u8; MAX_LENGTH],

	/// Each argument, terminated, packed from the start.
	args: [u8; MAX_LENGTH],

	/// Where each argument starts in `args`.
	starts: [u16; MAX_ARGC],

	/// The number of arguments, including the command name: 1 to 64.
	argc: usize,

	/// `m_nArgv0Size`: where the arguments after the name start in `line`, or
	/// the end of the line if there are none.
	args_offset: usize,
}

impl CommandLine {
	/// Copies and validates a `CCommand`.
	///
	/// # Safety
	///
	/// `raw` must point to a `CCommand` that stays live and unmodified for the
	/// duration of this call, as the engine's `CCommand::Tokenize` leaves it.
	pub(crate) unsafe fn copy(raw: NonNull<sys::CCommand>) -> Result<Self, MalformedCommand> {
		let raw = raw.as_ptr();

		// SAFETY: The caller guarantees `raw` is live. Fields are read without
		// forming references, since only the tokenized prefix is initialized.
		let (argc, argv0_size) = unsafe {
			(
				(&raw const (*raw).m_nArgc).read(),
				(&raw const (*raw).m_nArgv0Size).read(),
			)
		};

		let argc_usize = usize::try_from(argc)
			.ok()
			.filter(|argc| (1..=MAX_ARGC).contains(argc))
			.ok_or(MalformedCommand::ArgCount(argc))?;

		let mut copy = Self {
			line: [0; MAX_LENGTH],
			args: [0; MAX_LENGTH],
			starts: [0; MAX_ARGC],
			argc: argc_usize,
			args_offset: 0,
		};

		// SAFETY: `m_pArgSBuffer` is part of the live `CCommand`.
		let line = unsafe {
			raw.cast::<u8>()
				.add(offset_of!(sys::CCommand, m_pArgSBuffer))
		};

		// SAFETY: The buffer holds `MAX_LENGTH` bytes, read up to the first NUL.
		let line_length = unsafe { copy_terminated(line, MAX_LENGTH, &mut copy.line) }
			.ok_or(MalformedCommand::Unterminated)?;

		// `Tokenize` only records where the arguments start once it reaches one,
		// and `ArgS` reads 0 as there being none.
		copy.args_offset = match argv0_size {
			0 => line_length,
			offset => usize::try_from(offset)
				.ok()
				.filter(|&offset| offset <= line_length)
				.ok_or(MalformedCommand::ArgsOffset(offset))?,
		};

		// SAFETY: As for `line`.
		let argv_buffer = unsafe {
			raw.cast::<u8>()
				.add(offset_of!(sys::CCommand, m_pArgvBuffer))
		};
		let mut packed = 0;

		for index in 0..argc_usize {
			// SAFETY: The tokenizer wrote the first `argc` pointers.
			let arg = unsafe {
				(&raw const (*raw).m_ppArgv)
					.cast::<*const c_char>()
					.add(index)
					.read()
			};

			// Only the address is used: the bytes are read through `raw`, which
			// has provenance over the whole command.
			let offset = arg
				.addr()
				.checked_sub(argv_buffer.addr())
				.filter(|&offset| offset < MAX_LENGTH)
				.ok_or(MalformedCommand::ArgOutsideBuffer { index })?;

			// SAFETY: `offset` lies within the argument buffer, read up to the first
			// NUL before its end.
			let length = unsafe {
				copy_terminated(
					argv_buffer.add(offset),
					MAX_LENGTH - offset,
					&mut copy.args[packed..],
				)
			}
			.ok_or(MalformedCommand::ArgOutsideBuffer { index })?;

			// The packed arguments never exceed the buffer they were packed in.
			copy.starts[index] =
				u16::try_from(packed).map_err(|_| MalformedCommand::ArgOutsideBuffer { index })?;
			packed += length + 1;
		}

		Ok(copy)
	}

	fn arg(&self, index: usize) -> Option<&CStr> {
		if index >= self.argc {
			return None;
		}

		CStr::from_bytes_until_nul(&self.args[usize::from(self.starts[index])..]).ok()
	}

	fn line(&self) -> &CStr {
		CStr::from_bytes_until_nul(&self.line).unwrap_or_default()
	}

	fn raw_args(&self) -> &CStr {
		CStr::from_bytes_until_nul(&self.line[self.args_offset..]).unwrap_or_default()
	}

	/// Builds a command from arguments, as `CCommand::Tokenize` would store
	/// them. `args_start` is `m_nArgv0Size`: 0 without arguments, otherwise
	/// the offset of the second token, at its opening quote if it has one.
	#[cfg(test)]
	pub(crate) fn tokenized(line: &str, args: &[&str], args_start: usize) -> Box<sys::CCommand> {
		// SAFETY: `CCommand` is plain data, for which zeroes are valid.
		let mut command = Box::new(unsafe { std::mem::zeroed::<sys::CCommand>() });
		let mut packed = 0;

		for (slot, byte) in command.m_pArgSBuffer.iter_mut().zip(line.bytes()) {
			*slot = byte as c_char;
		}

		command.m_nArgc = args.len() as c_int;
		command.m_nArgv0Size = args_start as c_int;

		for (index, arg) in args.iter().enumerate() {
			for (slot, byte) in command.m_pArgvBuffer[packed..].iter_mut().zip(arg.bytes()) {
				*slot = byte as c_char;
			}

			command.m_ppArgv[index] = (&raw const command.m_pArgvBuffer[packed]).cast();
			packed += arg.len() + 1;
		}

		command
	}
}

/// Copies bytes from `source` into `destination` up to and including the
/// first NUL, returning the length before it, or `None` if no NUL comes within
/// `limit` bytes or `destination` is too short.
///
/// # Safety
///
/// `source` must be readable for the bytes up to its first NUL, or `limit`
/// bytes, whichever comes first.
unsafe fn copy_terminated(
	source: *const u8,
	limit: usize,
	destination: &mut [u8],
) -> Option<usize> {
	for (index, slot) in destination.iter_mut().take(limit).enumerate() {
		// SAFETY: Every earlier byte was not NUL, and the caller allows reading up
		// to `limit` bytes.
		let byte = unsafe { source.add(index).read() };
		*slot = byte;

		if byte == 0 {
			return Some(index);
		}
	}

	None
}

/// The arguments of one invocation.
///
/// Argument 0, the command name, is [`Self::name`]; [`Self::get`] counts the
/// arguments after it from 0.
#[derive(Clone, Copy)]
pub struct CommandArgs<'d> {
	line: &'d CommandLine,
}

impl<'d> CommandArgs<'d> {
	pub(crate) const fn new(line: &'d CommandLine) -> Self {
		Self { line }
	}

	/// The command name as typed. Its case may differ from the registered name.
	pub fn name(self) -> &'d CStr {
		self.line.arg(0).unwrap_or_default()
	}

	/// The number of arguments after the name.
	#[doc(alias = "ArgC")]
	pub fn len(self) -> usize {
		self.line.argc - 1
	}

	pub fn is_empty(self) -> bool {
		self.len() == 0
	}

	/// The argument at `index` after the name.
	#[doc(alias = "Arg")]
	pub fn get(self, index: usize) -> Option<&'d CStr> {
		self.line.arg(index.checked_add(1)?)
	}

	/// The arguments after the name.
	pub fn iter(self) -> impl Iterator<Item = &'d CStr> + 'd {
		(0..self.len()).filter_map(move |index| self.get(index))
	}

	/// Everything after the name, exactly as typed, quotes included.
	#[doc(alias = "ArgS")]
	pub fn raw_args(self) -> &'d CStr {
		self.line.raw_args()
	}

	/// The whole command line.
	#[doc(alias = "GetCommandString")]
	pub fn command_line(self) -> &'d CStr {
		self.line.line()
	}

	/// The argument at `index` after the name, as UTF-8.
	pub fn get_str(self, index: usize) -> Result<&'d str, ArgError> {
		let position = index + 1;

		self.get(index)
			.ok_or(ArgError::Missing { position })?
			.to_str()
			.map_err(|_| ArgError::NotUtf8 { position })
	}

	/// Parses the argument at `index` after the name.
	pub fn parse<T: FromStr>(self, index: usize) -> Result<T, ArgError> {
		let value = self.get_str(index)?;

		value.parse().map_err(|_| ArgError::Invalid {
			position: index + 1,
			value: value.to_owned(),
			expected: std::any::type_name::<T>(),
		})
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

/// An argument is missing or cannot be read as requested.
///
/// Positions count the command name as 0, as the console shows them.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ArgError {
	#[error("missing argument {position}")]
	Missing { position: usize },

	#[error("argument {position} is not valid UTF-8")]
	NotUtf8 { position: usize },

	#[error("argument {position} (`{value}`) is not a valid {expected}")]
	Invalid {
		position: usize,
		value: String,
		expected: &'static str,
	},
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn copies_are_validated_and_indexed() {
		let raw = CommandLine::tokenized(
			"sb_give scout \"1 2\" 3",
			&["sb_give", "scout", "1 2", "3"],
			8,
		);
		let line = unsafe { CommandLine::copy(NonNull::from(&*raw)) }.unwrap();
		let args = CommandArgs::new(&line);

		assert_eq!(args.name(), c"sb_give");
		assert_eq!(args.len(), 3);
		assert_eq!(args.get(0), Some(c"scout"));
		assert_eq!(args.get(1), Some(c"1 2"));
		assert_eq!(args.get(3), None);
		assert_eq!(args.iter().collect::<Vec<_>>(), [c"scout", c"1 2", c"3"]);
		assert_eq!(args.raw_args(), c"scout \"1 2\" 3");
		assert_eq!(args.command_line(), c"sb_give scout \"1 2\" 3");
		assert_eq!(args.parse::<u8>(2), Ok(3));
		assert_eq!(args.parse::<u8>(3), Err(ArgError::Missing { position: 4 }));
		assert_eq!(
			args.parse::<u8>(0).unwrap_err().to_string(),
			"argument 1 (`scout`) is not a valid u8"
		);
	}

	#[test]
	fn raw_arguments_match_args() {
		for line in ["sb_say", "sb_say   "] {
			let raw = CommandLine::tokenized(line, &["sb_say"], 0);
			let line = unsafe { CommandLine::copy(NonNull::from(&*raw)) }.unwrap();
			let args = CommandArgs::new(&line);

			assert!(args.is_empty());
			assert_eq!(args.raw_args(), c"");
		}

		let raw = CommandLine::tokenized(
			"sb_echo \"This is cryotheum\"",
			&["sb_echo", "This is cryotheum"],
			8,
		);
		let line = unsafe { CommandLine::copy(NonNull::from(&*raw)) }.unwrap();
		let args = CommandArgs::new(&line);

		assert_eq!(args.get(0), Some(c"This is cryotheum"));
		assert_eq!(args.raw_args(), c"\"This is cryotheum\"");
	}

	#[test]
	fn malformed_commands_are_refused() {
		let mut raw = CommandLine::tokenized("a", &["a"], 0);

		raw.m_nArgc = 0;
		assert_eq!(
			unsafe { CommandLine::copy(NonNull::from(&*raw)) }.err(),
			Some(MalformedCommand::ArgCount(0))
		);

		raw.m_nArgc = 1;
		raw.m_nArgv0Size = 2;
		assert_eq!(
			unsafe { CommandLine::copy(NonNull::from(&*raw)) }.err(),
			Some(MalformedCommand::ArgsOffset(2))
		);

		raw.m_nArgv0Size = 1;
		raw.m_ppArgv[0] = c"elsewhere".as_ptr();
		assert_eq!(
			unsafe { CommandLine::copy(NonNull::from(&*raw)) }.err(),
			Some(MalformedCommand::ArgOutsideBuffer { index: 0 })
		);

		raw.m_pArgSBuffer.fill(b'x' as c_char);
		assert_eq!(
			unsafe { CommandLine::copy(NonNull::from(&*raw)) }.err(),
			Some(MalformedCommand::Unterminated)
		);
	}
}
