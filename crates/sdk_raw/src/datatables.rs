//! Hand-written ABI of the game's send tables that the generated bindings do
//! not describe: the `SPROP_*` flags of their properties, recognition of the
//! game's standard send proxies, calls of a property's proxy, and the extra
//! data `SendPropUtlVector` gives the properties it builds.
//!
//! Send proxies are compared by address only: a linker may fold proxies whose
//! code is identical into one, so an address can stand for several of them.

use crate::send_proxies::game_proxy;
use std::ffi::{c_int, c_void};
use std::mem::{offset_of, zeroed};
use std::ptr::NonNull;

/// `EnsureCapacityFn` from `public/dt_utlvector_common.h`, which grows the
/// `CUtlVector` `offset_to_utl_vector` bytes into `object` to hold `len`
/// elements.
pub type EnsureCapacityFn =
	unsafe extern "C" fn(object: *mut c_void, offset_to_utl_vector: c_int, len: c_int);

// `CSendPropExtra_UtlVector` has no bases or virtual methods, so both
// supported ABIs lay it out as C does.
const _: () = {
	use SendPropExtraUtlVector as Extra;

	assert!(offset_of!(Extra, element_stride) == 3 * size_of::<*const ()>());
	assert!(offset_of!(Extra, offset) == offset_of!(Extra, element_stride) + size_of::<c_int>());
	assert!(offset_of!(Extra, max_elements) == offset_of!(Extra, offset) + size_of::<c_int>());
	assert!(size_of::<Extra>() == 40);
};

/// How many nodes of the game's list of registered pointer-preserving table
/// proxies are read before the list is assumed corrupt.
pub const MAX_NON_MODIFIED_PROXIES: usize = 256;

/// `SPROP_CHANGES_OFTEN` from `public/dt_common.h`: the variable changes
/// often, so the engine moves it to the start of its table.
pub const SPROP_CHANGES_OFTEN: c_int = 1 << 10;

/// `SPROP_COLLAPSIBLE` from `public/dt_common.h`: the data table sits at
/// offset 0 behind `SendProxy_DataTableToDataTable`, so the engine can
/// flatten it away.
pub const SPROP_COLLAPSIBLE: c_int = 1 << 12;

/// `SPROP_COORD` from `public/dt_common.h`: the float or vector is a world
/// coordinate.
pub const SPROP_COORD: c_int = 1 << 1;

/// `SPROP_COORD_MP` from `public/dt_common.h`: like [`SPROP_COORD`], with
/// special handling for multiplayer games.
pub const SPROP_COORD_MP: c_int = 1 << 13;

/// `SPROP_COORD_MP_INTEGRAL` from `public/dt_common.h`: like
/// [`SPROP_COORD_MP`], with coordinates rounded to whole units.
pub const SPROP_COORD_MP_INTEGRAL: c_int = 1 << 15;

/// `SPROP_COORD_MP_LOWPRECISION` from `public/dt_common.h`: like
/// [`SPROP_COORD_MP`], with 3 bits for the fractional part instead of 5.
pub const SPROP_COORD_MP_LOWPRECISION: c_int = 1 << 14;

/// `SPROP_ENCODED_AGAINST_TICKCOUNT` from `public/dt_common.h`: the
/// integer's proxy encodes it relative to the tick count.
pub const SPROP_ENCODED_AGAINST_TICKCOUNT: c_int = 1 << 16;

/// `SPROP_EXCLUDE` from `public/dt_common.h`: the property names another
/// property to exclude, rather than a variable.
pub const SPROP_EXCLUDE: c_int = 1 << 6;

/// `SPROP_INSIDEARRAY` from `public/dt_common.h`: the property describes the
/// elements of the array property after it.
pub const SPROP_INSIDEARRAY: c_int = 1 << 8;

/// `SPROP_IS_A_VECTOR_ELEM` from `public/dt_common.h`: the property is one
/// component of a vector.
pub const SPROP_IS_A_VECTOR_ELEM: c_int = 1 << 11;

