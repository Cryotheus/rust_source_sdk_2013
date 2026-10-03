//! Builders of the send tables a game DLL networks its classes with, and
//! stand-ins for the engine's standard send proxies.

use crate::datatables::PropFlags;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::mem::zeroed;

/// A custom proxy, like `SendProxy_EHandleToInt`, that adds one to show it
/// ran.
///
/// # Safety
///
/// `data` must point to a `c_int`, and `out` to a writable `DVariant`.
pub unsafe extern "C" fn custom_proxy(
	_: *const sys::SendProp,
	_: *const c_void,
	data: *const c_void,
	out: *mut sys::DVariant,
	_: c_int,
	_: c_int,
) {
	// SAFETY: As the caller promises.
	unsafe { (*out).__bindgen_anon_1.m_Int = data.cast::<c_int>().read() + 1 };
}

/// A table proxy that passes the data through, like
/// `SendProxy_DataTableToDataTable`.
///
/// # Safety
///
/// None: it only returns `data`. It is `unsafe` to fit the proxy's type.
pub unsafe extern "C" fn direct_table(
	_: *const sys::SendProp,
	_: *const c_void,
	data: *const c_void,
	_: *mut sys::CSendProxyRecipients,
	_: c_int,
) -> *mut c_void {
	data.cast_mut()
}

/// Stands in for `SendProxy_Int8ToInt32`.
///
/// # Safety
///
/// `data` must point to an `i8`, and `out` to a writable `DVariant`.
pub unsafe extern "C" fn int8_proxy(
	_: *const sys::SendProp,
	_: *const c_void,
	data: *const c_void,
	out: *mut sys::DVariant,
	_: c_int,
	_: c_int,
) {
	// SAFETY: As the caller promises.
	unsafe { (*out).__bindgen_anon_1.m_Int = c_int::from(data.cast::<i8>().read()) };
}

/// Stands in for `SendProxy_Int16ToInt32`.
///
/// # Safety
///
/// `data` must point to an `i16`, and `out` to a writable `DVariant`.
pub unsafe extern "C" fn int16_proxy(
	_: *const sys::SendProp,
	_: *const c_void,
	data: *const c_void,
	out: *mut sys::DVariant,
	_: c_int,
	_: c_int,
) {
	// SAFETY: As the caller promises.
	unsafe { (*out).__bindgen_anon_1.m_Int = c_int::from(data.cast::<i16>().read()) };
}

/// Stands in for `SendProxy_Int32ToInt32`, and for the unsigned and float
/// proxies a linker may fold into it.
///
/// # Safety
///
/// `data` must point to a `c_int`, and `out` to a writable `DVariant`.
pub unsafe extern "C" fn int32_proxy(
	_: *const sys::SendProp,
	_: *const c_void,
	data: *const c_void,
	out: *mut sys::DVariant,
	_: c_int,
	_: c_int,
) {
	// SAFETY: As the caller promises.
	unsafe { (*out).__bindgen_anon_1.m_Int = data.cast::<c_int>().read() };
}

/// A table proxy that follows a pointer, relocating the nested table's data
/// like `SendProxy_DataTablePtrToDataTable`.
///
/// # Safety
///
/// `data` must point to a pointer.
pub unsafe extern "C" fn pointer_table(
	_: *const sys::SendProp,
	_: *const c_void,
	data: *const c_void,
	_: *mut sys::CSendProxyRecipients,
	_: c_int,
) -> *mut c_void {
	// SAFETY: As the caller promises.
	unsafe { data.cast::<*mut c_void>().read() }
}

/// A property of `kind` with one element and no nested table.
///
/// For tests only.
pub fn prop(
	name: &'static CStr,
	kind: sys::SendPropType,
	offset: c_int,
	flags: PropFlags,
	proxy: sys::SendVarProxyFn,
) -> sys::SendProp {
	// SAFETY: Properties are plain data apart from the vtable, which is never
	// used.
	let mut prop: sys::SendProp = unsafe { zeroed() };

	prop.m_pVarName = name.as_ptr();
	prop.m_Type = kind;
	prop.m_Offset = offset;
	prop.m_Flags = flags.bits();
	prop.m_ProxyFn = proxy;
	prop.m_nElements = 1;
	prop
}

/// Standard proxies with `non_modified` as the list of registered
/// pointer-preserving table proxies.
///
/// For tests only.
pub fn proxies(non_modified: *mut *mut sys::CNonModifiedPointerProxy) -> sys::CStandardSendProxies {
	sys::CStandardSendProxies {
		_base: sys::CStandardSendProxiesV1 {
			m_Int8ToInt32: Some(int8_proxy),
			m_Int16ToInt32: Some(int16_proxy),
			m_Int32ToInt32: Some(int32_proxy),
			// Linkers fold identical code, so unsigned proxies may share
			// addresses.
			m_UInt8ToInt32: Some(int8_proxy),
			m_UInt16ToInt32: Some(int16_proxy),
			m_UInt32ToInt32: Some(int32_proxy),
			m_FloatToFloat: Some(int32_proxy),
			m_VectorToVector: Some(vector_proxy),
		},
		m_DataTableToDataTable: Some(direct_table),
		m_SendLocalDataTable: Some(direct_table),
		m_ppNonModifiedPointerProxies: non_modified,
	}
}

/// A pointer-preserving table proxy that is not a standard one, so it is only
/// recognized through the list of registered proxies.
///
/// # Safety
///
/// None: it only returns `data`. It is `unsafe` to fit the proxy's type.
pub unsafe extern "C" fn registered_table(
	_: *const sys::SendProp,
	_: *const c_void,
	data: *const c_void,
	_: *mut sys::CSendProxyRecipients,
	_: c_int,
) -> *mut c_void {
	data.cast_mut()
}

/// A table of `props`, which must stay in place while it is used.
///
/// For tests only.
///
/// # Panics
///
/// If there are more properties than a `c_int` can count.
pub fn table(name: &'static CStr, props: &mut [sys::SendProp]) -> sys::SendTable {
	// SAFETY: Tables are plain data.
	let mut table: sys::SendTable = unsafe { zeroed() };

	table.m_pNetTableName = name.as_ptr().cast::<c_char>();
	table.m_pProps = props.as_mut_ptr();
	table.m_nProps = c_int::try_from(props.len()).expect("too many properties for a table");
	table
}

/// A [`PropKind::DataTable`] property nesting `table` through `proxy`.
///
/// For tests only.
///
/// [`PropKind::DataTable`]: crate::datatables::PropKind::DataTable
pub fn table_prop(
	name: &'static CStr,
	offset: c_int,
	table: *mut sys::SendTable,
	proxy: sys::SendTableProxyFn,
) -> sys::SendProp {
	let mut prop = self::prop(
		name,
		sys::SendPropType_DPT_DataTable,
		offset,
		PropFlags::default(),
		None,
	);

	prop.m_pDataTable = table;
	prop.m_DataTableProxyFn = proxy;
	prop
}

/// Stands in for `SendProxy_VectorToVector`.
///
/// # Safety
///
/// `data` must point to three `f32`s, and `out` to a writable `DVariant`.
pub unsafe extern "C" fn vector_proxy(
	_: *const sys::SendProp,
	_: *const c_void,
	data: *const c_void,
	out: *mut sys::DVariant,
	_: c_int,
	_: c_int,
) {
	// SAFETY: As the caller promises.
	unsafe { (*out).__bindgen_anon_1.m_Vector = data.cast::<[f32; 3]>().read() };
}
