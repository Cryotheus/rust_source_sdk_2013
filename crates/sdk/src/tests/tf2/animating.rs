//! Tests of `crate::tf2::animating`: attachment lookups through an animating
//! entity's native member, and a fake animating entity's other members.

use super::*;
use crate::test_support::entities::{MockEntity, set_datamap, state_maps};
use crate::test_support::leak;
use crate::test_support::server::{mock_server, null_server};

use crate::test_support::tf2::script_binding::{
	SCRIPT_DESCRIPTION_SLOT, class_description, member_binding, script_description,
	set_script_description,
};

use sdk_raw::test_support::{mock_vtable, unexpected_call};
use sdk_raw::tf2::script_binding::{STRING, boolean};
use std::cell::{Cell, RefCell};
use std::ffi::{CString, c_char, c_void};
use std::ptr::{NonNull, null_mut};

/// A native method of the fake script class: its name, return type, and
/// parameter types.
type Method = (
	&'static CStr,
	sys::ScriptDataType_t,
	&'static [sys::ScriptDataType_t],
);

/// The members the fake `CBaseAnimating` script class declares, as the
/// game's does.
const ANIMATING_METHODS: &[Method] = &[
	(c"FindBodygroupByName", INT, &[STRING]),
	(c"GetAttachmentBone", INT, &[INT]),
	(c"GetBodygroup", INT, &[INT]),
	(c"GetBodygroupName", STRING, &[INT]),
	(c"GetBodygroupPartName", STRING, &[INT, INT]),
	(c"GetCycle", FLOAT, &[]),
	(c"GetModelScale", FLOAT, &[]),
	(c"GetPlaybackRate", FLOAT, &[]),
	(c"GetSequence", INT, &[]),
	(c"GetSequenceActivityName", STRING, &[INT]),
	(c"GetSequenceDuration", FLOAT, &[INT]),
	(c"GetSequenceName", STRING, &[INT]),
	(c"GetSkin", INT, &[]),
	(c"IsSequenceFinished", BOOL, &[]),
	(c"LookupActivity", INT, &[STRING]),
	(c"LookupBone", INT, &[STRING]),
	(c"LookupPoseParameter", INT, &[STRING]),
	(c"LookupSequence", INT, &[STRING]),
	(c"ResetSequence", VOID, &[INT]),
	(c"SetBodygroup", VOID, &[INT, INT]),
	(c"SetCycle", VOID, &[FLOAT]),
	(c"SetModelScale", VOID, &[FLOAT, FLOAT]),
	(c"SetModelSimple", VOID, &[STRING]),
	(c"SetPlaybackRate", VOID, &[FLOAT]),
	(c"SetPoseParameter", FLOAT, &[INT, FLOAT]),
	(c"SetSequence", VOID, &[INT]),
	(c"SetSkin", VOID, &[INT]),
	(c"StopAnimation", VOID, &[]),
	(c"StudioFrameAdvance", VOID, &[]),
	(c"StudioFrameAdvanceManual", VOID, &[FLOAT]),
];

/// The fake model's sequences: their names, the activities they play, and
/// their durations.
const SEQUENCES: [(&CStr, &CStr, f32); 2] = [
	(c"stand_MELEE", c"ACT_MP_STAND_MELEE", 2.0),
	(c"run_MELEE", c"ACT_MP_RUN_MELEE", 0.5),
];

/// The number the fake game gives the activity of the first of
/// [`SEQUENCES`], and the next numbers to the next.
const FIRST_ACTIVITY: c_int = 100;

/// The fake model's bones.
const BONES: [&CStr; 2] = [c"bip_pelvis", c"bip_head"];

/// The fake model's pose parameters, which it clamps to -1 to 1.
const POSE_PARAMETERS: [&CStr; 2] = [c"move_x", c"move_y"];

/// The fake model's bodygroups, and the names of their parts.
const BODYGROUPS: [(&CStr, &[&CStr]); 2] = [
	(c"hat", &[c"hat_none", c"hat_beanie"]),
	(c"backpack", &[c"backpack"]),
];

/// The model the mock model info knows as precached.
const MODEL: &CStr = c"models/bots/headless_hatman.mdl";

thread_local! {
	/// The index the adapter returns.
	static INDEX: Cell<c_int> = const { Cell::new(0) };

	/// The entities and names the adapter was called with.
	static LOOKED_UP: RefCell<Vec<(*mut c_void, CString)>> = const { RefCell::new(Vec::new()) };

	/// Whether the adapter reports failure.
	static REJECTS: Cell<bool> = const { Cell::new(false) };

	/// What the fake animating entity's members read and write.
	static STATE: RefCell<FakeState> = RefCell::new(FakeState::default());
}