/// `SPROP_NORMAL` from `public/dt_common.h`: the vector is a normal. Integer
/// properties reuse the bit as `SPROP_VARINT`.
#[doc(alias("SPROP_VARINT"))]
pub const SPROP_NORMAL: c_int = 1 << 5;

/// `SPROP_NOSCALE` from `public/dt_common.h`: the float is sent as is, rather
/// than scaled into its range.
pub const SPROP_NOSCALE: c_int = 1 << 2;

/// `SPROP_PROXY_ALWAYS_YES` from `public/dt_common.h`: the data table's proxy
/// is a standard one that sends the table to every client.
pub const SPROP_PROXY_ALWAYS_YES: c_int = 1 << 9;

/// `SPROP_ROUNDDOWN` from `public/dt_common.h`: the float's high value is
/// lowered by one encoding step.
pub const SPROP_ROUNDDOWN: c_int = 1 << 3;

/// `SPROP_ROUNDUP` from `public/dt_common.h`: the float's low value is raised
/// by one encoding step.
pub const SPROP_ROUNDUP: c_int = 1 << 4;

/// `SPROP_UNSIGNED` from `public/dt_common.h`: the integer is networked
/// unsigned.
pub const SPROP_UNSIGNED: c_int = 1 << 0;

/// `SPROP_XYZE` from `public/dt_common.h`: the vector uses XYZ/exponent
/// encoding.
pub const SPROP_XYZE: c_int = 1 << 7;

/// `CSendPropExtra_UtlVector` from `public/dt_utlvector_send.cpp`, which
/// `SendPropUtlVector` allocates for each networked vector, never frees, and
/// shares between the properties of the table it builds for the vector.
///
/// The class is private to that file, so the generated bindings lack it.
#[doc(alias("CSendPropExtra_UtlVector"))]
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct SendPropExtraUtlVector {
	/// `m_DataTableProxyFn`: the proxy given for the elements of a vector of
	/// data tables.
	#[doc(alias("m_DataTableProxyFn"))]
	pub data_table_proxy: sys::SendTableProxyFn,

	/// `m_ProxyFn`: the proxy given for the elements of a vector of other
	/// values.
	#[doc(alias("m_ProxyFn"))]
	pub proxy: sys::SendVarProxyFn,

	/// `m_EnsureCapacityFn`: grows the vector.
	#[doc(alias("m_EnsureCapacityFn"))]
	pub ensure_capacity: Option<EnsureCapacityFn>,

	/// `m_ElementStride`: the size of each element.
	#[doc(alias("m_ElementStride"))]
	pub element_stride: c_int,

	/// `m_Offset`: bytes from the structure the vector's property belongs to,
	/// to the vector.
	#[doc(alias("m_Offset"))]
	pub offset: c_int,

	/// `m_nMaxElements`: the most elements networked.
	#[doc(alias("m_nMaxElements"))]
	pub max_elements: c_int,
}

/// Which of the game's standard variable proxies (`CStandardSendProxies`) a
/// property's proxy is.
///
/// Several can hold for one proxy, which a linker folded them into. An
/// unsigned proxy is only told apart from its signed counterpart by the
/// property's [`SPROP_UNSIGNED`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct StandardVarProxies {
	/// `m_Int8ToInt32` or `m_UInt8ToInt32`, which read an 8-bit integer.
	pub int8: bool,

	/// `m_Int16ToInt32` or `m_UInt16ToInt32`, which read a 16-bit integer.
	pub int16: bool,

	/// `m_Int32ToInt32` or `m_UInt32ToInt32`, which read a 32-bit integer.
	pub int32: bool,

	/// `m_FloatToFloat`, which reads a float.
	pub float: bool,

	/// `m_VectorToVector`, which reads three floats.
	pub vector: bool,
}

