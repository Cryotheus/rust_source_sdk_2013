//! Tests of `crate::tf2::next_bot`: a mock actor whose `MyNextBotPointer`
//! returns a fake `INextBot`, whose components answer through mock vtables
//! that record what they are asked.

use super::*;
use crate::test_support::entities::MockEntity;
use crate::test_support::server::null_server;
use crate::test_support::tf2::spawning::patch_slots;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use sdk_raw::tf2::next_bot::MY_NEXT_BOT_POINTER_SLOT;
use std::cell::{Cell, RefCell};
use std::ffi::c_char;
use std::mem::MaybeUninit;

/// An object with only a vtable pointer, standing in for `INextBot`, its
/// components and a known entity, whose mock vtables read nothing else.
#[repr(C)]
struct Fake<V> {
	vtable: *const V,
}

/// A leaked object whose vtable is `V`, with `patch` applied.
fn fake<V, T>(patch: impl FnOnce(*mut V)) -> *mut T {
	// SAFETY: The generated vtables hold only function pointers,
	// `unexpected_call` aborts whichever unpatched slot is reached, and the
	// patches only write slots of the vtable being built.
	let vtable = unsafe { mock_vtable::<V>(unexpected_call as *const (), patch) };

	Box::into_raw(Box::new(Fake {
		vtable: Box::leak(vtable),
	}))
	.cast()
}

/// Where the mock locomotion's feet are, and its velocity.
static FEET: sys::Vector = sys::Vector {
	x: 1.0,
	y: 2.0,
	z: 3.0,
};

/// The mock body's hull corners.
static HULL: [sys::Vector; 2] = [
	sys::Vector {
		x: -24.0,
		y: -24.0,
		z: 0.0,
	},
	sys::Vector {
		x: 24.0,
		y: 24.0,
		z: 82.0,
	},
];

thread_local! {
	/// The calls the mock components received.
	static CALLS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };

	/// The mock actor's entity.
	static ACTOR: Cell<*mut sys::CBaseEntity> = const { Cell::new(null_mut()) };

	/// The mock actor's `INextBot`.
	static BOT: Cell<*mut sys::INextBot> = const { Cell::new(null_mut()) };

	/// The mock actor's locomotion.
	static LOCOMOTION: Cell<*mut sys::ILocomotion> = const { Cell::new(null_mut()) };

	/// The mock actor's body.
	static BODY_COMPONENT: Cell<*mut sys::IBody> = const { Cell::new(null_mut()) };

	/// The mock actor's vision.
	static VISION: Cell<*mut sys::IVision> = const { Cell::new(null_mut()) };

	/// The entity another one stands on, or the actor's threat.
	static OTHER: Cell<*mut sys::CBaseEntity> = const { Cell::new(null_mut()) };

	/// The mock body's desired posture.
	static POSTURE: Cell<sys::IBody_PostureType> = const { Cell::new(sys::IBody_PostureType_STAND) };
}

/// Records a call.
fn record(call: String) {
	CALLS.with_borrow_mut(|calls| calls.push(call));
}

/// A vector's components, for recording.
fn components(vector: *const sys::Vector) -> (f32, f32, f32) {
	// SAFETY: The wrappers pass a pointer to a vector they hold.
	let vector = unsafe { vector.read() };

	(vector.x, vector.y, vector.z)
}

unsafe extern "C" fn my_next_bot_pointer(this: *mut sys::CBaseEntity) -> *mut sys::INextBot {
	if this == ACTOR.get() {
		BOT.get()
	} else {
		null_mut()
	}
}

unsafe extern "C" fn get_entity(_: *const sys::INextBot) -> *mut sys::CBaseCombatCharacter {
	ACTOR.get().cast()
}

unsafe extern "C" fn get_locomotion(_: *const sys::INextBot) -> *mut sys::ILocomotion {
	LOCOMOTION.get()
}

unsafe extern "C" fn get_body(_: *const sys::INextBot) -> *mut sys::IBody {
	BODY_COMPONENT.get()
}

unsafe extern "C" fn get_vision(_: *const sys::INextBot) -> *mut sys::IVision {
	VISION.get()
}

unsafe extern "C" fn approach(_: *mut sys::ILocomotion, goal: *const sys::Vector, weight: f32) {
	record(format!("Approach {:?} {weight}", components(goal)));
}

unsafe extern "C" fn drive_to(_: *mut sys::ILocomotion, goal: *const sys::Vector) {
	record(format!("DriveTo {:?}", components(goal)));
}