/// The fields of the fake animating entity that its native members read and
/// write.
#[derive(Debug, Default)]
struct FakeState {
	sequence: c_int,
	/// The sequences `ResetSequence` was called with.
	resets: Vec<c_int>,
	cycle: f32,
	rate: f32,
	finished: bool,
	/// The parts the bodygroups show.
	bodygroups: [c_int; 2],
	poses: [f32; 2],
	scale: f32,
	/// The durations `SetModelScale` was called with.
	scale_durations: Vec<f32>,
	skin: c_int,
	/// The intervals the animation was advanced by, `None` for one the game
	/// works out.
	advances: Vec<Option<f32>>,
	/// The models `SetModelSimple` was called with.
	models: Vec<CString>,
	/// Whether every member refuses its call, as an adapter does when its
	/// arguments do not convert, without running the member.
	rejects: bool,
	/// The calls that reached a member, refused or not.
	calls: usize,
}

/// Reads the fake state.
fn state<T>(read: impl FnOnce(&FakeState) -> T) -> T {
	STATE.with_borrow(read)
}

/// The index of `name` among `names`, or -1, as the game's lookups return.
///
/// # Safety
///
/// `name` must be NUL-terminated.
unsafe fn index_of<'a>(names: impl IntoIterator<Item = &'a CStr>, name: *const c_char) -> c_int {
	// SAFETY: As the caller promises.
	let name = unsafe { CStr::from_ptr(name) };

	names
		.into_iter()
		.position(|candidate| candidate == name)
		.map_or(-1, |index| c_int::try_from(index).unwrap())
}

/// A fake binding's adapter, which runs the fake member named by the
/// binding's function, on the fake state.
unsafe extern "C" fn adapter(
	function: sys::ScriptFunctionBindingStorageType_t,
	_: *mut c_void,
	arguments: *mut sys::ScriptVariant_t,
	count: c_int,
	result: *mut sys::ScriptVariant_t,
) -> bool {
	// SAFETY: Every fake binding stores the address of its member's name, a
	// static C string, as its function.
	let name = unsafe { CStr::from_ptr(function.val_0 as *const c_char) };

	let argument = |index: usize| {
		assert!(index < count as usize);

		// SAFETY: `call` checked the arguments' types against the binding, and
		// passes `count` of them.
		unsafe { &(*arguments.add(index)).__bindgen_anon_1 }
	};

	let sequence_name =
		|sequence: c_int, name: fn(&(&'static CStr, &'static CStr, f32)) -> &'static CStr| {
			usize::try_from(sequence)
				.ok()
				.and_then(|sequence| SEQUENCES.get(sequence))
				.map_or(c"Unknown", name)
		};

	STATE.with_borrow_mut(|state| {
		state.calls += 1;

		if state.rejects {
			return false;
		}

		// SAFETY: Each member reads the union member of its declared parameter
		// types, which `call` checked, and the fake model's names are static.
		let value = unsafe {
			match name.to_bytes() {
				b"FindBodygroupByName" => Some(int(index_of(
					BODYGROUPS.map(|(group, _)| group),
					argument(0).m_pszString,
				))),

				b"GetAttachmentBone" => Some(int(argument(0).m_int * 10)),
				b"GetBodygroup" => Some(int(state.bodygroups[argument(0).m_int as usize])),

				b"GetBodygroupName" => Some(string(
					BODYGROUPS
						.get(argument(0).m_int as usize)
						.map_or(c"", |(group, _)| group),
				)),

				b"GetBodygroupPartName" => Some(string(
					BODYGROUPS
						.get(argument(0).m_int as usize)
						.and_then(|(_, parts)| parts.get(argument(1).m_int as usize))
						.copied()
						.unwrap_or(c""),
				)),

				b"GetCycle" => Some(float(state.cycle)),
				b"GetModelScale" => Some(float(state.scale)),
				b"GetPlaybackRate" => Some(float(state.rate)),
				b"GetSequence" => Some(int(state.sequence)),

				b"GetSequenceActivityName" => Some(string(sequence_name(
					argument(0).m_int,
					|(_, activity, _)| activity,
				))),

				b"GetSequenceDuration" => Some(float(SEQUENCES[argument(0).m_int as usize].2)),

				b"GetSequenceName" => {
					Some(string(sequence_name(argument(0).m_int, |(name, ..)| name)))
				}

				b"GetSkin" => Some(int(state.skin)),
				b"IsSequenceFinished" => Some(boolean(state.finished)),

				b"LookupActivity" => Some(int(
					match index_of(
						SEQUENCES.map(|(_, activity, _)| activity),
						argument(0).m_pszString,
					) {
						-1 => -1,
						index => FIRST_ACTIVITY + index,
					},
				)),

				b"LookupBone" => Some(int(index_of(BONES, argument(0).m_pszString))),

				b"LookupPoseParameter" => {
					Some(int(index_of(POSE_PARAMETERS, argument(0).m_pszString)))
				}

				// By sequence name, then by the name of an activity.
				b"LookupSequence" => Some(int(
					match index_of(SEQUENCES.map(|(name, ..)| name), argument(0).m_pszString) {
						-1 => index_of(
							SEQUENCES.map(|(_, activity, _)| activity),
							argument(0).m_pszString,
						),
						index => index,
					},
				)),

				b"ResetSequence" => {
					state.sequence = argument(0).m_int;
					state.resets.push(state.sequence);
					state.rate = 1.0;
					state.finished = false;
					None
				}

				b"SetBodygroup" => {
					state.bodygroups[argument(0).m_int as usize] = argument(1).m_int;
					None
				}

				b"SetCycle" => {
					state.cycle = argument(0).m_float;
					None
				}

				b"SetModelScale" => {
					state.scale = argument(0).m_float;
					state.scale_durations.push(argument(1).m_float);
					None
				}

				b"SetModelSimple" => {
					state
						.models
						.push(CStr::from_ptr(argument(0).m_pszString).to_owned());

					None
				}

				b"SetPlaybackRate" => {
					state.rate = argument(0).m_float;
					None
				}

				b"SetPoseParameter" => {
					let value = argument(1).m_float.clamp(-1.0, 1.0);

					state.poses[argument(0).m_int as usize] = value;
					Some(float(value))
				}

				b"SetSequence" => {
					state.sequence = argument(0).m_int;
					None
				}

				b"SetSkin" => {
					state.skin = argument(0).m_int;
					None
				}

				b"StopAnimation" => {
					state.rate = 0.0;
					None
				}

				b"StudioFrameAdvance" => {
					state.advances.push(None);
					None
				}

				b"StudioFrameAdvanceManual" => {
					state.advances.push(Some(argument(0).m_float));
					None
				}

				other => unreachable!("no fake member {other:?}"),
			}
		};

		if let Some(value) = value {
			// SAFETY: The binding declares a result, so the caller passes a
			// writable one.
			unsafe { result.write(value) };
		}

		true
	})
}

