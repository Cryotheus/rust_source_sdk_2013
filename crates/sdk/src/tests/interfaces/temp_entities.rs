//! Tests of the particle effects `TempEntities` dispatches: the effect data the
//! game receives, and the effects refused before they reach it.

use super::*;
use crate::math::Vector;
use crate::test_support::entities::{MockEntity, set_networking};
use crate::test_support::leak;
use crate::test_support::user_messages::recipients;
use sdk_raw::test_support::edicts::mock_edict;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char};
use std::ptr::null_mut;

thread_local! {
	/// Every call to `DispatchEffect`, in order.
	static DISPATCHED: RefCell<Vec<Dispatched>> = const { RefCell::new(Vec::new()) };
}

/// The arguments `DispatchEffect` received, with the filter's recipients as
/// the game reads them through its vtable, and the prediction state it saw.
#[derive(Debug, Clone, PartialEq)]
struct Dispatched {
	recipients: Vec<c_int>,
	reliable: bool,
	delay: f32,
	position: Vector,
	name: CString,
	origin: Vector,
	start: Vector,
	flags: c_int,
	entity_index: c_int,
	scale: f32,
	attachment_index: c_int,
	damage_type: c_int,
	hit_box: c_int,
	control_point_1_attachment: c_int,

	/// Whether `IPredictionSystem::GetSuppressHost` returned null, so that
	/// `SuppressTE` left the filter alone.
	unfiltered: bool,
}

/// A mock of the game's temporary entity system, which keeps its vtable
/// alive.
struct MockTempEntities {
	_vtable: Box<sys::ITempEntsSystem__bindgen_vtable>,
	system: Box<sys::ITempEntsSystem>,
}

impl MockTempEntities {
	/// Records `DispatchEffect`, and fills every other slot with a stub that
	/// fails the test if called. A player's command is running, so the system
	/// suppresses that host's effects.
	fn new() -> Self {
		// SAFETY: The vtable holds only function pointers, `unexpected_call`
		// aborts whichever slot reaches it, and the patch only writes a slot of
		// the vtable being built.
		let vtable = unsafe {
			mock_vtable::<sys::ITempEntsSystem__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).ITempEntsSystem_DispatchEffect).write(dispatch_effect);
				},
			)
		};
		let system = Box::new(sys::ITempEntsSystem {
			_base: sys::IPredictionSystem {
				vtable_: (&raw const *vtable).cast(),
				m_pNextSystem: null_mut(),
				m_bSuppressEvent: false,
				m_pSuppressHost: leak(0_usize).cast(),
				m_nStatusPushed: 0,
			},
		});

		DISPATCHED.take();

		Self {
			_vtable: vtable,
			system,
		}
	}

	/// The mock, as the wrapper sees it.
	fn temp_entities(&mut self) -> TempEntities<'_> {
		// SAFETY: The mock outlives the borrow.
		unsafe { TempEntities::from_raw(NonNull::from(&mut *self.system)) }
	}
}

/// `ITempEntsSystem::DispatchEffect`, which records its arguments.
unsafe extern "C" fn dispatch_effect(
	this: *mut sys::ITempEntsSystem,
	filter: *mut sys::IRecipientFilter,
	delay: f32,
	position: *const sys::Vector,
	name: *const c_char,
	data: *const sys::CEffectData,
) {
	// SAFETY: The wrapper passes a live system, filter, position, name, and
	// data, which are read as the game reads them.
	let dispatched = unsafe {
		let vtable = (*filter).vtable_;
		let count = ((*vtable).IRecipientFilter_GetRecipientCount)(filter);
		let data = &*data.cast::<EffectData>();
		let base = &(*this)._base;

		Dispatched {
			recipients: (0..count)
				.map(|slot| ((*vtable).IRecipientFilter_GetRecipientIndex)(filter, slot))
				.collect(),
			reliable: ((*vtable).IRecipientFilter_IsReliable)(filter),
			delay,
			position: (*position).into(),
			name: CStr::from_ptr(name).to_owned(),
			origin: data.origin.into(),
			start: data.start.into(),
			flags: data.flags,
			entity_index: data.entity_index,
			scale: data.scale,
			attachment_index: data.attachment_index,
			damage_type: data.damage_type,
			hit_box: data.hit_box,
			control_point_1_attachment: data.control_point_1_attachment,
			unfiltered: base.m_nStatusPushed > 0 || base.m_pSuppressHost.is_null(),
		}
	};

	DISPATCHED.with_borrow_mut(|dispatched_effects| dispatched_effects.push(dispatched));
}