unsafe extern "C" fn face_towards(_: *mut sys::ILocomotion, target: *const sys::Vector) {
	record(format!("FaceTowards {:?}", components(target)));
}

unsafe extern "C" fn get_feet(_: *const sys::ILocomotion) -> *const sys::Vector {
	&raw const FEET
}

unsafe extern "C" fn get_ground(_: *const sys::ILocomotion) -> *mut sys::CBaseEntity {
	OTHER.get()
}

unsafe extern "C" fn is_area_traversable(
	_: *const sys::ILocomotion,
	area: *const sys::CNavArea,
) -> bool {
	record(format!("IsAreaTraversable {}", area.is_null()));

	true
}

unsafe extern "C" fn set_desired_speed(_: *mut sys::ILocomotion, speed: f32) {
	record(format!("SetDesiredSpeed {speed}"));
}

unsafe extern "C" fn clear_stuck_status(_: *mut sys::ILocomotion, reason: *const c_char) {
	// SAFETY: The wrapper passes a NUL-terminated reason.
	record(format!("ClearStuckStatus {:?}", unsafe {
		CStr::from_ptr(reason)
	}));
}

unsafe extern "C" fn jump(_: *mut sys::ILocomotion) {
	record("Jump".to_owned());
}

unsafe extern "C" fn aim_at_position(
	_: *mut sys::IBody,
	position: *const sys::Vector,
	priority: sys::IBody_LookAtPriorityType,
	duration: f32,
	reply: *mut sys::INextBotReply,
	reason: *const c_char,
) {
	assert!(reply.is_null() && reason.is_null());
	record(format!(
		"AimHeadTowards {:?} {priority} {duration}",
		components(position)
	));
}

unsafe extern "C" fn aim_at_entity(
	_: *mut sys::IBody,
	subject: *mut sys::CBaseEntity,
	priority: sys::IBody_LookAtPriorityType,
	duration: f32,
	reply: *mut sys::INextBotReply,
	reason: *const c_char,
) {
	assert!(reply.is_null() && reason.is_null());
	record(format!(
		"AimHeadTowards {} {priority} {duration}",
		subject == OTHER.get()
	));
}

unsafe extern "C" fn set_desired_posture(_: *mut sys::IBody, posture: sys::IBody_PostureType) {
	POSTURE.set(posture);
}

unsafe extern "C" fn get_desired_posture(_: *const sys::IBody) -> sys::IBody_PostureType {
	POSTURE.get()
}

unsafe extern "C" fn get_hull_mins(_: *const sys::IBody) -> *const sys::Vector {
	&raw const HULL[0]
}

unsafe extern "C" fn get_hull_maxs(_: *const sys::IBody) -> *const sys::Vector {
	&raw const HULL[1]
}

unsafe extern "C" fn is_able_to_see(
	_: *const sys::IVision,
	subject: *mut sys::CBaseEntity,
	check: sys::IVision_FieldOfViewCheckType,
	visible_spot: *mut sys::Vector,
) -> bool {
	assert!(visible_spot.is_null());

	subject == OTHER.get() && check == sys::IVision_FieldOfViewCheckType_DISREGARD_FOV
}

unsafe extern "C" fn is_able_to_see_position(
	_: *const sys::IVision,
	position: *const sys::Vector,
	check: sys::IVision_FieldOfViewCheckType,
) -> bool {
	components(position) == (1.0, 2.0, 3.0) && check == sys::IVision_FieldOfViewCheckType_USE_FOV
}

unsafe extern "C" fn get_known_count(
	_: *const sys::IVision,
	team: c_int,
	only_visible: bool,
	range: f32,
) -> c_int {
	record(format!("GetKnownCount {team} {only_visible} {range}"));

	4
}

unsafe extern "C" fn known_entity(_: *const sys::CKnownEntity) -> *mut sys::CBaseEntity {
	OTHER.get()
}

unsafe extern "C" fn primary_threat(
	_: *const sys::IVision,
	only_visible: bool,
) -> *const sys::CKnownEntity {
	if only_visible {
		return null();
	}

	fake::<sys::CKnownEntity__bindgen_vtable, sys::CKnownEntity>(|vtable| unsafe {
		(&raw mut (*vtable).CKnownEntity_GetEntity).write(known_entity);
	})
}