/// The leaked descriptors of the fake `CBaseAnimating` script class,
/// declaring [`ANIMATING_METHODS`] through [`adapter`], and of
/// `CDynamicProp`, which derives from it.
fn prop_description() -> *mut sys::ScriptClassDesc_t {
	let bindings = ANIMATING_METHODS
		.iter()
		.map(|&(method, returns, parameters)| {
			let mut binding =
				member_binding(method, returns, Vec::from(parameters).leak(), Some(adapter));

			binding.m_pFunction.val_0 = method.as_ptr() as isize;
			binding
		})
		.collect::<Vec<_>>();

	let animating = leak(class_description(
		c"CBaseAnimating",
		bindings.leak(),
		null_mut(),
	));

	leak(class_description(c"CDynamicProp", &mut [], animating))
}

/// A mock entity whose data maps are a `CBaseAnimating`'s and whose script
/// descriptors are the fake prop's, wrapped, with the vtable it uses, which
/// must outlive it.
fn mock_animating<'s>(
	server: Server<'s>,
	mock: &mut MockEntity,
) -> (Animating<'s>, Vec<*const ()>) {
	let (entity, vtable) = described(mock);

	set_datamap(state_maps(vec![(c"CBaseAnimating", Vec::new())]));
	set_script_description(prop_description());

	(Animating::new(server, entity).unwrap(), vtable)
}

/// `IVModelInfo::GetModelIndex`, which knows only [`MODEL`] as precached.
unsafe extern "C" fn model_index(_: *const sys::IVModelInfo, name: *const c_char) -> c_int {
	// SAFETY: The wrapper passes a NUL-terminated name.
	if unsafe { CStr::from_ptr(name) } == MODEL {
		5
	} else {
		-1
	}
}

