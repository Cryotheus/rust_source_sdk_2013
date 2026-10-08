//! Tests of replacing a property's send proxy with a trampoline, and of
//! putting the property's own proxy back.

use super::*;
use std::mem::zeroed;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

#[test]
fn a_handler_changes_what_the_games_proxy_sent_until_restored() {
	let prop = prop(Some(int_proxy));
	let calls = Arc::new(AtomicUsize::new(0));
	let counted = Arc::clone(&calls);

	let handler = move |call: ProxyCall| {
		counted.fetch_add(1, Ordering::Relaxed);

		// SAFETY: The trampoline passes the value the game's proxy wrote.
		unsafe {
			let value = &raw mut (*call.out).__bindgen_anon_1.m_Int;

			value.write(value.read() * 10 + call.element + call.object_id * 100);
		}
	};

	// SAFETY: The property is leaked, and the handler writes an integer to an
	// integer property. No other thread sends it.
	let slot = unsafe { install(prop, Box::new(handler)) }.unwrap();

	assert!(is_installed(slot));
	assert_ne!(proxy_address(prop), Some(address(int_proxy)));
	assert_eq!(send(prop, 4, 2, 7), 742);
	assert_eq!(calls.load(Ordering::Relaxed), 1);

	// Storage detection looks through the trampoline to the game's proxy.
	assert_eq!(
		// SAFETY: The property is live.
		unsafe { game_proxy(prop.as_ptr()) }.map(address),
		Some(address(int_proxy))
	);

	assert_eq!(
		// SAFETY: As for `install`.
		unsafe { install(prop, Box::new(|_| {})) },
		Err(InstallError::AlreadyOverridden)
	);

	// SAFETY: The property is leaked, and no other thread sends it.
	assert_eq!(unsafe { restore(slot) }, Restored::Restored);
	assert!(!is_installed(slot));
	assert_eq!(proxy_address(prop), Some(address(int_proxy)));
	assert_eq!(send(prop, 4, 2, 7), 4);

	// The handler was freed, and its captures with it.
	assert_eq!(Arc::strong_count(&calls), 1);

	// SAFETY: As above.
	assert_eq!(unsafe { restore(slot) }, Restored::Free);

	// A slot taken again is not the earlier override's to restore.
	// SAFETY: As for `install`.
	let again = unsafe { install(prop, Box::new(|_| {})) }.unwrap();
	// SAFETY: As above.
	assert_eq!(unsafe { restore(slot) }, Restored::Free);
	assert!(is_installed(again));
	// SAFETY: As above.
	assert_eq!(unsafe { restore(again) }, Restored::Restored);
}

#[test]
fn a_panicking_handler_leaves_what_the_game_sent() {
	let prop = prop(Some(int_proxy));

	let handler = |call: ProxyCall| {
		// SAFETY: As in the test above.
		unsafe { (*call.out).__bindgen_anon_1.m_Int = -1 };
		panic!("the handler fails");
	};

	// SAFETY: As in the test above.
	let slot = unsafe { install(prop, Box::new(handler)) }.unwrap();

	// The handler wrote before panicking, which a handler may: the value is
	// still one of the property's type.
	assert_eq!(send(prop, 5, 0, 1), -1);

	// SAFETY: As in the test above.
	assert_eq!(unsafe { restore(slot) }, Restored::Restored);
}

#[test]
fn a_property_without_a_proxy_is_refused() {
	let prop = prop(None);

	assert_eq!(
		// SAFETY: As in the tests above.
		unsafe { install(prop, Box::new(|_| {})) },
		Err(InstallError::NoProxy)
	);
}

#[test]
fn a_trampoline_another_proxy_replaced_is_retired_and_still_passes_through() {
	let prop = prop(Some(int_proxy));

	let handler = |call: ProxyCall| {
		// SAFETY: As in the tests above.
		unsafe { (*call.out).__bindgen_anon_1.m_Int += 1 };
	};

	// SAFETY: As in the tests above.
	let slot = unsafe { install(prop, Box::new(handler)) }.unwrap();

	// Another plugin keeps the trampoline to call, and puts its own proxy in.
	// SAFETY: As in `send`.
	let trampoline = unsafe { (&raw const (*prop.as_ptr()).m_ProxyFn).read() };
	// SAFETY: As above.
	unsafe { (&raw mut (*prop.as_ptr()).m_ProxyFn).write(Some(other_plugins_proxy)) };

	// SAFETY: As in the tests above.
	assert_eq!(unsafe { restore(slot) }, Restored::Retired);
	assert!(!is_installed(slot));
	assert_eq!(proxy_address(prop), Some(address(other_plugins_proxy)));

	// The other plugin puts the trampoline back as it stops: it now sends what
	// the game's proxy sends, without the handler.
	// SAFETY: As above.
	unsafe { (&raw mut (*prop.as_ptr()).m_ProxyFn).write(trampoline) };
	assert_eq!(send(prop, 9, 0, 1), 9);

	// The retired slot is not the property's override any more, so another
	// can be installed, over the trampoline.
	// SAFETY: As in the tests above.
	let next = unsafe { install(prop, Box::new(|_| {})) }.unwrap();
	assert_ne!(next, slot);
	// SAFETY: As in the tests above.
	assert_eq!(unsafe { restore(next) }, Restored::Restored);
	assert_eq!(proxy_address(prop), trampoline.map(address));
}