/// Leaks a fake actor: a mock entity whose `INextBot` holds mock components,
/// and another entity, which is not an actor. Returns both entities.
fn actor() -> (Entity<'static>, Entity<'static>) {
	let locomotion = fake::<sys::ILocomotion__bindgen_vtable, sys::ILocomotion>(|vtable| unsafe {
		(&raw mut (*vtable).ILocomotion_Approach).write(approach);
		(&raw mut (*vtable).ILocomotion_ClearStuckStatus).write(clear_stuck_status);
		(&raw mut (*vtable).ILocomotion_DriveTo).write(drive_to);
		(&raw mut (*vtable).ILocomotion_FaceTowards).write(face_towards);
		(&raw mut (*vtable).ILocomotion_GetFeet).write(get_feet);
		(&raw mut (*vtable).ILocomotion_GetGround).write(get_ground);
		(&raw mut (*vtable).ILocomotion_GetVelocity).write(get_feet);
		(&raw mut (*vtable).ILocomotion_IsAreaTraversable).write(is_area_traversable);
		(&raw mut (*vtable).ILocomotion_Jump).write(jump);
		(&raw mut (*vtable).ILocomotion_SetDesiredSpeed).write(set_desired_speed);
	});

	let body = fake::<sys::IBody__bindgen_vtable, sys::IBody>(|vtable| unsafe {
		(&raw mut (*vtable).IBody_AimHeadTowards).write(aim_at_position);
		(&raw mut (*vtable).IBody_AimHeadTowards1).write(aim_at_entity);
		(&raw mut (*vtable).IBody_GetDesiredPosture).write(get_desired_posture);
		(&raw mut (*vtable).IBody_GetHullMaxs).write(get_hull_maxs);
		(&raw mut (*vtable).IBody_GetHullMins).write(get_hull_mins);
		(&raw mut (*vtable).IBody_SetDesiredPosture).write(set_desired_posture);
	});

	let vision = fake::<sys::IVision__bindgen_vtable, sys::IVision>(|vtable| unsafe {
		(&raw mut (*vtable).IVision_GetKnownCount).write(get_known_count);
		(&raw mut (*vtable).IVision_GetPrimaryKnownThreat).write(primary_threat);
		(&raw mut (*vtable).IVision_IsAbleToSee).write(is_able_to_see);
		(&raw mut (*vtable).IVision_IsAbleToSee1).write(is_able_to_see_position);
	});

	LOCOMOTION.set(locomotion);
	BODY_COMPONENT.set(body);
	VISION.set(vision);

	BOT.set(fake::<sys::INextBot__bindgen_vtable, sys::INextBot>(
		|vtable| unsafe {
			(&raw mut (*vtable).INextBot_GetBodyInterface).write(get_body);
			(&raw mut (*vtable).INextBot_GetEntity).write(get_entity);
			(&raw mut (*vtable).INextBot_GetLocomotionInterface).write(get_locomotion);
			(&raw mut (*vtable).INextBot_GetVisionInterface).write(get_vision);
		},
	));

	let mut actor = MockEntity::new(1);
	let mut other = MockEntity::new(2);

	for mock in [&mut actor, &mut other] {
		patch_slots(
			mock,
			&[(MY_NEXT_BOT_POINTER_SLOT, my_next_bot_pointer as *const ())],
		);
	}

	ACTOR.set(actor.as_ptr());
	OTHER.set(other.as_ptr());

	// SAFETY: Mock entities are leaked, and their vtables answer what the
	// wrappers call of a `CBaseEntity`.
	unsafe {
		(
			Entity::from_raw(NonNull::new(actor.as_ptr()).unwrap()),
			Entity::from_raw(NonNull::new(other.as_ptr()).unwrap()),
		)
	}
}

/// The fake actor's interface.
fn interface(server: Server<'static>, actor: Entity<'static>) -> NextBotInterface<'static> {
	NextBotInterface::of(server, actor).unwrap().unwrap()
}

#[test]
fn actors_are_found_through_their_next_bot_pointer() {
	let (actor, other) = actor();
	let server = null_server(Game::TeamFortress2, &());

	let bot = interface(server, actor);

	assert_eq!(bot.as_ptr(), BOT.get());
	assert_eq!(bot.entity(server), actor);
	assert_eq!(bot.locomotion().as_ptr(), LOCOMOTION.get());
	assert_eq!(bot.body().as_ptr(), BODY_COMPONENT.get());
	assert_eq!(bot.vision().as_ptr(), VISION.get());
	assert_eq!(NextBotInterface::of(server, other), Ok(None));
	assert_eq!(
		NextBotInterface::of(null_server(Game::SourceSdk2013, &()), actor),
		Err(BotError::UnsupportedGame)
	);
}

