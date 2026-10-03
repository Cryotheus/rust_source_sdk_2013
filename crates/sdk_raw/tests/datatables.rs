//! Tests of the game's send tables: calls and recognition of send proxies, and
//! `SendPropUtlVector`'s extra data.

use source_sdk_2013_raw::datatables::{
	SendPropExtraUtlVector, StandardVarProxies, call_var_proxy, is_direct_table_proxy,
	standard_var_proxies, utl_vector_extra,
};
use std::ffi::{c_int, c_void};
use std::mem::zeroed;
use std::ptr::{NonNull, null_mut};

unsafe extern "C" fn direct_table(
	_: *const sys::SendProp,
	_: *const c_void,
	data: *const c_void,
	_: *mut sys::CSendProxyRecipients,
	_: c_int,
) -> *mut c_void {
	data.cast_mut()
}

unsafe extern "C" fn int8_proxy(
	_: *const sys::SendProp,
	_: *const c_void,
	data: *const c_void,
	out: *mut sys::DVariant,
	_: c_int,
	_: c_int,
) {
	// SAFETY: The tests pass an 8-bit integer and a value to fill in.
	unsafe { (*out).__bindgen_anon_1.m_Int = c_int::from(data.cast::<i8>().read()) };
}

/// Stands in for every 32-bit proxy, as a linker may fold them.
unsafe extern "C" fn int32_proxy(
	_: *const sys::SendProp,
	_: *const c_void,
	data: *const c_void,
	out: *mut sys::DVariant,
	element: c_int,
	object_id: c_int,
) {
	// SAFETY: The tests pass a 32-bit integer and a value to fill in.
	unsafe {
		(*out).__bindgen_anon_1.m_Int = data.cast::<c_int>().read() + element * 10 + object_id
	};
}

/// A table proxy that relocates its data.
unsafe extern "C" fn pointer_table(
	_: *const sys::SendProp,
	_: *const c_void,
	data: *const c_void,
	_: *mut sys::CSendProxyRecipients,
	_: c_int,
) -> *mut c_void {
	// SAFETY: Never called.
	unsafe { data.cast::<*mut c_void>().read() }
}

/// A property with the given proxies, and nothing else.
fn prop(proxy: sys::SendVarProxyFn, table_proxy: sys::SendTableProxyFn) -> sys::SendProp {
	// SAFETY: Properties are plain data apart from the vtable, which is
	// never used.
	let mut prop: sys::SendProp = unsafe { zeroed() };

	prop.m_ProxyFn = proxy;
	prop.m_DataTableProxyFn = table_proxy;
	prop
}

/// Standard proxies with `registered` as the list of registered
/// pointer-preserving table proxies.
fn proxies(registered: *mut *mut sys::CNonModifiedPointerProxy) -> sys::CStandardSendProxies {
	sys::CStandardSendProxies {
		_base: sys::CStandardSendProxiesV1 {
			m_Int8ToInt32: Some(int8_proxy),
			m_Int16ToInt32: None,
			m_Int32ToInt32: Some(int32_proxy),
			m_UInt8ToInt32: Some(int8_proxy),
			m_UInt16ToInt32: None,
			m_UInt32ToInt32: Some(int32_proxy),
			m_FloatToFloat: Some(int32_proxy),
			m_VectorToVector: None,
		},
		m_DataTableToDataTable: Some(direct_table),
		m_SendLocalDataTable: None,
		m_ppNonModifiedPointerProxies: registered,
	}
}

#[test]
fn proxies_are_called_as_the_engine_calls_them() {
	let int = prop(Some(int32_proxy), None);
	let value: c_int = 40;
	let data = (&raw const value).cast();

	// SAFETY: The property is a local, and its proxy reads the integer.
	let called = unsafe { call_var_proxy(&int, data, data, 0, 2) }.unwrap();

	// SAFETY: The proxy filled in the integer.
	assert_eq!(unsafe { called.__bindgen_anon_1.m_Int }, 42);
	// SAFETY: The property has no proxy to call.
	assert!(unsafe { call_var_proxy(&prop(None, None), data, data, 0, 0) }.is_none());
}

/// A pointer-preserving table proxy that is not a standard one.
unsafe extern "C" fn registered_table(
	_: *const sys::SendProp,
	_: *const c_void,
	data: *const c_void,
	_: *mut sys::CSendProxyRecipients,
	_: c_int,
) -> *mut c_void {
	data.cast_mut()
}

#[test]
fn standard_proxies_are_recognized_by_address() {
	let mut registered = sys::CNonModifiedPointerProxy {
		m_Fn: Some(registered_table),
		m_pNext: null_mut(),
	};
	let mut head = &raw mut registered;
	let standard = proxies(&raw mut head);
	let with_list = |prop: sys::SendProp| {
		// SAFETY: The proxies and their list are locals, and so is the
		// property.
		unsafe { is_direct_table_proxy(&standard, &prop) }
	};

	assert!(with_list(prop(None, Some(direct_table))));
	assert!(with_list(prop(None, Some(registered_table))));
	assert!(!with_list(prop(None, Some(pointer_table))));
	assert!(!with_list(prop(None, None)));

	// SAFETY: The proxies and their list are locals, and so is the property.
	let matches = |proxy| unsafe { standard_var_proxies(&standard, &prop(proxy, None)) };

	assert_eq!(
		matches(Some(int8_proxy)),
		StandardVarProxies {
			int8: true,
			..StandardVarProxies::default()
		}
	);
	// Folded proxies match every standard proxy they stand for.
	assert_eq!(
		matches(Some(int32_proxy)),
		StandardVarProxies {
			int32: true,
			float: true,
			..StandardVarProxies::default()
		}
	);
	assert_eq!(matches(None), StandardVarProxies::default());
}

#[test]
fn utl_vector_extra_data_is_read_when_set_and_aligned() {
	let extra = SendPropExtraUtlVector {
		data_table_proxy: None,
		proxy: None,
		ensure_capacity: None,
		element_stride: 24,
		offset: 8,
		max_elements: 20,
	};
	let mut vector = prop(None, None);

	// SAFETY: The property is a local.
	let read = |vector: &sys::SendProp| unsafe { utl_vector_extra(vector) };

	assert_eq!(read(&vector), None);

	vector.m_pExtraData = (&raw const extra).cast();
	assert_eq!(
		read(&vector)
			.map(NonNull::as_ptr)
			.map(|extra| extra.cast_const()),
		Some(&raw const extra)
	);
	// SAFETY: The extra data is a local.
	assert_eq!(unsafe { read(&vector).unwrap().read() }.max_elements, 20);

	vector.m_pExtraData = (&raw const extra).cast::<u8>().wrapping_add(4).cast();
	assert_eq!(read(&vector), None);
}
