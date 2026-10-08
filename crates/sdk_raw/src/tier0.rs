//! tier0's exports, found at runtime so that no Source library is linked, and
//! values from its headers.
//!
//! tier0 is the engine's base library, which every other Source module links
//! against. It is loaded before them and unloaded after them, so inside a
//! Source process an export found while it is loaded stays callable for as
//! long as any module that uses this crate is.

use crate::util::loaded_symbol;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::sync::OnceLock;

/// `Msg` from `public/tier0/dbg.h`, which tier0 exports with C linkage, and
/// which prints a `printf`-style format and its arguments to the console.
#[doc(alias("Msg"))]
pub type MsgFn = unsafe extern "C" fn(format: *const c_char, ...);

/// A spew function, tier0's `SpewOutputFunc_t`: receives each line of console
/// output, already formatted, and tells tier0 what to do next.
#[doc(alias("SpewOutputFunc_t"))]
pub type SpewOutputFn = unsafe extern "C" fn(kind: SpewType, message: *const c_char) -> SpewRetval;

/// The names tier0 has: on Windows, and in 64-bit and older Linux dedicated
/// servers.
const LIBRARIES: &[&CStr] = cfg_select! {
	windows => &[c"tier0.dll"],
	target_os = "linux" => &[c"libtier0.so", c"libtier0_srv.so"],
};

/// The size of a buffer for a file path, terminator included.
///
/// This is `MAX_PATH` from `public/tier0/platform.h`.
pub const MAX_PATH: usize = 260;

/// tier0's functions for the spew function, from `public/tier0/dbg.h`, which
/// tier0 exports with C linkage.
///
/// tier0 calls a single spew function, held in one global pointer, for every
/// line of console output, from whichever thread printed it. It reads and
/// writes the pointer without a lock.
#[derive(Debug, Clone, Copy)]
pub struct SpewApi {
	/// `SpewOutputFunc`: makes a function the spew function. Null makes it
	/// `DefaultSpewFunc` again.
	#[doc(alias("SpewOutputFunc"))]
	pub set_output: unsafe extern "C" fn(function: Option<SpewOutputFn>),

	/// `GetSpewOutputFunc`: the current spew function.
	#[doc(alias("GetSpewOutputFunc"))]
	pub output: unsafe extern "C" fn() -> Option<SpewOutputFn>,

	/// `DefaultSpewFunc`: tier0's own spew function, which prints to standard
	/// output.
	#[doc(alias("DefaultSpewFunc"))]
	pub default: SpewOutputFn,

	/// `GetSpewOutputGroup`: the group of the line being spewed, such as
	/// `"developer"` or `"console"`, or an empty or null string. Only valid
	/// inside a spew function, for its line.
	#[doc(alias("GetSpewOutputGroup"))]
	pub group: unsafe extern "C" fn() -> *const c_char,

	/// `GetSpewOutputLevel`: the level of the line being spewed within its
	/// group. Only valid inside a spew function, for its line.
	#[doc(alias("GetSpewOutputLevel"))]
	pub level: unsafe extern "C" fn() -> c_int,

	/// `GetSpewOutputColor`: the colour of the line being spewed, or null.
	/// Only valid inside a spew function, for its line.
	#[doc(alias("GetSpewOutputColor"))]
	pub color: unsafe extern "C" fn() -> *const SpewColor,
}

/// A colour as tier0 reports it for a line of output: the engine's `Color`,
/// red, green, blue and alpha.
#[doc(alias("Color"))]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SpewColor {
	/// Red.
	pub r: u8,

	/// Green.
	pub g: u8,

	/// Blue.
	pub b: u8,

	/// Alpha.
	pub a: u8,
}

/// What tier0 does after a spew function returns, tier0's `SpewRetval_t`.
#[doc(alias("SpewRetval_t"))]
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpewRetval(pub c_int);

impl SpewRetval {
	/// `SPEW_ABORT`: exit the process.
	pub const ABORT: Self = Self(2);

	/// `SPEW_CONTINUE`: carry on.
	pub const CONTINUE: Self = Self(1);

	/// `SPEW_DEBUGGER`: break into the debugger.
	pub const DEBUGGER: Self = Self(0);
}

/// The kind of a line of console output, tier0's `SpewType_t`.
///
/// tier0 passes it by value, as the `int` of a C enum.
#[doc(alias("SpewType_t"))]
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpewType(pub c_int);

impl SpewType {
	/// `SPEW_ASSERT`: a failed assertion.
	pub const ASSERT: Self = Self(2);

	/// `SPEW_ERROR`: a fatal error, such as `Error`'s, after which the process
	/// exits.
	pub const ERROR: Self = Self(3);