/// Calls the property's variable proxy (`m_ProxyFn`) as the engine does when
/// it encodes the variable, and returns the value it gave, or `None` if the
/// property has no proxy.
///
/// The proxy is passed the property, `struct_base` and `data`, the value to
/// fill in, which starts zeroed, `element`, and `object_id`.
///
/// # Safety
///
/// - `prop` must point to a live `SendProp` of the loaded game DLL, and the
///   call must be made on the server's main thread.
/// - The arguments must be those the engine passes the proxy: `struct_base`
///   must point to the structure the property's table describes, and `data`
///   to the variable within it, both within a live object of a class the
///   property belongs to. `element` must be the variable's index in its
///   array, or 0, and `object_id` the object's edict index.
#[doc(alias("SendVarProxyFn"))]
pub unsafe fn call_var_proxy(
	prop: *const sys::SendProp,
	struct_base: *const c_void,
	data: *const c_void,
	element: c_int,
	object_id: c_int,
) -> Option<sys::DVariant> {
	// SAFETY: The property is live, and its field is read without forming a
	// reference.
	let proxy = unsafe { (&raw const (*prop).m_ProxyFn).read() }?;

	// SAFETY: The union is plain data, for which zeroes are valid.
	let mut value: sys::DVariant = unsafe { zeroed() };

	// SAFETY: The proxy is the game's own, called with the arguments the
	// engine passes, as the caller promises, and the value is a local.
	unsafe { proxy(prop, struct_base, data, &raw mut value, element, object_id) };

	Some(value)
}

/// Calls the property's data table proxy (`m_DataTableProxyFn`) as the
/// engine does when it encodes the nested table, and returns the structure it
/// gave, which is null when the table is not sent, or `None` if the property
/// has no proxy.
///
/// The proxy is passed the property, `struct_base`, `data`, recipients that
/// start with every client set, as the engine's do, and `object_id`. The
/// recipients it leaves are discarded.
///
/// # Safety
///
/// - `prop` must point to a live `DPT_DataTable` `SendProp` of the loaded game
///   DLL, and the call must be made on the server's main thread.
/// - The arguments must be those the engine passes the proxy: `struct_base`
///   must point to the structure the property's table describes, and `data`
///   to the nested structure within it, both within a live object of a class
///   the property belongs to, and `object_id` must be the object's edict
///   index.
#[doc(alias("SendTableProxyFn"))]
pub unsafe fn call_table_proxy(
	prop: *const sys::SendProp,
	struct_base: *const c_void,
	data: *const c_void,
	object_id: c_int,
) -> Option<*mut c_void> {
	// SAFETY: The property is live, and its field is read without forming a
	// reference.
	let proxy = unsafe { (&raw const (*prop).m_DataTableProxyFn).read() }?;

	// A bit vector of 255 clients in eight 32-bit words, all set, as
	// `CSendProxyRecipients::SetAllRecipients` leaves it.
	let mut recipients = sys::CSendProxyRecipients {
		m_Bits: sys::__BindgenOpaqueArray([u32::MAX; 8]),
	};

	// SAFETY: The proxy is the game's own, called with the arguments the
	// engine passes, as the caller promises, and the recipients are a local.
	Some(unsafe { proxy(prop, struct_base, data, &raw mut recipients, object_id) })
}

/// Whether the property's data table proxy (`m_DataTableProxyFn`) passes the
/// structure it is given through unchanged, so that the offsets of the
/// nested table's properties are relative to the containing structure.
///
/// That is `m_DataTableToDataTable`, `m_SendLocalDataTable`, or one of the
/// proxies the game registers in its list of pointer-preserving table proxies
/// (`m_ppNonModifiedPointerProxies`), which the engine consults for the same
/// purpose. At most [`MAX_NON_MODIFIED_PROXIES`] nodes of the list are read.
/// A property without a proxy has none that passes its data through.
///
/// # Safety
///
/// `proxies` must point to the game DLL's `g_StandardSendProxies`, or a copy,
/// whose list of pointer-preserving table proxies is null or points to the
/// head of a list whose nodes stay allocated and unmodified for the call, as
/// the game's static ones do. `prop` must point to a live `SendProp`.
pub unsafe fn is_direct_table_proxy(
	proxies: *const sys::CStandardSendProxies,
	prop: *const sys::SendProp,
) -> bool {
	// SAFETY: The property is live, and its field is read without forming a
	// reference.
	let Some(proxy) =
		table_proxy_address(unsafe { (&raw const (*prop).m_DataTableProxyFn).read() })
	else {
		return false;
	};

	// SAFETY: The proxies are live, and copied without forming a reference.
	let proxies = unsafe { proxies.read() };

	if [proxies.m_DataTableToDataTable, proxies.m_SendLocalDataTable]
		.into_iter()
		.any(|candidate| table_proxy_address(candidate) == Some(proxy))
	{
		return true;
	}

	let Some(head) = NonNull::new(proxies.m_ppNonModifiedPointerProxies) else {
		return false;
	};

	// SAFETY: The list's head and nodes stay allocated and unmodified, as the
	// caller promises.
	let mut node = unsafe { head.as_ptr().read() };

	for _ in 0..MAX_NON_MODIFIED_PROXIES {
		let Some(current) = NonNull::new(node) else {
			return false;
		};

		// SAFETY: As above.
		let current = unsafe { current.as_ptr().read() };

		if table_proxy_address(current.m_Fn) == Some(proxy) {
			return true;
		}

		node = current.m_pNext;
	}

	false
}