/// Mock model info, which knows only [`MODEL`] as precached.
fn models() -> ModelInfo<'static> {
	// SAFETY: The vtable holds only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and the patch only writes a slot of the vtable
	// being built.
	let vtable = unsafe {
		mock_vtable::<sys::IVModelInfo__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IVModelInfo_GetModelIndex).write(model_index);
		})
	};
	let raw = leak(sys::IVModelInfo {
		vtable_: Box::leak(vtable),
	});

	// SAFETY: The model info and its vtable are leaked, and the vtable answers
	// what setting a model calls.
	unsafe { ModelInfo::from_raw(NonNull::new(raw).unwrap()) }
}

/// A mock entity whose `GetScriptDesc` returns the descriptor
/// [`set_script_description`] sets, for as long as the mocks live, with the
/// vtable it then uses, which must outlive it.
fn described(mock: &mut MockEntity) -> (Entity<'static>, Vec<*const ()>) {
	let pointer = mock.as_ptr();
	let mut vtable = vec![std::ptr::null::<()>(); SCRIPT_DESCRIPTION_SLOT + 1];

	// The mock's own vtable answers every slot before the descriptor's, which
	// include the datamap's.
	// SAFETY: A mock entity starts with the pointer to its vtable, which has
	// slots up to TF2's `Teleport`, past `GetScriptDesc`.
	unsafe {
		let original = pointer.cast::<*const *const ()>().read();

		for (slot, entry) in vtable.iter_mut().enumerate().take(SCRIPT_DESCRIPTION_SLOT) {
			*entry = original.add(slot).read();
		}
	}

	vtable[SCRIPT_DESCRIPTION_SLOT] = script_description as *const ();
	// SAFETY: A mock entity starts with the pointer to its vtable, and the
	// caller keeps the new vtable alive while the entity is used.
	unsafe { pointer.cast::<*const *const ()>().write(vtable.as_ptr()) };

	// SAFETY: Mock entities are leaked, and their vtable answers what the
	// lookup calls of an entity.
	(
		unsafe { Entity::from_raw(NonNull::new(pointer).unwrap()) },
		vtable,
	)
}

/// The adapter of `CBaseAnimating::LookupAttachment`, which records the
/// entity and the name, and returns [`INDEX`], unless [`REJECTS`] is set.
unsafe extern "C" fn lookup_adapter(
	_: sys::ScriptFunctionBindingStorageType_t,
	object: *mut c_void,
	arguments: *mut sys::ScriptVariant_t,
	count: c_int,
	result: *mut sys::ScriptVariant_t,
) -> bool {
	assert_eq!(count, 1);

	if REJECTS.get() {
		return false;
	}

	// SAFETY: The binding declares one string parameter, so the caller passes
	// one string variant, NUL-terminated.
	let name = unsafe { CStr::from_ptr((*arguments).__bindgen_anon_1.m_pszString) };

	LOOKED_UP.with_borrow_mut(|looked_up| looked_up.push((object, name.to_owned())));
	// SAFETY: The binding returns an integer, so the caller passes a writable
	// result.
	unsafe { result.write(sdk_raw::tf2::script_binding::int(INDEX.get())) };
	true
}