#[test]
fn locomotion_requests_reach_the_actor() {
	let (actor, other) = actor();
	let server = null_server(Game::TeamFortress2, &());
	let locomotion = interface(server, actor).locomotion();

	locomotion.approach(Vector::new(4.0, 5.0, 6.0), 0.5);
	locomotion.drive_to(Vector::new(7.0, 8.0, 9.0));
	locomotion.face_towards(Vector::new(1.0, 0.0, 0.0));
	locomotion.set_desired_speed(300.0);
	locomotion.jump();
	locomotion.clear_stuck_status(c"unstuck");

	assert_eq!(
		CALLS.take(),
		[
			"Approach (4.0, 5.0, 6.0) 0.5",
			"DriveTo (7.0, 8.0, 9.0)",
			"FaceTowards (1.0, 0.0, 0.0)",
			"SetDesiredSpeed 300",
			"Jump",
			"ClearStuckStatus \"unstuck\"",
		]
	);
	assert_eq!(locomotion.feet(), Vector::new(1.0, 2.0, 3.0));
	assert_eq!(locomotion.velocity(), Vector::new(1.0, 2.0, 3.0));
	assert_eq!(locomotion.ground(server), Some(other));

	OTHER.set(null_mut());

	assert_eq!(locomotion.ground(server), None);
}

#[test]
fn locomotion_checks_nav_areas() {
	let (actor, _) = actor();
	let server = null_server(Game::TeamFortress2, &());
	let locomotion = interface(server, actor).locomotion();
	let mut area = Box::new(MaybeUninit::<sys::CTFNavArea>::zeroed());

	// SAFETY: The area is only passed on by address.
	let area = unsafe { NavArea::from_raw(NonNull::from(&mut *area).cast()) };

	assert!(locomotion.is_area_traversable(area));
	assert_eq!(CALLS.take(), ["IsAreaTraversable false"]);
}

#[test]
fn bodies_aim_and_take_postures() {
	let (actor, other) = actor();
	let server = null_server(Game::TeamFortress2, &());
	let body = interface(server, actor).body();

	body.aim_head_towards(Vector::new(1.0, 2.0, 3.0), LookAtPriority::Important, 1.5);
	body.aim_head_at(other, LookAtPriority::Mandatory, 0.0);

	assert_eq!(
		CALLS.take(),
		[
			"AimHeadTowards (1.0, 2.0, 3.0) 2 1.5",
			"AimHeadTowards true 4 0"
		]
	);
	assert_eq!(body.desired_posture(), Some(Posture::Stand));

	body.set_desired_posture(Posture::Crouch);

	assert_eq!(body.desired_posture(), Some(Posture::Crouch));

	POSTURE.set(9);

	assert_eq!(body.desired_posture(), None);
	assert_eq!(
		body.hull(),
		(
			Vector::new(-24.0, -24.0, 0.0),
			Vector::new(24.0, 24.0, 82.0)
		)
	);
}

#[test]
fn visions_see_and_count_what_they_know() {
	let (actor, other) = actor();
	let server = null_server(Game::TeamFortress2, &());
	let vision = interface(server, actor).vision();

	assert!(vision.is_able_to_see(other, false));
	assert!(!vision.is_able_to_see(other, true));
	assert!(vision.is_able_to_see_position(Vector::new(1.0, 2.0, 3.0), true));
	assert_eq!(vision.known_count(None, false, None), 4);
	assert_eq!(vision.known_count(Some(Team::Red), true, Some(500.0)), 4);
	assert_eq!(
		CALLS.take(),
		["GetKnownCount -2 false -1", "GetKnownCount 2 true 500"]
	);
	assert_eq!(vision.primary_known_threat(server, false), Some(other));
	assert_eq!(vision.primary_known_threat(server, true), None);
}

#[test]
fn enums_have_their_game_values() {
	assert_eq!(Posture::ALL.map(Posture::to_raw), [0, 1, 2, 3, 4]);
	assert_eq!(
		Posture::ALL.map(|posture| Posture::from_raw(posture.to_raw())),
		Posture::ALL.map(Some)
	);
	assert_eq!(Arousal::ALL.map(Arousal::to_raw), [0, 1, 2]);
	assert_eq!(Arousal::from_raw(3), None);
	assert_eq!(
		[
			LookAtPriority::Boring,
			LookAtPriority::Interesting,
			LookAtPriority::Important,
			LookAtPriority::Critical,
			LookAtPriority::Mandatory,
		]
		.map(LookAtPriority::to_raw),
		[0, 1, 2, 3, 4]
	);
}
