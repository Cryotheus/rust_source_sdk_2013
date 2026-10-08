//! Tests of the sounds `EngineSound` emits and stops: the arguments the engine
//! receives, and the emissions refused before they reach it.

use super::*;
use crate::test_support::entities::{MockEntity, set_networking};
use crate::test_support::leak;
use crate::test_support::user_messages::recipients;
use sdk_raw::test_support::edicts::mock_edict;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::RefCell;
use std::ffi::{CString, c_char};
use std::ptr::NonNull;

thread_local! {
	/// Every call to `EmitSound`, in order.
	static EMITTED: RefCell<Vec<Emitted>> = const { RefCell::new(Vec::new()) };

	/// The entity, channel and sample of every call to `StopSound`, in order.
	static STOPPED: RefCell<Vec<(c_int, c_int, CString)>> = const { RefCell::new(Vec::new()) };
}

/// The arguments `EmitSound` received, with the filter's recipients as the
/// engine reads them through its vtable.
#[derive(Debug, Clone, PartialEq)]
struct Emitted {
	recipients: Vec<c_int>,
	reliable: bool,
	init_message: bool,
	entity: c_int,
	channel: c_int,
	sample: CString,
	volume: f32,
	level: sys::soundlevel_t,
	flags: c_int,
	pitch: c_int,
	special_dsp: c_int,
	origin: Option<Vector>,
	direction_is_null: bool,
	origins_is_null: bool,
	update_positions: bool,
	sound_time: f32,
	speaker: c_int,
}

/// A mock of the engine's sound system, which keeps its vtable alive.
struct MockEngineSound {
	_vtable: Box<sys::IEngineSound__bindgen_vtable>,
	interface: Box<sys::IEngineSound>,
}

impl MockEngineSound {
	/// Records `EmitSound` and `StopSound`, and fills every other slot,
	/// including the float-attenuation `EmitSound`, with a stub that fails the
	/// test if called.
	fn new() -> Self {
		// SAFETY: The vtable holds only function pointers, `unexpected_call`
		// aborts whichever slot reaches it, and the patch only writes slots of
		// the vtable being built.
		let vtable = unsafe {
			mock_vtable::<sys::IEngineSound__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IEngineSound_EmitSound1).write(emit_sound);
					(&raw mut (*vtable).IEngineSound_StopSound).write(stop_sound);
				},
			)
		};
		let interface = Box::new(sys::IEngineSound {
			vtable_: &raw const *vtable,
		});

		EMITTED.take();
		STOPPED.take();

		Self {
			_vtable: vtable,
			interface,
		}
	}

	/// The mock, as the wrappers see it.
	fn engine_sound(&mut self) -> EngineSound<'_> {
		// SAFETY: The mock outlives the borrow.
		unsafe { EngineSound::from_raw(NonNull::from(&mut *self.interface)) }
	}
}

/// A leaked edict at `index`, for mock entities to report.
fn edict(index: c_int) -> *mut sys::edict_t {
	leak(mock_edict(index, false))
}

#[test]
fn emissions_default_to_the_header_values() {
	let mut mock = MockEngineSound::new();
	let sound = mock.engine_sound();
	let mut speaker = MockEntity::new(9);

	set_networking(null_mut(), edict(9));

	let recipients = recipients(&[2], false);

	sound
		.emit_sound(
			&recipients,
			&SoundEmission::new(c"ambient/alarm.wav", SoundSource::World),
		)
		.unwrap();

	sound
		.emit_sound(
			&recipients,
			&SoundEmission {
				speaker: Some(speaker.entity()),
				flags: SoundFlags::SPEAKER,
				..SoundEmission::new(c"ambient/alarm.wav", SoundSource::World)
			},
		)
		.unwrap();

	EMITTED.with_borrow(|emitted| {
		let defaults = Emitted {
			recipients: vec![2],
			reliable: false,
			init_message: false,
			entity: SOUND_FROM_WORLD,
			channel: 0,
			sample: c"ambient/alarm.wav".to_owned(),
			volume: 1.0,
			level: sys::soundlevel_t_SNDLVL_NORM,
			flags: 0,
			pitch: 100,
			special_dsp: 0,
			origin: None,
			direction_is_null: true,
			origins_is_null: true,
			update_positions: true,
			sound_time: 0.0,
			speaker: -1,
		};

		assert_eq!(
			*emitted,
			[
				defaults.clone(),
				Emitted {
					flags: 1 << 6,
					speaker: 9,
					..defaults
				},
			]
		);
	});
}