/// Which of the game's standard variable proxies the property's proxy
/// (`m_ProxyFn`) is. A property without a proxy, or with another one, such
/// as a custom or string proxy, is none of them.
///
/// The proxy is the one the game gave the property, looking through the
/// overrides of [`send_proxies`](crate::send_proxies), whose trampolines call
/// it.
///
/// # Safety
///
/// `proxies` must point to the game DLL's `g_StandardSendProxies`, or a copy,
/// and `prop` to a live `SendProp`.
pub unsafe fn standard_var_proxies(
	proxies: *const sys::CStandardSendProxies,
	prop: *const sys::SendProp,
) -> StandardVarProxies {
	// SAFETY: The property is live.
	let Some(proxy) = var_proxy_address(unsafe { game_proxy(prop) }) else {
		return StandardVarProxies::default();
	};

	// SAFETY: The proxies are live, and copied without forming a reference.
	let standard = unsafe { proxies.read() }._base;

	let is = |candidates: &[sys::SendVarProxyFn]| {
		candidates
			.iter()
			.any(|&candidate| var_proxy_address(candidate) == Some(proxy))
	};

	StandardVarProxies {
		int8: is(&[standard.m_Int8ToInt32, standard.m_UInt8ToInt32]),
		int16: is(&[standard.m_Int16ToInt32, standard.m_UInt16ToInt32]),
		int32: is(&[standard.m_Int32ToInt32, standard.m_UInt32ToInt32]),
		float: is(&[standard.m_FloatToFloat]),
		vector: is(&[standard.m_VectorToVector]),
	}
}

/// The address of a table proxy, for comparing proxies.
fn table_proxy_address(proxy: sys::SendTableProxyFn) -> Option<usize> {
	proxy.map(|proxy| proxy as usize)
}

/// The property's extra data (`m_pExtraData`), as the
/// [`SendPropExtraUtlVector`] that `SendPropUtlVector` gives the properties
/// it builds, or `None` if it is null or misaligned for one.
///
/// Only `SendPropUtlVector` gives properties extra data in the SDK, but the
/// pointer is not checked to be one: whoever reads it must know the property
/// to be one of those it builds.
///
/// # Safety
///
/// `prop` must point to a live `SendProp`.
#[doc(alias("m_pExtraData"))]
pub unsafe fn utl_vector_extra(
	prop: *const sys::SendProp,
) -> Option<NonNull<SendPropExtraUtlVector>> {
	// SAFETY: The property is live, and its field is read without forming a
	// reference.
	let extra = unsafe { (&raw const (*prop).m_pExtraData).read() };

	NonNull::new(extra.cast_mut().cast::<SendPropExtraUtlVector>())
		.filter(|extra| extra.as_ptr().is_aligned())
}

/// The address of a variable proxy, for comparing proxies.
fn var_proxy_address(proxy: sys::SendVarProxyFn) -> Option<usize> {
	proxy.map(|proxy| proxy as usize)
}
