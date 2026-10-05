//! Tests of `crate::sound_hooks`: pre hooks of `EmitSound` on mock engine
//! sound interfaces, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::raw::abi::VTABLE_SLOT_SIZE;
use source_sdk_2013::raw::vtable_slot;
use std::cell::RefCell;
use std::ffi::{c_char, c_void};
use std::ptr;

thread_local! {
	/// What happened during the emissions since the last [`emit`], in order.
	static SEEN: RefCell<Vec<Seen>> = const { RefCell::new(Vec::new()) };
}

/// An engine sound interface of a C++ class, as far as hooks know it.
#[repr(C)]
struct Engine {
	vtable: *mut *mut c_void,
}

impl Engine {
	fn new() -> Box<Self> {
		let vtable = mock_vtable::<sys::IEngineSound__bindgen_vtable>(&[(
			EMIT_SOUND_SLOT,
			engine_emit as *mut c_void,
		)]);

		Box::new(Self { vtable })
	}

	fn ptr(&mut self) -> NonNull<sys::IEngineSound> {
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
	/// The callback saw the sample, sent to these clients, reliably or not,
	/// from the entity, at the origin.
	Callback(String, Vec<c_int>, bool, c_int, Option<Vector>),

	/// The engine emitted the sample.
	Emitted(String),
}

#[test]
fn announcer_samples_are_blocked_and_others_reach_the_engine() {
	on_both(|harness| {
		let api = harness.api();
		let mut engine = Engine::new();
		let mut filter = Filter::new(vec![1, 3, 4], true);
		let origin = sys::Vector {
			x: 1.0,
			y: -2.0,
			z: 3.5,
		};

		// SAFETY: The mock interface has `EmitSound` at the slot, and is leaked
		// with its vtable.
		unsafe {
			api.install_emit_sound(engine.ptr(), tf2_binding(no_interfaces), block_announcer)
		}
		.unwrap();

		// A second hook is refused, so that each sound is decided once.
		assert!(matches!(
			// SAFETY: As above.
			unsafe {
				api.install_emit_sound(engine.ptr(), tf2_binding(no_interfaces), block_announcer)
			},
			Err(HookError::AlreadyInstalled)
		));

		assert_eq!(
			emit(
				harness,
				&mut engine,
				&mut filter,
				c"*vo/announcer_begins_5sec.mp3",
				None
			),
			[Seen::Callback(
				"*vo/announcer_begins_5sec.mp3".to_owned(),
				vec![1, 3, 4],
				true,
				7,
				None
			)]
		);

		assert_eq!(
			emit(
				harness,
				&mut engine,
				&mut filter,
				c")weapons/shotgun_shoot.wav",
				Some(&origin)
			),
			[
				Seen::Callback(
					")weapons/shotgun_shoot.wav".to_owned(),
					vec![1, 3, 4],
					true,
					7,
					Some(origin.into())
				),
				Seen::Emitted(")weapons/shotgun_shoot.wav".to_owned())
			]
		);

		// A sound sent to no one still reaches the callback.
		let mut nobody = Filter::new(vec![], false);

		assert_eq!(
			emit(harness, &mut engine, &mut nobody, c"vo/null.mp3", None),
			[
				Seen::Callback("vo/null.mp3".to_owned(), vec![], false, 7, None),
				Seen::Emitted("vo/null.mp3".to_owned())
			]
		);
	});
}

/// The callback, which notes what it saw and blocks the announcer's samples.
fn block_announcer(_server: Server<'_>, sound: &EmittedSound<'_>) -> EmitSoundAction {
	let sample = sound.sample.to_str().unwrap();

	SEEN.with_borrow_mut(|seen| {
		seen.push(Seen::Callback(
			sample.to_owned(),
			sound.recipients.iter().collect(),
			sound.recipients.is_reliable(),
			sound.entity,
			sound.origin,
		));
	});

	match sample
		.trim_start_matches(['*', ')'])
		.starts_with("vo/announcer")
	{
		true => EmitSoundAction::Block,
		false => EmitSoundAction::Continue,
	}
}

/// Emits `sample` from entity 7 to `filter`'s clients, at `origin`, through
/// the engine's hooked vtable, and returns what happened.
fn emit(
	harness: &Harness,
	engine: &mut Engine,
	filter: &mut Filter,
	sample: &CStr,
	origin: Option<&sys::Vector>,
) -> Vec<Seen> {
	SEEN.take();

	harness.call::<EmitSound>(
		engine.ptr().as_ptr(),
		EMIT_SOUND_SLOT,
		(
			filter.ptr(),
			7,
			0,
			sample.as_ptr(),
			1.0,
			75,
			0,
			100,
			0,
			origin.map_or(ptr::null(), ptr::from_ref),
			ptr::null(),
			ptr::null_mut(),
			true,
			0.0,
			-1,
		),
	);

	SEEN.take()
}

/// The mock engine's `EmitSound`, which notes the sample.
unsafe extern "C" fn engine_emit(
	_this: *mut sys::IEngineSound,
	_filter: *mut sys::IRecipientFilter,
	_entity: c_int,
	_channel: c_int,
	sample: *const c_char,
	_volume: f32,
	_level: sys::soundlevel_t,
	_flags: c_int,
	_pitch: c_int,
	_special_dsp: c_int,
	_origin: *const sys::Vector,
	_direction: *const sys::Vector,
	_origins: *mut sys::CUtlVector<sys::Vector, sys::CUtlMemory<sys::Vector>>,
	_update_positions: bool,
	_sound_time: f32,
	_speaker: c_int,
) {
	// SAFETY: The tests emit terminated samples.
	let sample = unsafe { CStr::from_ptr(sample) }
		.to_str()
		.unwrap()
		.to_owned();

	SEEN.with_borrow_mut(|seen| seen.push(Seen::Emitted(sample)));
}

/// The mock filter's `GetRecipientCount`.
unsafe extern "C" fn filter_count(this: *const sys::IRecipientFilter) -> c_int {
	// SAFETY: As above.
	unsafe { (*this.cast::<Filter>()).clients.len() as c_int }
}

/// The mock filter's `GetRecipientIndex`.
unsafe extern "C" fn filter_index(this: *const sys::IRecipientFilter, slot: c_int) -> c_int {
	// SAFETY: As above.
	unsafe { (&(*this.cast::<Filter>()).clients)[slot as usize] }
}

/// The mock filter's `IsReliable`.
unsafe extern "C" fn filter_reliable(this: *const sys::IRecipientFilter) -> bool {
	// SAFETY: Only mock filters have this vtable.
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

#[test]
fn sounds_an_earlier_hook_blocked_skip_the_callback() {
	on_both(|harness| {
		let api = harness.api();
		let mut engine = Engine::new();
		let mut filter = Filter::new(vec![2], false);

		fn block(_call: &HookCall<'_, EmitSound>) -> HookAction<()> {
			HookAction::Supersede(())
		}

		// SAFETY: As above.
		unsafe {
			api.add_hook(
				EMIT_SOUND,
				HookTarget::instance(engine.ptr()),
				HookTiming::Pre,
				&block,
			)
			.unwrap();
			api.install_emit_sound(engine.ptr(), tf2_binding(no_interfaces), block_announcer)
				.unwrap();
		}

		assert_eq!(
			emit(
				harness,
				&mut engine,
				&mut filter,
				c"weapons/shotgun_shoot.wav",
				None
			),
			[]
		);
	});
}

/// A slot no test expects to be called, which aborts the test process.
extern "C" fn unexpected_call() {
	panic!("unexpected virtual call");
}
