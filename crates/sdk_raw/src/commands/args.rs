//! A validated copy of the arguments of one command invocation, tier1's
//! `CCommand`.

use std::ffi::{CStr, c_char, c_int};
use std::mem::offset_of;
use std::ptr::NonNull;

// The buffers a copy reads are as long as the limits say.
const _: () = {
	use sys::CCommand;

	assert!(
		offset_of!(CCommand, m_pArgvBuffer) - offset_of!(CCommand, m_pArgSBuffer) == MAX_LENGTH
	);
	assert!(offset_of!(CCommand, m_ppArgv) - offset_of!(CCommand, m_pArgvBuffer) == MAX_LENGTH);

	assert!(
		size_of::<CCommand>() - offset_of!(CCommand, m_ppArgv)
			== MAX_ARGC * size_of::<*const c_char>()
	);
};

/// `CCommand::COMMAND_MAX_ARGC`: the most arguments a command has, its name
/// included.
#[doc(alias = "COMMAND_MAX_ARGC")]
const MAX_ARGC: usize = sys::CCommand_COMMAND_MAX_ARGC as usize;

/// `CCommand::COMMAND_MAX_LENGTH`: the size of each of its buffers.
#[doc(alias = "COMMAND_MAX_LENGTH")]
const MAX_LENGTH: usize = sys::CCommand_COMMAND_MAX_LENGTH as usize;

/// A validated copy of one `CCommand`, which keeps its arguments however the
/// engine's own copy changes.
///
/// The engine re-tokenizes its command buffer in place, so a command executed
/// while another runs rewrites the engine's copy. The tail of each of its
/// buffers is also uninitialized, so only the bytes up to each terminator are
/// copied.
#[doc(alias = "CCommand")]
pub struct CommandLine {
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
	/// # Errors
	///
	/// If the argument count, the line's terminator, the start of the
	/// arguments, or the pointer to an argument is not one `CCommand::Tokenize`
	/// could have stored.
	///
	/// # Safety
	///
	/// `raw` must point to a `CCommand` that stays live and unmodified for the
	/// duration of this call, as the engine's `CCommand::Tokenize` leaves it.
	pub unsafe fn copy(raw: NonNull<sys::CCommand>) -> Result<Self, MalformedCommand> {
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

	/// The argument at `index`, counting the command name as 0, or `None` past
	/// the last.
	#[doc(alias = "Arg")]
	pub fn arg(&self, index: usize) -> Option<&CStr> {
		if index >= self.argc {
			return None;
		}

		CStr::from_bytes_until_nul(&self.args[usize::from(self.starts[index])..]).ok()
	}

	/// The number of arguments, including the command name: at least 1, and
	/// at most `CCommand::COMMAND_MAX_ARGC`.
	#[doc(alias = "ArgC")]
	pub const fn argc(&self) -> usize {
		self.argc
	}

	/// The whole command line.
	#[doc(alias = "GetCommandString")]
	pub fn line(&self) -> &CStr {
		CStr::from_bytes_until_nul(&self.line).unwrap_or_default()
	}

	/// Everything after the command name, exactly as typed, quotes included.
	#[doc(alias = "ArgS")]
	pub fn raw_args(&self) -> &CStr {
		CStr::from_bytes_until_nul(&self.line[self.args_offset..]).unwrap_or_default()
	}
}

/// A `CCommand` could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MalformedCommand {
	/// Its argument count, given, is outside 1 to `COMMAND_MAX_ARGC`.
	#[error("the command has {0} arguments, outside 1 to {MAX_ARGC}")]
	ArgCount(c_int),

	/// Its line is not terminated within `COMMAND_MAX_LENGTH` bytes.
	#[error("the command line is not terminated within {MAX_LENGTH} bytes")]
	Unterminated,

	/// The pointer to an argument does not lead to a terminated string in the
	/// argument buffer.
	#[error("argument {index} does not point into the argument buffer")]
	ArgOutsideBuffer {
		/// The argument's position, counting the command name as 0.
		index: usize,
	},

	/// Its `m_nArgv0Size`, given, lies beyond the end of the line.
	#[error("the arguments start at byte {0}, beyond the end of the line")]
	ArgsOffset(c_int),
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

/// Builds a command from arguments, as `CCommand::Tokenize` would store them,
/// for tests. `args_start` is `m_nArgv0Size`: 0 without arguments, otherwise
/// the offset of the second token, at its opening quote if it has one.
///
/// # Panics
///
/// If there are more than `COMMAND_MAX_ARGC` arguments.
#[cfg(any(test, feature = "test-support"))]
pub fn tokenized(line: &str, args: &[&str], args_start: usize) -> Box<sys::CCommand> {
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn copies_are_indexed_from_the_name() {
		let raw = tokenized(
			"sb_give scout \"1 2\" 3",
			&["sb_give", "scout", "1 2", "3"],
			8,
		);
		let line = copy(&raw).unwrap();

		assert_eq!(line.argc(), 4);
		assert_eq!(line.arg(0), Some(c"sb_give"));
		assert_eq!(line.arg(2), Some(c"1 2"));
		assert_eq!(line.arg(4), None);
		assert_eq!(line.line(), c"sb_give scout \"1 2\" 3");
		assert_eq!(line.raw_args(), c"scout \"1 2\" 3");
	}

	/// Copies a command the tests built, which stays unchanged for the call.
	fn copy(raw: &sys::CCommand) -> Result<CommandLine, MalformedCommand> {
		// SAFETY: `raw` is live and unmodified for the call.
		unsafe { CommandLine::copy(NonNull::from(raw)) }
	}

	#[test]
	fn malformed_commands_are_refused() {
		let mut raw = tokenized("a", &["a"], 0);

		raw.m_nArgc = 0;
		assert_eq!(copy(&raw).err(), Some(MalformedCommand::ArgCount(0)));
		assert_eq!(
			MalformedCommand::ArgCount(0).to_string(),
			"the command has 0 arguments, outside 1 to 64"
		);

		raw.m_nArgc = 1;
		raw.m_nArgv0Size = 2;
		assert_eq!(copy(&raw).err(), Some(MalformedCommand::ArgsOffset(2)));

		raw.m_nArgv0Size = 1;
		raw.m_ppArgv[0] = c"elsewhere".as_ptr();
		assert_eq!(
			copy(&raw).err(),
			Some(MalformedCommand::ArgOutsideBuffer { index: 0 })
		);

		raw.m_pArgSBuffer.fill(b'x' as c_char);
		assert_eq!(copy(&raw).err(), Some(MalformedCommand::Unterminated));
	}

	#[test]
	fn raw_arguments_match_args() {
		for line in ["sb_say", "sb_say   "] {
			let raw = tokenized(line, &["sb_say"], 0);
			let line = copy(&raw).unwrap();

			assert_eq!(line.argc(), 1);
			assert_eq!(line.raw_args(), c"");
		}

		let raw = tokenized(
			"sb_echo \"This is cryotheum\"",
			&["sb_echo", "This is cryotheum"],
			8,
		);
		let line = copy(&raw).unwrap();

		assert_eq!(line.arg(1), Some(c"This is cryotheum"));
		assert_eq!(line.raw_args(), c"\"This is cryotheum\"");
	}
}
