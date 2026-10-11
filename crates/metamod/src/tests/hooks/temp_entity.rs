//! Tests of `crate::hooks::temp_entity`: pre hooks of `PlaybackTempEntity` on
//! mock engines, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::raw::abi::VTABLE_SLOT_SIZE;
use source_sdk_2013::raw::vtable_slot;
use std::cell::RefCell;
use std::ffi::{c_char, c_void};
use std::mem::zeroed;
use std::ptr;

thread_local! {
	/// What happened during the playbacks since the last [`play`], in order.
	static SEEN: RefCell<Vec<Seen>> = const { RefCell::new(Vec::new()) };
}

/// An engine interface of a C++ class, as far as hooks know it.
#[repr(C)]
struct Engine {
	vtable: *mut *mut c_void,
}

impl Engine {
	fn new() -> Box<Self> {
		let vtable = mock_vtable::<sys::IVEngineServer__bindgen_vtable>(&[(
			PLAYBACK_TEMP_ENTITY_SLOT,
			engine_playback as *mut c_void,
		)]);

		Box::new(Self { vtable })
	}

	fn ptr(&mut self) -> NonNull<sys::IVEngineServer> {
		NonNull::from(self).cast()
	}
}

/// A recipient filter of a C++ class, which lists its clients.
#[repr(C)]
struct Filter {
	vtable: *mut *mut c_void,
	clients: Vec<c_int>,
	reliable: bool,
}

impl Filter {
	fn new(clients: Vec<c_int>, reliable: bool) -> Box<Self> {
		let vtable = mock_vtable::<sys::IRecipientFilter__bindgen_vtable>(&[
			(
				vtable_slot!(
					sys::IRecipientFilter__bindgen_vtable,
					IRecipientFilter_IsReliable
				),
				filter_reliable as *mut c_void,
			),
			(
				vtable_slot!(
					sys::IRecipientFilter__bindgen_vtable,
					IRecipientFilter_GetRecipientCount
				),
				filter_count as *mut c_void,
			),
			(
				vtable_slot!(
					sys::IRecipientFilter__bindgen_vtable,
					IRecipientFilter_GetRecipientIndex
				),
				filter_index as *mut c_void,
			),
		]);

		Box::new(Self {
			vtable,
			clients,
			reliable,
		})
	}

	fn ptr(&mut self) -> *mut sys::IRecipientFilter {
		ptr::from_mut(self).cast()
	}
}

/// Something the engine or the callback noticed.
#[derive(Debug, Clone, PartialEq)]
enum Seen {
	/// The callback saw the table, sent to these clients, reliably or not, with
	/// the delay and the class index.
	Callback(String, Vec<c_int>, bool, f32, c_int),

	/// The engine queued a temporary entity of the table.
	Queued(String),
}

#[test]
fn explosions_are_blocked_and_others_reach_the_engine() {
	on_both(|harness| {
		let api = harness.api();
		let mut engine = Engine::new();
		let mut filter = Filter::new(vec![1, 3, 4], false);

		// SAFETY: The mock interface has `PlaybackTempEntity` at the slot, and is
		// leaked with its vtable.
		unsafe {
			api.install_playback_temp_entity(
				engine.ptr(),
				tf2_binding(no_interfaces),
				block_explosions,
			)
		}
		.unwrap();

		// A second hook is refused, so that each temporary entity is decided once.
		assert!(matches!(
			// SAFETY: As above.
			unsafe {
				api.install_playback_temp_entity(
					engine.ptr(),
					tf2_binding(no_interfaces),
					block_explosions,
				)
			},
			Err(HookError::AlreadyInstalled)
		));

		assert_eq!(
			play(harness, &mut engine, &mut filter, c"DT_TETFExplosion", 0.0),
			[Seen::Callback(
				"DT_TETFExplosion".to_owned(),
				vec![1, 3, 4],
				false,
				0.0,
				11
			)]
		);

		assert_eq!(
			play(harness, &mut engine, &mut filter, c"DT_TEFireBullets", 0.25),
			[
				Seen::Callback(
					"DT_TEFireBullets".to_owned(),
					vec![1, 3, 4],
					false,
					0.25,
					11
				),
				Seen::Queued("DT_TEFireBullets".to_owned())
			]
		);

		// A temporary entity sent to no one still reaches the callback.
		let mut nobody = Filter::new(vec![], true);

		assert_eq!(
			play(harness, &mut engine, &mut nobody, c"DT_TEBloodStream", 0.0),
			[
				Seen::Callback("DT_TEBloodStream".to_owned(), vec![], true, 0.0, 11),
				Seen::Queued("DT_TEBloodStream".to_owned())
			]
		);
	});
}