#[test]
fn attachments_are_looked_up_through_the_animating_member() {
	let scope = ();
	let server = mock_server(&scope);
	let mut parameters = [STRING];
	let mut bindings = [member_binding(
		c"LookupAttachment",
		binding::INT,
		&mut parameters,
		Some(lookup_adapter),
	)];
	let mut animating = class_description(c"CBaseAnimating", &mut bindings, null_mut());

	// Changed below only through the pointer `call` reads it by.
	let binding = animating.m_FunctionBindings.m_Memory.m_pMemory;
	let mut prop = class_description(c"CDynamicProp", &mut [], &raw mut animating);
	let mut mock = MockEntity::new(1);
	let (entity, _vtable) = described(&mut mock);

	set_script_description(&raw mut prop);
	LOOKED_UP.take();

	INDEX.set(3);
	assert_eq!(lookup_attachment(server, entity, c"head"), Ok(Some(3)));
	assert_eq!(
		LOOKED_UP.take(),
		[(entity.as_ptr().cast(), c"head".to_owned())]
	);

	// The game's 0 is no attachment of the name, or no model.
	INDEX.set(0);
	assert_eq!(lookup_attachment(server, entity, c"head"), Ok(None));

	INDEX.set(256);
	assert_eq!(
		lookup_attachment(server, entity, c"head"),
		Err(AttachmentError::OutOfRange(256))
	);
	INDEX.set(-1);
	assert_eq!(
		lookup_attachment(server, entity, c"head"),
		Err(AttachmentError::OutOfRange(-1))
	);

	REJECTS.set(true);
	assert_eq!(
		lookup_attachment(server, entity, c"head"),
		Err(AttachmentError::Rejected)
	);
	REJECTS.set(false);

	// An entity marked for deletion is not used.
	mock.set_eflags(1);
	assert_eq!(
		lookup_attachment(server, entity, c"head"),
		Err(AttachmentError::MarkedForDeletion)
	);
	mock.set_eflags(0);
	LOOKED_UP.take();

	// Another signature is refused before the call.
	// SAFETY: `binding` points to the binding in `bindings`, which is alive,
	// and which the lookup only reads through the same pointer.
	unsafe { (*binding).m_desc.m_ReturnType = binding::FLOAT };
	assert_eq!(
		lookup_attachment(server, entity, c"head"),
		Err(AttachmentError::UnsupportedMethod)
	);
	assert!(LOOKED_UP.take().is_empty());
	// SAFETY: As above.
	unsafe { (*binding).m_desc.m_ReturnType = binding::INT };

	// An entity that is not animating has no such member.
	let mut entity_description = class_description(c"CBaseEntity", &mut [], null_mut());

	set_script_description(&raw mut entity_description);
	assert_eq!(
		lookup_attachment(server, entity, c"head"),
		Err(AttachmentError::UnsupportedMethod)
	);
	assert!(LOOKED_UP.take().is_empty());
	set_script_description(null_mut());
}

#[test]
fn lookups_find_the_models_names_or_nothing() {
	let scope = ();
	let mut mock = MockEntity::new(1);
	let (animating, _vtable) = mock_animating(mock_server(&scope), &mut mock);

	assert_eq!(animating.lookup_sequence(c"run_MELEE"), Ok(Some(1)));
	// An activity's name finds a sequence that plays it.
	assert_eq!(
		animating.lookup_sequence(c"ACT_MP_STAND_MELEE"),
		Ok(Some(0))
	);
	assert_eq!(animating.lookup_sequence(c"taunt01"), Ok(None));
	assert_eq!(
		animating.lookup_activity(c"ACT_MP_RUN_MELEE"),
		Ok(Some(FIRST_ACTIVITY + 1))
	);
	assert_eq!(animating.lookup_activity(c"run_MELEE"), Ok(None));
	assert_eq!(animating.lookup_bone(c"bip_head"), Ok(Some(1)));
	assert_eq!(animating.lookup_bone(c"bip_tail"), Ok(None));
	assert_eq!(animating.lookup_pose_parameter(c"move_y"), Ok(Some(1)));
	assert_eq!(animating.lookup_pose_parameter(c"body_yaw"), Ok(None));
	assert_eq!(animating.find_bodygroup_by_name(c"backpack"), Ok(Some(1)));
	assert_eq!(animating.find_bodygroup_by_name(c"medal"), Ok(None));
	assert_eq!(animating.attachment_bone(2), Ok(20));

	assert_eq!(
		animating.sequence_name(1),
		Ok(Some(c"run_MELEE".to_owned()))
	);
	assert_eq!(animating.sequence_name(7), Ok(Some(c"Unknown".to_owned())));
	assert_eq!(
		animating.sequence_activity_name(0),
		Ok(Some(c"ACT_MP_STAND_MELEE".to_owned()))
	);
	assert_eq!(animating.sequence_duration(1), Ok(0.5));
	assert_eq!(animating.bodygroup_name(0), Ok(Some(c"hat".to_owned())));
	assert_eq!(animating.bodygroup_name(2), Ok(Some(c"".to_owned())));
	assert_eq!(
		animating.bodygroup_part_name(0, 1),
		Ok(Some(c"hat_beanie".to_owned()))
	);
	assert_eq!(animating.entity().as_ptr(), mock.as_ptr());
}

