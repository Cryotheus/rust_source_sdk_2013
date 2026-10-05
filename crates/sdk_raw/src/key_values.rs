//! tier1's `KeyValues`, as far as reading the name of key values the engine
//! made, and vstdlib's `IKeyValuesSystem`, whose symbol table holds the names.
//!
//! Key values name themselves by a symbol, which `KeyValues::GetName` looks up
//! in the table of the `IKeyValuesSystem` that vstdlib's `KeyValuesSystem`
//! returns. Every module that links tier1 shares that table, unless it moved
//! its own key values to a table of their own
//! (`KeyValues::SetUseGrowableStringTable`), which neither the SDK's game code
//! nor its tier1 does by default.

#[cfg(test)]
#[path = "tests/key_values.rs"]
mod tests;

use crate::util::loaded_symbol;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::mem::offset_of;
use std::ptr::NonNull;
use std::sync::OnceLock;

/// `IKeyValuesSystem::GetStringForSymbol`: the name `symbol` stands for.
#[doc(alias("GetStringForSymbol"))]
pub type GetStringForSymbolFn =
	unsafe extern "C" fn(this: *mut IKeyValuesSystem, symbol: HKeySymbol) -> *const c_char;

/// `IKeyValuesSystem::GetSymbolForString`: the symbol of `name`, ignoring
/// case, which is created if `create` is set and it has none yet, or `-1`.
#[doc(alias("GetSymbolForString"))]
pub type GetSymbolForStringFn = unsafe extern "C" fn(
	this: *mut IKeyValuesSystem,
	name: *const c_char,
	create: bool,
) -> HKeySymbol;

/// `HKeySymbol`, from `public/vstdlib/IKeyValuesSystem.h`: a name's place in
/// the key values system's symbol table.
pub type HKeySymbol = c_int;

/// `KeyValuesSystem`, which vstdlib exports with C linkage
/// (`VSTDLIB_INTERFACE`), and which returns the process's key values system.
#[doc(alias("KeyValuesSystem"))]
pub type KeyValuesSystemFn = unsafe extern "C" fn() -> *mut IKeyValuesSystem;

// `IKeyValuesSystem` declares no destructor, so its slots are the same on both
// ABIs, in the order `public/vstdlib/IKeyValuesSystem.h` declares them.
// `KeyValues` declares no virtual function: `m_iKeyName` is its first field,
// at offset 0, of the 64 bytes the generated bindings assert on both ABIs.
const _: () = assert!(
	GET_SYMBOL_FOR_STRING_SLOT == 3
		&& GET_STRING_FOR_SYMBOL_SLOT == 4
		&& size_of::<sys::KeyValues>() == 64
);

/// `IKeyValuesSystem::GetStringForSymbol`'s slot.
#[doc(alias("GetStringForSymbol"))]
pub const GET_STRING_FOR_SYMBOL_SLOT: usize =
	offset_of!(IKeyValuesSystemVtable, get_string_for_symbol) / size_of::<usize>();

/// `IKeyValuesSystem::GetSymbolForString`'s slot.
#[doc(alias("GetSymbolForString"))]
pub const GET_SYMBOL_FOR_STRING_SLOT: usize =
	offset_of!(IKeyValuesSystemVtable, get_symbol_for_string) / size_of::<usize>();

/// The names vstdlib has: on Windows, and in 64-bit and older Linux dedicated
/// servers.
const LIBRARIES: &[&CStr] = cfg_select! {
	windows => &[c"vstdlib.dll"],
	target_os = "linux" => &[c"libvstdlib.so", c"libvstdlib_srv.so"],
};

/// `IKeyValuesSystem`, the process's store of key values' names and memory,
/// from `public/vstdlib/IKeyValuesSystem.h`.
#[repr(C)]
pub struct IKeyValuesSystem {
	/// The pointer to the object's vtable.
	pub vtable_: *const IKeyValuesSystemVtable,
}

/// The leading entries of `IKeyValuesSystem`'s vtable, through
/// `GetStringForSymbol`. Those after it are not declared.
#[repr(C)]
pub struct IKeyValuesSystemVtable {
	/// `RegisterSizeofKeyValues`.
	pub register_sizeof_key_values: *const c_void,

	/// `AllocKeyValuesMemory`.
	pub alloc_key_values_memory: *const c_void,

	/// `FreeKeyValuesMemory`.
	pub free_key_values_memory: *const c_void,

	/// `GetSymbolForString`.
	pub get_symbol_for_string: GetSymbolForStringFn,

	/// `GetStringForSymbol`.
	pub get_string_for_symbol: GetStringForSymbolFn,
}

/// Looks up `KeyValuesSystem` in the vstdlib library the process has already
/// loaded. Returns `None` if vstdlib is not loaded or does not export it, and
/// under Miri.
fn find_key_values_system() -> Option<KeyValuesSystemFn> {
	// Miri cannot call the platform's loader.
	if cfg!(miri) {
		return None;
	}

	let address = LIBRARIES
		.iter()
		.find_map(|library| loaded_symbol(library, c"KeyValuesSystem"))?;

	// SAFETY: vstdlib exports `KeyValuesSystem` with this signature.
	Some(unsafe { std::mem::transmute::<*mut c_void, KeyValuesSystemFn>(address.as_ptr()) })
}

/// The name of `key_values`: the string its symbol stands for in `system`'s
/// table, which lives as long as `system` does.
///
/// # Safety
///
/// `system` must be live, and `key_values` must point to live key values
/// named by a symbol of `system`'s table, such as those the engine makes.
#[doc(alias("GetName"))]
pub unsafe fn key_name(
	system: NonNull<IKeyValuesSystem>,
	key_values: NonNull<sys::KeyValues>,
) -> *const c_char {
	// SAFETY: The caller passes live key values, whose first field is
	// `m_iKeyName`.
	let symbol = unsafe { key_values.cast::<HKeySymbol>().read() };

	// SAFETY: The caller passes a live system, whose vtable has
	// `GetStringForSymbol` at its slot, and a symbol of its table.
	unsafe { ((*(*system.as_ptr()).vtable_).get_string_for_symbol)(system.as_ptr(), symbol) }
}

/// The process's key values system, which vstdlib keeps for as long as the
/// process runs, or `None` if vstdlib is not loaded or does not export
/// `KeyValuesSystem`, and under Miri.
///
/// The process's vstdlib is looked up on the first call, and its result kept.
#[doc(alias("KeyValuesSystem"))]
pub fn key_values_system() -> Option<NonNull<IKeyValuesSystem>> {
	static KEY_VALUES_SYSTEM: OnceLock<Option<KeyValuesSystemFn>> = OnceLock::new();

	let key_values_system = (*KEY_VALUES_SYSTEM.get_or_init(find_key_values_system))?;

	// SAFETY: vstdlib's `KeyValuesSystem` returns its static system, and
	// vstdlib, which every Source module links against, stays loaded for as
	// long as any module that uses this crate.
	NonNull::new(unsafe { key_values_system() })
}