/// A proxy's address.
fn address(proxy: ProxyFn) -> usize {
	proxy as usize
}

/// Stands in for a game proxy that sends a `c_int` variable as it is.
///
/// # Safety
///
/// `data` must point to a `c_int`, and `out` to a writable `DVariant`.
unsafe extern "C" fn int_proxy(
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

#[test]
fn other_copies_trampolines_are_looked_through() {
	// The game's proxy, under one of this copy's trampolines, under another
	// copy's, which `other_plugins_proxy` stands for.
	let under = prop(Some(int_proxy));
	// SAFETY: As in the tests above.
	let ours = unsafe { install(under, Box::new(|_| {})) }.unwrap();
	let trampoline = TRAMPOLINES[ours.index()];
	let calls_ours =
		|proxy: ProxyFn| (address(proxy) == address(other_plugins_proxy)).then_some(trampoline);

	assert_eq!(
		look_through(other_plugins_proxy, calls_ours).map(address),
		Some(address(int_proxy))
	);

	// The game's proxy, under another copy's trampoline, under one of this
	// copy's.
	let over = prop(Some(other_plugins_proxy));
	// SAFETY: As in the tests above.
	let theirs = unsafe { install(over, Box::new(|_| {})) }.unwrap();
	let calls_game = |proxy: ProxyFn| {
		(address(proxy) == address(other_plugins_proxy)).then_some(int_proxy as ProxyFn)
	};

	assert_eq!(
		look_through(TRAMPOLINES[theirs.index()], calls_game).map(address),
		Some(address(int_proxy))
	);

	// A proxy that is no copy's trampoline is the game's, and a chain that
	// never ends stops.
	assert_eq!(
		look_through(int_proxy, |_| None).map(address),
		Some(address(int_proxy))
	);
	assert_eq!(
		look_through(other_plugins_proxy, Some).map(address),
		Some(address(other_plugins_proxy))
	);

	// SAFETY: The properties are leaked, and no other thread sends them.
	unsafe {
		assert_eq!(restore(ours), Restored::Restored);
		assert_eq!(restore(theirs), Restored::Restored);
	}
}

/// Stands in for another plugin's proxy, which a test chains over a
/// trampoline by hand.
///
/// # Safety
///
/// None: it does nothing. It is `unsafe` to fit the proxy's type.
unsafe extern "C" fn other_plugins_proxy(
	_: *const sys::SendProp,
	_: *const c_void,
	_: *const c_void,
	_: *mut sys::DVariant,
	_: c_int,
	_: c_int,
) {
}

/// A property sent through `proxy`, leaked so it outlives any slot.
fn prop(proxy: sys::SendVarProxyFn) -> NonNull<sys::SendProp> {
	// SAFETY: Properties are plain data apart from the vtable, which is never
	// used.
	let mut prop: sys::SendProp = unsafe { zeroed() };

	prop.m_Type = sys::SendPropType_DPT_Int;
	prop.m_ProxyFn = proxy;
	NonNull::from(Box::leak(Box::new(prop)))
}

/// The property's proxy, as an address.
fn proxy_address(prop: NonNull<sys::SendProp>) -> Option<usize> {
	// SAFETY: As in `send`.
	unsafe { (&raw const (*prop.as_ptr()).m_ProxyFn).read() }.map(address)
}

/// Calls the property's proxy, as the engine does, for a variable holding
/// `value`, and returns what it sent.
fn send(prop: NonNull<sys::SendProp>, value: c_int, element: c_int, object_id: c_int) -> c_int {
	// SAFETY: The property is the test's, and its field is read without
	// forming a reference.
	let proxy = unsafe { (&raw const (*prop.as_ptr()).m_ProxyFn).read() }.unwrap();

	// SAFETY: The union is plain data, for which zeroes are valid.
	let mut out: sys::DVariant = unsafe { zeroed() };

	// SAFETY: The proxy is the test's or a trampoline calling it, with a
	// `c_int` variable and a local value.
	unsafe {
		proxy(
			prop.as_ptr(),
			ptr::null(),
			(&raw const value).cast(),
			&raw mut out,
			element,
			object_id,
		);

		out.__bindgen_anon_1.m_Int
	}
}

#[test]
fn the_export_names_what_this_copys_trampolines_call() {
	let prop = prop(Some(int_proxy));
	// SAFETY: As in the tests above.
	let slot = unsafe { install(prop, Box::new(|_| {})) }.unwrap();
	let trampoline = TRAMPOLINES[slot.index()] as *const c_void;

	assert_eq!(trampoline_original(trampoline) as usize, address(int_proxy));
	assert!(trampoline_original(int_proxy as *const c_void).is_null());

	// A proxy in a library without the export, such as this test's, is the
	// game's.
	let foreign = self::prop(Some(other_plugins_proxy));

	assert_eq!(
		// SAFETY: The property is live, and no library unloads.
		unsafe { game_proxy(foreign.as_ptr()) }.map(address),
		Some(address(other_plugins_proxy))
	);

	// SAFETY: As in the tests above.
	assert_eq!(unsafe { restore(slot) }, Restored::Restored);
	assert!(trampoline_original(trampoline).is_null());
}