	/// `SPEW_LOG`: a line for the log, such as `Log`'s and `ConLog`'s.
	pub const LOG: Self = Self(4);

	/// `SPEW_MESSAGE`: a message, such as `Msg`'s, `ConMsg`'s and `DevMsg`'s.
	pub const MESSAGE: Self = Self(0);

	/// `SPEW_WARNING`: a warning, such as `Warning`'s and `DevWarning`'s.
	pub const WARNING: Self = Self(1);
}

/// Looks up `Msg` in the tier0 library the process has already loaded.
/// Returns `None` if tier0 is not loaded or does not export it, and under Miri.
fn find_msg() -> Option<MsgFn> {
	// Miri cannot call the platform's loader.
	if cfg!(miri) {
		return None;
	}

	let address = LIBRARIES
		.iter()
		.find_map(|library| loaded_symbol(library, c"Msg"))?;

	// SAFETY: tier0 exports `Msg` with this signature.
	Some(unsafe { std::mem::transmute::<*mut c_void, MsgFn>(address.as_ptr()) })
}

/// Looks up the spew functions in the tier0 library the process has already
/// loaded. Returns `None` if tier0 is not loaded or lacks any of them, and
/// under Miri.
fn find_spew() -> Option<SpewApi> {
	if cfg!(miri) {
		return None;
	}

	let find = |name: &CStr| {
		LIBRARIES
			.iter()
			.find_map(|library| loaded_symbol(library, name))
			.map(|address| address.as_ptr())
	};

	let set_output = find(c"SpewOutputFunc")?;
	let output = find(c"GetSpewOutputFunc")?;
	let default = find(c"DefaultSpewFunc")?;
	let group = find(c"GetSpewOutputGroup")?;
	let level = find(c"GetSpewOutputLevel")?;
	let color = find(c"GetSpewOutputColor")?;

	// SAFETY: tier0 exports each function with the signature its field has,
	// from `public/tier0/dbg.h`.
	unsafe {
		Some(SpewApi {
			set_output: std::mem::transmute::<
				*mut c_void,
				unsafe extern "C" fn(Option<SpewOutputFn>),
			>(set_output),
			output: std::mem::transmute::<
				*mut c_void,
				unsafe extern "C" fn() -> Option<SpewOutputFn>,
			>(output),
			default: std::mem::transmute::<*mut c_void, SpewOutputFn>(default),
			group: std::mem::transmute::<*mut c_void, unsafe extern "C" fn() -> *const c_char>(
				group,
			),
			level: std::mem::transmute::<*mut c_void, unsafe extern "C" fn() -> c_int>(level),
			color: std::mem::transmute::<*mut c_void, unsafe extern "C" fn() -> *const SpewColor>(
				color,
			),
		})
	}
}

/// tier0's `Msg`, or `None` if tier0 is not loaded or does not export it, and
/// under Miri.
///
/// The process's tier0 is looked up on the first call, and its result kept.
#[doc(alias("Msg"))]
pub fn msg() -> Option<MsgFn> {
	static MSG: OnceLock<Option<MsgFn>> = OnceLock::new();

	*MSG.get_or_init(find_msg)
}

/// Prints `message` through tier0's `Msg`, whose output the dedicated server's
/// console and rcon's redirection both receive. Returns `false` if tier0 is
/// not loaded.
///
/// The message is printed as is, never interpreted as a format.
///
/// # Safety
///
/// Any loaded tier0 library must be the Source engine's, which stays loaded
/// for the call, and the call must be made on a thread where tier0's console
/// output may run, such as the server's main thread.
pub unsafe fn print(message: &CStr) -> bool {
	let Some(msg) = msg() else {
		return false;
	};

	// SAFETY: `msg` is tier0's `Msg`, which formats as `printf` does, and the
	// caller keeps tier0 loaded and calls on a thread where it may print.
	unsafe { print_through(msg, message) };
	true
}

/// Prints `message` through `msg`, a `printf`-style function such as tier0's
/// `Msg`, as the argument of a constant `"%s"` format, so the message is never
/// interpreted as one.
///
/// # Safety
///
/// `msg` must be callable, and read its format and arguments as `printf`
/// does.
pub unsafe fn print_through(msg: MsgFn, message: &CStr) {
	// SAFETY: As the caller promises; the format consumes exactly the one
	// string argument passed.
	unsafe { msg(c"%s".as_ptr(), message.as_ptr()) };
}

/// tier0's spew functions, or `None` if tier0 is not loaded or lacks any of
/// them, and under Miri.
///
/// The process's tier0 is looked up on the first call, and its result kept.
pub fn spew_api() -> Option<SpewApi> {
	static SPEW: OnceLock<Option<SpewApi>> = OnceLock::new();

	*SPEW.get_or_init(find_spew)
}
