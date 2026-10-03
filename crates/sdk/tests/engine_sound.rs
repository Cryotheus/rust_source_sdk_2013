//! Tests of what the engine's sound system (`IEngineSound`) answers.

use sdk_raw::test_support::{mock_vtable, unexpected_call};
use source_sdk_2013::Module;
use source_sdk_2013::interfaces::EngineSound;
use source_sdk_2013::test_support::leak;
use source_sdk_2013::test_support::server::{export, mock_server};
use std::cell::Cell;
use std::ffi::c_char;

thread_local! {
	/// What `GetSoundDuration` returns.
	static DURATION: Cell<f32> = const { Cell::new(0.0) };
}

/// `IEngineSound::GetSoundDuration`, which returns [`DURATION`].
unsafe extern "C" fn sound_duration(_: *mut sys::IEngineSound, _: *const c_char) -> f32 {
	DURATION.get()
}

#[test]
fn sound_durations_must_be_positive() {
	// SAFETY: The vtable holds only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and the patch only writes a slot of the
	// vtable being built.
	let vtable = unsafe {
		mock_vtable::<sys::IEngineSound__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IEngineSound_GetSoundDuration).write(sound_duration);
		})
	};

	export(
		Module::Engine,
		EngineSound::VERSION,
		leak(sys::IEngineSound {
			vtable_: Box::leak(vtable),
		}),
	);

	let scope = ();
	let sound = mock_server(&scope).engine_sound().unwrap();

	DURATION.set(1.5);
	assert_eq!(sound.sound_duration(c"vo/a.wav"), Some(1.5));

	for unknown in [0.0, -1.0, f32::NAN] {
		DURATION.set(unknown);
		assert_eq!(sound.sound_duration(c"vo/a.mp3"), None);
	}
}