#[test]
fn sequences_play_and_advance_through_the_members() {
	let scope = ();
	let mut mock = MockEntity::new(1);
	let (animating, _vtable) = mock_animating(mock_server(&scope), &mut mock);

	animating.set_sequence(1).unwrap();
	assert_eq!(animating.sequence(), Ok(1));
	assert!(state(|state| state.resets.is_empty()));

	animating.set_playback_rate(2.0).unwrap();
	assert_eq!(animating.playback_rate(), Ok(2.0));
	animating.reset_sequence(0).unwrap();
	assert_eq!(state(|state| state.resets.clone()), [0]);
	assert_eq!(animating.sequence(), Ok(0));
	assert_eq!(animating.playback_rate(), Ok(1.0));

	animating.set_cycle(0.25).unwrap();
	assert_eq!(animating.cycle(), Ok(0.25));
	animating.stop_animation().unwrap();
	assert_eq!(animating.playback_rate(), Ok(0.0));

	assert_eq!(animating.is_sequence_finished(), Ok(false));
	STATE.with_borrow_mut(|state| state.finished = true);
	assert_eq!(animating.is_sequence_finished(), Ok(true));

	animating.studio_frame_advance().unwrap();
	animating.studio_frame_advance_manual(0.1).unwrap();
	assert_eq!(state(|state| state.advances.clone()), [None, Some(0.1)]);
}

#[test]
fn bodygroups_skins_poses_and_scale_are_set_through_the_members() {
	let scope = ();
	let mut mock = MockEntity::new(1);
	let (animating, _vtable) = mock_animating(mock_server(&scope), &mut mock);

	animating.set_bodygroup(0, 1).unwrap();
	assert_eq!(animating.bodygroup(0), Ok(1));
	assert_eq!(animating.bodygroup(1), Ok(0));
	animating.set_skin(1).unwrap();
	assert_eq!(animating.skin(), Ok(1));

	// The model clamps the value, and the member returns what it kept.
	assert_eq!(animating.set_pose_parameter(0, 3.0), Ok(1.0));
	assert_eq!(animating.set_pose_parameter(1, -0.5), Ok(-0.5));
	assert_eq!(state(|state| state.poses), [1.0, -0.5]);

	animating.set_model_scale(2.0, 0.5).unwrap();
	assert_eq!(animating.model_scale(), Ok(2.0));
	assert_eq!(state(|state| state.scale_durations.clone()), [0.5]);
}

#[test]
fn only_precached_models_are_set() {
	let scope = ();
	let mut mock = MockEntity::new(1);
	let (animating, _vtable) = mock_animating(mock_server(&scope), &mut mock);
	let models = models();

	animating.set_model(models, MODEL).unwrap();
	assert_eq!(state(|state| state.models.clone()), [MODEL.to_owned()]);

	assert_eq!(
		animating.set_model(models, c"models/bots/merasmus/merasmus.mdl"),
		Err(AnimatingError::ModelNotPrecached)
	);
	assert_eq!(state(|state| (state.models.len(), state.calls)), (1, 1));
}

#[test]
fn only_live_animating_entities_on_tf2_servers_are_used() {
	let scope = ();
	let server = mock_server(&scope);
	let mut mock = MockEntity::new(1);
	let (animating, _vtable) = mock_animating(server, &mut mock);
	let entity = animating.entity();

	assert_eq!(
		Animating::new(null_server(Game::SourceSdk2013, &scope), entity),
		Err(AnimatingError::NotTf2)
	);

	// Data maps without `CBaseAnimating`'s are not an animating entity's.
	set_datamap(state_maps(Vec::new()));
	assert_eq!(
		Animating::new(server, entity),
		Err(AnimatingError::NotAnimating)
	);
	set_datamap(state_maps(vec![(c"CBaseAnimating", Vec::new())]));
	assert_eq!(Animating::new(server, entity), Ok(animating));

	STATE.with_borrow_mut(|state| state.rejects = true);
	assert_eq!(animating.skin(), Err(AnimatingError::Rejected));
	assert_eq!(animating.sequence_name(0), Err(AnimatingError::Rejected));
	STATE.with_borrow_mut(|state| state.rejects = false);
	assert_eq!(state(|state| state.calls), 2);

	// Entities marked for deletion are neither wrapped nor called.
	mock.set_eflags(1);
	assert_eq!(
		Animating::new(server, entity),
		Err(AnimatingError::MarkedForDeletion)
	);
	assert_eq!(animating.skin(), Err(AnimatingError::MarkedForDeletion));
	assert_eq!(
		animating.sequence_name(0),
		Err(AnimatingError::MarkedForDeletion)
	);
	mock.set_eflags(0);
	assert_eq!(state(|state| state.calls), 2);

	// A game whose descriptor lacks a member refuses it before any call.
	let mut bare = class_description(c"CBaseAnimating", &mut [], null_mut());

	set_script_description(&raw mut bare);
	assert_eq!(animating.skin(), Err(AnimatingError::UnsupportedMethod));
	assert_eq!(state(|state| state.calls), 2);
	set_script_description(null_mut());
}