/// A leaked edict at `index`, for mock entities to report.
fn edict(index: c_int) -> *mut sys::edict_t {
	leak(mock_edict(index, false))
}

#[test]
fn particle_effects_reach_the_game_as_dispatch_particle_effect_sends_them() {
	let mut mock = MockTempEntities::new();
	let temp_entities = mock.temp_entities();
	let mut corpse = MockEntity::new(9);

	set_networking(null_mut(), edict(9));

	let effect = ParticleEffect::new(42, corpse.entity());

	temp_entities
		.dispatch_particle_effect(&recipients(&[2, 5], false), &effect)
		.unwrap();

	temp_entities
		.dispatch_particle_effect(
			&recipients(&[3], true),
			&ParticleEffect {
				attachment: ParticleAttachment::PointFollow(15),
				reset: true,
				..effect
			},
		)
		.unwrap();

	let following = Dispatched {
		recipients: vec![2, 5],
		reliable: false,
		delay: 0.0,
		position: Vector::new(1.0, 2.0, 3.0),
		name: c"ParticleEffect".to_owned(),
		origin: Vector::new(1.0, 2.0, 3.0),
		start: Vector::new(0.0, 0.0, 0.0),
		flags: 1,
		entity_index: 9,
		scale: 1.0,
		attachment_index: 0,
		damage_type: 1,
		hit_box: 42,
		control_point_1_attachment: 0,
		unfiltered: true,
	};

	DISPATCHED.with_borrow(|dispatched| {
		assert_eq!(
			*dispatched,
			[
				following.clone(),
				Dispatched {
					recipients: vec![3],
					reliable: true,
					flags: 3,
					attachment_index: 15,
					damage_type: 4,
					..following
				},
			]
		);
	});

	assert_eq!(
		mock.system._base.m_nStatusPushed, 0,
		"the prediction filtering is restored"
	);
}

#[test]
fn each_attachment_is_sent_as_its_pattach_value() {
	let mut mock = MockTempEntities::new();
	let temp_entities = mock.temp_entities();
	let mut corpse = MockEntity::new(9);

	set_networking(null_mut(), edict(9));

	let attachments = [
		(ParticleAttachment::Origin, 0, 0),
		(ParticleAttachment::OriginFollow, 1, 0),
		(ParticleAttachment::Point(1), 3, 1),
		(ParticleAttachment::PointFollow(7), 4, 7),
		(ParticleAttachment::RootBoneFollow, 6, 0),
	];

	for (attachment, _, _) in attachments {
		temp_entities
			.dispatch_particle_effect(
				&recipients(&[2], false),
				&ParticleEffect {
					attachment,
					..ParticleEffect::new(1, corpse.entity())
				},
			)
			.unwrap();
	}

	DISPATCHED.with_borrow(|dispatched| {
		let sent: Vec<_> = dispatched
			.iter()
			.map(|effect| (effect.damage_type, effect.attachment_index))
			.collect();

		let expected: Vec<_> = attachments
			.iter()
			.map(|&(_, attach_type, point)| (attach_type, point))
			.collect();

		assert_eq!(sent, expected);
	});
}

#[test]
fn effects_clients_cannot_receive_never_reach_the_game() {
	let mut mock = MockTempEntities::new();
	let temp_entities = mock.temp_entities();
	let mut corpse = MockEntity::new(9);
	let everyone = recipients(&[2], false);

	set_networking(null_mut(), edict(9));

	let effect = ParticleEffect::new(1, corpse.entity());

	for (refused, error) in [
		(
			ParticleEffect {
				system: 8192,
				..effect
			},
			ParticleEffectError::SystemOutOfRange(8192),
		),
		(
			ParticleEffect {
				attachment: ParticleAttachment::Point(0),
				..effect
			},
			ParticleEffectError::Attachment(0),
		),
		(
			ParticleEffect {
				attachment: ParticleAttachment::PointFollow(16),
				..effect
			},
			ParticleEffectError::Attachment(16),
		),
	] {
		assert_eq!(
			temp_entities.dispatch_particle_effect(&everyone, &refused),
			Err(error)
		);
	}

	// An effect that passes reaches no one without recipients.
	temp_entities
		.dispatch_particle_effect(&recipients(&[], false), &effect)
		.unwrap();

	set_networking(null_mut(), null_mut());

	assert_eq!(
		temp_entities.dispatch_particle_effect(&everyone, &effect),
		Err(ParticleEffectError::NotNetworked)
	);

	DISPATCHED.with_borrow(|dispatched| assert!(dispatched.is_empty()));
}
