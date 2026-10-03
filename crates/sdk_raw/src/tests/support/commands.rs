//! Builders of the console commands the engine passes to command objects.

use std::ffi::{c_char, c_int};

/// Builds a command from arguments, as `CCommand::Tokenize` would store them,
/// for tests. `args_start` is `m_nArgv0Size`: 0 without arguments, otherwise
/// the offset of the second token, at its opening quote if it has one.
///
/// # Panics
///
/// If there are more than `COMMAND_MAX_ARGC` arguments.
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