#[test]
fn temp_entities_an_earlier_hook_blocked_skip_the_callback() {
	on_both(|harness| {
		let api = harness.api();
		let mut engine = Engine::new();
		let mut filter = Filter::new(vec![2], false);

		fn block(_call: &HookCall<'_, PlaybackTempEntity>) -> HookAction<()> {
			HookAction::Supersede(())
		}

		// SAFETY: As above.
		unsafe {
			api.add_hook(
				PLAYBACK_TEMP_ENTITY,
				HookTarget::instance(engine.ptr()),
				HookTiming::Pre,
				&block,
			)
			.unwrap();
			api.install_playback_temp_entity(
				engine.ptr(),
				tf2_binding(no_interfaces),
				block_explosions,
			)
			.unwrap();
		}

		assert_eq!(
			play(harness, &mut engine, &mut filter, c"DT_TEFireBullets", 0.0),
			[]
		);
	});
}

/// The callback, which notes what it saw and blocks explosions.
fn block_explosions(_server: Server<'_>, played: &PlayedTempEntity<'_>) -> TempEntityAction {
	let table = played.table.to_str().unwrap();

	SEEN.with_borrow_mut(|seen| {
		seen.push(Seen::Callback(
			table.to_owned(),
			played.recipients.iter().collect(),
			played.recipients.is_reliable(),
			played.delay,
			played.class_id,
		));
	});

	match table == "DT_TETFExplosion" {
		true => TempEntityAction::Block,
		false => TempEntityAction::Continue,
	}
}

/// Plays back a temporary entity of a table named `name`, of server class 11,
/// to `filter`'s clients after `delay`, through the engine's hooked vtable,
/// and returns what happened.
fn play(
	harness: &Harness,
	engine: &mut Engine,
	filter: &mut Filter,
	name: &'static CStr,
	delay: f32,
) -> Vec<Seen> {
	// SAFETY: Tables are plain data.
	let mut table: sys::SendTable = unsafe { zeroed() };
	let sender = 0_u64;

	table.m_pNetTableName = name.as_ptr();
	SEEN.take();

	harness.call::<PlaybackTempEntity>(
		engine.ptr().as_ptr(),
		PLAYBACK_TEMP_ENTITY_SLOT,
		(
			filter.ptr(),
			delay,
			ptr::from_ref(&sender).cast::<c_void>(),
			ptr::from_ref(&table),
			11,
		),
	);

	SEEN.take()
}

/// The mock engine's `PlaybackTempEntity`, which notes the table's name.
unsafe extern "C" fn engine_playback(
	_this: *mut sys::IVEngineServer,
	_filter: *mut sys::IRecipientFilter,
	_delay: f32,
	_sender: *const c_void,
	table: *const sys::SendTable,
	_class_id: c_int,
) {
	// SAFETY: The tests pass live tables with terminated names.
	let name = unsafe { CStr::from_ptr((*table).m_pNetTableName.cast::<c_char>()) }
		.to_str()
		.unwrap()
		.to_owned();

	SEEN.with_borrow_mut(|seen| seen.push(Seen::Queued(name)));
}

/// The mock filter's `GetRecipientCount`.
unsafe extern "C" fn filter_count(this: *const sys::IRecipientFilter) -> c_int {
	// SAFETY: Only mock filters have this vtable.
	unsafe { (*this.cast::<Filter>()).clients.len() as c_int }
}

/// The mock filter's `GetRecipientIndex`.
unsafe extern "C" fn filter_index(this: *const sys::IRecipientFilter, slot: c_int) -> c_int {
	// SAFETY: As above.
	unsafe { (&(*this.cast::<Filter>()).clients)[slot as usize] }
}

/// The mock filter's `IsReliable`.
unsafe extern "C" fn filter_reliable(this: *const sys::IRecipientFilter) -> bool {
	// SAFETY: As above.
	unsafe { (*this.cast::<Filter>()).reliable }
}

/// A leaked vtable of `V`'s size, whose slots hold `functions` at their
/// indices, and [`unexpected_call`] elsewhere.
fn mock_vtable<V>(functions: &[(usize, *mut c_void)]) -> *mut *mut c_void {
	let mut slots = vec![unexpected_call as *mut c_void; size_of::<V>() / VTABLE_SLOT_SIZE];

	for &(slot, function) in functions {
		slots[slot] = function;
	}

	Vec::leak(slots).as_mut_ptr()
}

/// A slot no test expects to be called, which aborts the test process.
extern "C" fn unexpected_call() {
	panic!("unexpected virtual call");
}
