//! tier1's `KeyValues`, as far as reading the names, string values and own key
//! values of key values the engine or game made, and vstdlib's
//! `IKeyValuesSystem`, whose symbol table holds most names.
//!
//! Key values name themselves by a symbol, which `KeyValues::GetName` looks up
//! in the table of the `IKeyValuesSystem` that vstdlib's `KeyValuesSystem`
//! returns. Every module that links tier1 shares that table, unless it moved
//! its own key values to a table of their own
//! (`KeyValues::SetUseGrowableStringTable`), which neither the SDK's game code
//! nor its tier1 does by default.
//!
//! TF2 is built with a newer tier1 than the SDK's, whose key values can also
//! keep their names in a table of their own. TF2's engine makes the key values
//! it reads from clients' commands so (`Base_CmdKeyValues::ReadFromBuffer`),
//! which keeps clients from filling the process's table. Those key values set
//! the byte the SDK's header leaves `unused`, and are 72 bytes rather than 64,
//! with a pointer to their table after the SDK's fields. Their symbols are
//! offsets into the table's strings. This layout was read from TF2's 64-bit
//! Windows `engine.dll` and `server.dll`. The Linux build is assumed to match,
//! and [`key_name`] reads only from tables that look as expected.

#[cfg(test)]
#[path = "tests/key_values.rs"]
mod tests;

use crate::util::loaded_symbol;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::mem::offset_of;
use std::ptr::{NonNull, null, null_mut};
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
// at offset 0, of the 64 bytes the generated bindings assert on both ABIs,
// which end with `m_pPeer`, `m_pSub` and `m_pChain`. TF2's tier1 puts the
// pointer to the key values' own table after those.
const _: () = assert!(
	GET_SYMBOL_FOR_STRING_SLOT == 3
		&& GET_STRING_FOR_SYMBOL_SLOT == 4
		&& size_of::<sys::KeyValues>() == 64
		&& PEER_OFFSET == 64 - 3 * size_of::<usize>()
		&& SUB_OFFSET == 64 - 2 * size_of::<usize>()
		&& OWN_NAMES_TABLE_OFFSET == 64
);

/// Where key values keep `m_iDataType`, the type of their value.
const DATA_TYPE_OFFSET: usize = 0x20;

/// `IKeyValuesSystem::GetStringForSymbol`'s slot.
#[doc(alias("GetStringForSymbol"))]
pub const GET_STRING_FOR_SYMBOL_SLOT: usize =
	offset_of!(IKeyValuesSystemVtable, get_string_for_symbol) / size_of::<usize>();

/// `IKeyValuesSystem::GetSymbolForString`'s slot.
#[doc(alias("GetSymbolForString"))]
pub const GET_SYMBOL_FOR_STRING_SLOT: usize =
	offset_of!(IKeyValuesSystemVtable, get_symbol_for_string) / size_of::<usize>();

/// `INVALID_KEY_SYMBOL`: the symbol of key values without a name.
const INVALID_KEY_SYMBOL: HKeySymbol = -1;

/// The names vstdlib has: on Windows, and in 64-bit and older Linux dedicated
/// servers.
const LIBRARIES: &[&CStr] = cfg_select! {
	windows => &[c"vstdlib.dll"],
	target_os = "linux" => &[c"libvstdlib.so", c"libvstdlib_srv.so"],
};

/// Where, in a table of names that key values keep for themselves, its
/// `CUtlVector<char>` of strings has its elements' pointer, which mirrors its
/// memory's.
const OWN_NAMES_ELEMENTS_OFFSET: usize = 0x70;

/// Where, in key values that keep their names in a table of their own, TF2's
/// tier1 marks them so: the byte the SDK's header leaves `unused`, after
/// `m_bEvaluateConditionals`.
const OWN_NAMES_FLAG_OFFSET: usize = 0x23;

/// Where, in a table of names that key values keep for themselves, its
/// `CUtlVector<char>` of strings has its memory's pointer: after the table's
/// mutex and its hash of symbols.
const OWN_NAMES_MEMORY_OFFSET: usize = 0x58;

/// Where, in a table of names that key values keep for themselves, its
/// `CUtlVector<char>` of strings has its size.
const OWN_NAMES_SIZE_OFFSET: usize = 0x68;

/// Where, in key values that keep their names in a table of their own, TF2's
/// tier1 keeps the pointer to the table: after the SDK's 64 bytes.
const OWN_NAMES_TABLE_OFFSET: usize = 0x40;

/// Where key values keep `m_pPeer`, the next key values in their parent's
/// list, or null after the last.
const PEER_OFFSET: usize = 0x28;

/// Where key values keep `m_sValue`, their value as a string, which they own
/// and free with themselves.
const STRING_OFFSET: usize = 0x08;

/// Where key values keep `m_pSub`, the first of their own key values, or null
/// if they have none.
const SUB_OFFSET: usize = 0x30;

/// `KeyValues::TYPE_STRING`: the type of key values whose value is a string.
const TYPE_STRING: u8 = 1;

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