#[test]
fn emissions_reach_the_engine_as_its_arguments() {
	let mut mock = MockEngineSound::new();
	let sound = mock.engine_sound();
	let mut source = MockEntity::new(5);

	set_networking(null_mut(), edict(5));

	let emission = SoundEmission {
		channel: Channel::VOICE,
		volume: Volume::new(0.5).unwrap(),
		level: SoundLevel::new(95),
		flags: SoundFlags::STOP_LOOPING | SoundFlags::IGNORE_PHONEMES,
		pitch: Pitch::HIGH,
		origin: Some(Vector::new(1.0, 2.0, 3.0)),
		sound_time: Some(12.5),
		..SoundEmission::new(
			c"vo/scout_thanks01.mp3",
			SoundSource::Entity(source.entity()),
		)
	};

	sound
		.emit_sound(&recipients(&[3, 1], true), &emission)
		.unwrap();

	EMITTED.with_borrow(|emitted| {
		assert_eq!(
			*emitted,
			[Emitted {
				recipients: vec![3, 1],
				reliable: true,
				init_message: false,
				entity: 5,
				channel: 2,
				sample: c"vo/scout_thanks01.mp3".to_owned(),
				volume: 0.5,
				level: 95,
				flags: (1 << 5) | (1 << 8),
				pitch: 120,
				special_dsp: 0,
				origin: Some(Vector::new(1.0, 2.0, 3.0)),
				direction_is_null: true,
				origins_is_null: true,
				update_positions: true,
				sound_time: 12.5,
				speaker: -1,
			}]
		);
	});
}

/// `IEngineSound::EmitSound`, without float attenuation, which records its
/// arguments.
unsafe extern "C" fn emit_sound(
	_: *mut sys::IEngineSound,
	filter: *mut sys::IRecipientFilter,
	entity: c_int,
	channel: c_int,
	sample: *const c_char,
	volume: f32,
	level: sys::soundlevel_t,
	flags: c_int,
	pitch: c_int,
	special_dsp: c_int,
	origin: *const sys::Vector,
	direction: *const sys::Vector,
	origins: *mut sys::CUtlVector<sys::Vector, sys::CUtlMemory<sys::Vector>>,
	update_positions: bool,
	sound_time: f32,
	speaker: c_int,
) {
	// SAFETY: The wrapper passes a live filter, sample, and origin, which
	// are read as the engine reads them.
	let emitted = unsafe {
		let vtable = (*filter).vtable_;
		let count = ((*vtable).IRecipientFilter_GetRecipientCount)(filter);

		Emitted {
			recipients: (0..count)
				.map(|slot| ((*vtable).IRecipientFilter_GetRecipientIndex)(filter, slot))
				.collect(),
			reliable: ((*vtable).IRecipientFilter_IsReliable)(filter),
			init_message: ((*vtable).IRecipientFilter_IsInitMessage)(filter),
			entity,
			channel,
			sample: CStr::from_ptr(sample).to_owned(),
			volume,
			level,
			flags,
			pitch,
			special_dsp,
			origin: origin.as_ref().map(|&origin| origin.into()),
			direction_is_null: direction.is_null(),
			origins_is_null: origins.is_null(),
			update_positions,
			sound_time,
			speaker,
		}
	};

	EMITTED.with_borrow_mut(|emitted_sounds| emitted_sounds.push(emitted));
}

#[test]
fn invalid_emissions_are_refused_before_the_engine() {
	let mut mock = MockEngineSound::new();
	let sound = mock.engine_sound();
	let mut entity = MockEntity::new(4);
	let sample = c"ui/hint.wav";
	let recipients = recipients(&[1], false);
	let emit = |emission: SoundEmission<'_>| sound.emit_sound(&recipients, &emission);

	// The entity has no edict, so the engine could not tell clients of it.
	set_networking(null_mut(), null_mut());

	assert_eq!(
		emit(SoundEmission::new(
			sample,
			SoundSource::Entity(entity.entity())
		)),
		Err(SoundError::NotNetworked)
	);
	assert_eq!(
		emit(SoundEmission {
			speaker: Some(entity.entity()),
			..SoundEmission::new(sample, SoundSource::World)
		}),
		Err(SoundError::NotNetworked)
	);
	assert_eq!(
		sound.stop_sound(SoundSource::Entity(entity.entity()), Channel::AUTO, sample),
		Err(SoundError::NotNetworked)
	);

	EMITTED.with_borrow(|emitted| assert!(emitted.is_empty()));
	STOPPED.with_borrow(|stopped| assert!(stopped.is_empty()));
}

/// `IEngineSound::StopSound`, which records its arguments.
unsafe extern "C" fn stop_sound(
	_: *mut sys::IEngineSound,
	entity: c_int,
	channel: c_int,
	sample: *const c_char,
) {
	// SAFETY: The wrapper passes a live sample.
	let sample = unsafe { CStr::from_ptr(sample) }.to_owned();

	STOPPED.with_borrow_mut(|stopped| stopped.push((entity, channel, sample)));
}

#[test]
fn stops_reach_the_engine() {
	let mut mock = MockEngineSound::new();
	let sound = mock.engine_sound();
	let mut entity = MockEntity::new(7);

	set_networking(null_mut(), edict(7));

	sound
		.stop_sound(
			SoundSource::Entity(entity.entity()),
			Channel::STATIC,
			c"music/loop.wav",
		)
		.unwrap();
	sound
		.stop_sound(SoundSource::World, Channel::VOICE, c"ambient/alarm.wav")
		.unwrap();

	STOPPED.with_borrow(|stopped| {
		assert_eq!(
			*stopped,
			[
				(7, 6, c"music/loop.wav".to_owned()),
				(SOUND_FROM_WORLD, 2, c"ambient/alarm.wav".to_owned()),
			]
		);
	});
}