/// The first of `key_values`' own key values, as `KeyValues::GetFirstSubKey`
/// returns it, or null if they have none.
///
/// # Safety
///
/// `key_values` must point to live key values laid out as tier1 lays them out.
/// The result, if not null, points to key values of the same tree, which live
/// as long as their parent does.
#[doc(alias("GetFirstSubKey", "m_pSub"))]
pub unsafe fn first_sub_key(key_values: NonNull<sys::KeyValues>) -> *mut sys::KeyValues {
	// SAFETY: The caller passes live key values, which have the aligned pointer
	// among their first 64 bytes.
	unsafe {
		key_values
			.cast::<u8>()
			.add(SUB_OFFSET)
			.cast::<*mut sys::KeyValues>()
			.read()
	}
}

/// The name of `key_values`, found as TF2's `KeyValues::GetName` finds it: in
/// their own table of names if they keep one, as the key values the engine
/// reads from clients do, or else as the string their symbol stands for in
/// `system`'s table.
///
/// Returns null if the name is in `system`'s table and `system` is `None`, or
/// in a table of their own that does not look as TF2's tier1 lays it out, or
/// that does not have their symbol.
///
/// # Safety
///
/// `key_values` must point to live key values laid out as TF2's tier1 lays
/// them out: 72 bytes if they keep their names in a table of their own, whose
/// pointer, if not null, points to that live table of 120 bytes. If they are
/// named by a symbol of the process's table, `system` must be live if `Some`.
///
/// A name from `system`'s table lives as long as `system` does, and one from
/// the key values' own table as long as the key values do, unchanged.
#[doc(alias("GetName"))]
pub unsafe fn key_name(
	system: Option<NonNull<IKeyValuesSystem>>,
	key_values: NonNull<sys::KeyValues>,
) -> *const c_char {
	let bytes = key_values.cast::<u8>();

	// SAFETY: The caller passes live key values, whose first field is
	// `m_iKeyName`, and which have the flag among their first 64 bytes.
	let (symbol, own_names) = unsafe {
		(
			key_values.cast::<HKeySymbol>().read(),
			bytes.add(OWN_NAMES_FLAG_OFFSET).read() != 0,
		)
	};

	if own_names {
		// SAFETY: Key values that keep their names are 72 bytes, the last 8 of
		// them the aligned pointer to their table, as the caller promises.
		let table = unsafe { bytes.add(OWN_NAMES_TABLE_OFFSET).cast::<*const u8>().read() };

		// SAFETY: The caller promises the table is null or live.
		return unsafe { own_name(table, symbol) };
	}

	let Some(system) = system else { return null() };

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

/// The key values after `key_values` in their parent's list, as
/// `KeyValues::GetNextKey` returns them, or null after the last.
///
/// # Safety
///
/// As for [`first_sub_key`].
#[doc(alias("GetNextKey", "m_pPeer"))]
pub unsafe fn next_key(key_values: NonNull<sys::KeyValues>) -> *mut sys::KeyValues {
	// SAFETY: As for `first_sub_key`.
	unsafe {
		key_values
			.cast::<u8>()
			.add(PEER_OFFSET)
			.cast::<*mut sys::KeyValues>()
			.read()
	}
}

/// The name `symbol` stands for in `table`, a table of names that key values
/// keep for themselves: the string at that offset in its strings. Returns an
/// empty name for [`INVALID_KEY_SYMBOL`], as the game does, and null if
/// `table` is null, its strings' vector does not look as TF2's tier1 lays it
/// out, or `symbol` is not among its strings.
///
/// # Safety
///
/// `table` must be null or point to a live table of 120 bytes, aligned for
/// pointers.
unsafe fn own_name(table: *const u8, symbol: HKeySymbol) -> *const c_char {
	if symbol == INVALID_KEY_SYMBOL {
		return c"".as_ptr();
	}

	if table.is_null() {
		return null();
	}

	// SAFETY: The table is live, aligned and 120 bytes, as the caller
	// promises, which covers each field, at an offset aligned for it.
	let (memory, size, elements) = unsafe {
		(
			table
				.add(OWN_NAMES_MEMORY_OFFSET)
				.cast::<*const c_char>()
				.read(),
			table.add(OWN_NAMES_SIZE_OFFSET).cast::<c_int>().read(),
			table
				.add(OWN_NAMES_ELEMENTS_OFFSET)
				.cast::<*const c_char>()
				.read(),
		)
	};

	// `CUtlVector` keeps its elements' pointer equal to its memory's, so
	// fields read from elsewhere are unlikely to pass.
	if memory.is_null() || memory != elements || !(0..size).contains(&symbol) {
		return null();
	}

	// SAFETY: The symbol is an offset within the vector's `size` bytes, where
	// the table writes each name with its NUL.
	unsafe { memory.add(symbol as usize) }
}

/// The value of `key_values` whose value is a string, which they own, or null
/// for key values of another type, such as those that only hold key values
/// of their own.
///
/// `KeyValues::GetString` converts values of other types to strings, and keeps
/// the string in place of the value. This only reads.
///
/// # Safety
///
/// As for [`first_sub_key`]. The string, if any, is NUL-terminated, and lives
/// until its key values change their value or are freed.
#[doc(alias("GetString", "m_sValue"))]
pub unsafe fn string_value(key_values: NonNull<sys::KeyValues>) -> *mut c_char {
	let bytes = key_values.cast::<u8>();

	// SAFETY: As for `first_sub_key`, with the type a byte, and the string an
	// aligned pointer, among their first 64 bytes.
	unsafe {
		if bytes.add(DATA_TYPE_OFFSET).read() != TYPE_STRING {
			return null_mut();
		}

		bytes.add(STRING_OFFSET).cast::<*mut c_char>().read()
	}
}
