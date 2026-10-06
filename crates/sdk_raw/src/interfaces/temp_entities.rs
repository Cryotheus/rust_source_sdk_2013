//! Hand-written ABI of `ITempEntsSystem`'s effect dispatch that the generated
//! bindings do not describe: `CEffectData` from
//! `game/shared/effect_dispatch_data.h`, which they leave opaque, and the
//! particle dispatch values of `game/shared/particle_parse.h`.

use std::ffi::{CStr, c_int, c_short};
use std::mem::offset_of;

/// The number of particle systems the `ParticleEffectNames` table holds, which
/// bounds the index clients receive in [`EffectData::hit_box`]
/// (`MAX_PARTICLESYSTEMS_STRINGS` from
/// `game/server/networkstringtable_gamedll.h`).
#[doc(alias("MAX_PARTICLESYSTEMS_STRINGS"))]
pub const MAX_PARTICLE_SYSTEMS: c_int = 1 << 13;

/// The [`EffectData::flags`] of a particle effect that starts on the entity
/// of [`EffectData::entity_index`], rather than at [`EffectData::origin`].
pub const PARTICLE_DISPATCH_FROM_ENTITY: c_int = 1 << 0;

/// The [`EffectData::flags`] of a particle effect that first stops the
/// entity's other particle effects.
pub const PARTICLE_DISPATCH_RESET_PARTICLES: c_int = 1 << 1;

/// The name of the effect that plays a particle system, `ParticleEffect`,
/// which `DispatchParticleEffect` dispatches.
pub const PARTICLE_EFFECT: &CStr = c"ParticleEffect";

/// A particle effect created at its entity's origin, which stays where it
/// started (`ParticleAttachment_t`).
pub const PATTACH_ABSORIGIN: c_int = 0;

/// A particle effect created at its entity's origin, which follows it.
pub const PATTACH_ABSORIGIN_FOLLOW: c_int = 1;

/// A particle effect created at a custom origin, which stays where it started.
pub const PATTACH_CUSTOMORIGIN: c_int = 2;

/// A particle effect created at one of its entity's attachments, which stays
/// where it started.
pub const PATTACH_POINT: c_int = 3;

/// A particle effect created at one of its entity's attachments, which
/// follows it.
pub const PATTACH_POINT_FOLLOW: c_int = 4;

/// A particle effect created at the root bone of its entity's model, which
/// follows it.
pub const PATTACH_ROOTBONE_FOLLOW: c_int = 6;

/// A control point attached to no entity.
pub const PATTACH_WORLDORIGIN: c_int = 5;

// The layout of `CEffectData` in the server's game DLL, from the header's
// declaration order, which both ABIs pack alike: 4-byte fields, with the one-
// and two-byte fields padded up to the next.
const _: () = {
	assert!(offset_of!(EffectData, angles) == 36);
	assert!(offset_of!(EffectData, flags) == 48);
	assert!(offset_of!(EffectData, entity_index) == 52);
	assert!(offset_of!(EffectData, attachment_index) == 68);
	assert!(offset_of!(EffectData, surface_prop) == 72);
	assert!(offset_of!(EffectData, material) == 76);
	assert!(offset_of!(EffectData, hit_box) == 84);
	assert!(offset_of!(EffectData, color) == 88);
	assert!(offset_of!(EffectData, custom_colors) == 89);
	assert!(offset_of!(EffectData, custom_color_1) == 92);
	assert!(offset_of!(EffectData, control_point_1) == 116);
	assert!(offset_of!(EffectData, control_point_1_attachment) == 120);
	assert!(offset_of!(EffectData, effect_name) == 136);
	assert!(size_of::<EffectData>() == 140);
	assert!(align_of::<EffectData>() == 4);
};

/// The data the server sends with an effect it dispatches to clients,
/// `CEffectData` as the server's game DLL lays it out.
///
/// Each effect reads the fields its own way; [`Self::new`] fills them as the
/// class's constructor does. The nested `m_CustomColors` and `m_ControlPoint1`
/// are spelled out field by field, which leaves the layout unchanged.
#[doc(alias("CEffectData"))]
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct EffectData {
	/// Where the effect plays (`m_vOrigin`), which `ParticleEffect` takes as
	/// its first control point when it plays on no entity.
	#[doc(alias("m_vOrigin"))]
	pub origin: sys::Vector,

	/// A second point, such as where a tracer starts (`m_vStart`).
	/// `ParticleEffect` takes it as the offset of an effect that follows its
	/// entity.
	#[doc(alias("m_vStart"))]
	pub start: sys::Vector,

	/// A direction, such as a surface's normal (`m_vNormal`).
	#[doc(alias("m_vNormal"))]
	pub normal: sys::Vector,

	/// An orientation (`m_vAngles`).
	#[doc(alias("m_vAngles"))]
	pub angles: sys::QAngle,

	/// Bits only the effect gives meaning to, such as
	/// [`PARTICLE_DISPATCH_FROM_ENTITY`] (`m_fFlags`). Clients receive the low
	/// eight.
	#[doc(alias("m_fFlags"))]
	pub flags: c_int,

	/// The edict index of the entity the effect plays on (`m_nEntIndex`).
	#[doc(alias("m_nEntIndex", "entindex"))]
	pub entity_index: c_int,

	/// A scale (`m_flScale`).
	#[doc(alias("m_flScale"))]
	pub scale: f32,

	/// A magnitude, from 0 to 1023 (`m_flMagnitude`).
	#[doc(alias("m_flMagnitude"))]
	pub magnitude: f32,

	/// A radius, from 0 to 1023 (`m_flRadius`).
	#[doc(alias("m_flRadius"))]
	pub radius: f32,

	/// One of the entity's attachments (`m_nAttachmentIndex`), from -16 to 15
	/// as clients receive it.
	#[doc(alias("m_nAttachmentIndex"))]
	pub attachment_index: c_int,

	/// A surface property (`m_nSurfaceProp`).
	#[doc(alias("m_nSurfaceProp"))]
	pub surface_prop: c_short,

	/// Most often a model index (`m_nMaterial`).
	#[doc(alias("m_nMaterial"))]
	pub material: c_int,

	/// A damage type, or for `ParticleEffect`, the `PATTACH_*` value its
	/// particle system attaches to its entity by (`m_nDamageType`).
	#[doc(alias("m_nDamageType"))]
	pub damage_type: c_int,

	/// A hitbox, or for `ParticleEffect`, the particle system's index in the
	/// `ParticleEffectNames` table (`m_nHitBox`), below
	/// [`MAX_PARTICLE_SYSTEMS`].
	#[doc(alias("m_nHitBox"))]
	pub hit_box: c_int,

	/// A color (`m_nColor`).
	#[doc(alias("m_nColor"))]
	pub color: u8,

	/// Whether a particle effect takes [`Self::custom_color_1`] and
	/// [`Self::custom_color_2`] (`m_bCustomColors`).
	#[doc(alias("m_bCustomColors"))]
	pub custom_colors: bool,

	/// A particle effect's first custom color (`m_CustomColors.m_vecColor1`).
	#[doc(alias("m_vecColor1"))]
	pub custom_color_1: sys::Vector,

	/// A particle effect's second custom color (`m_CustomColors.m_vecColor2`).
	#[doc(alias("m_vecColor2"))]
	pub custom_color_2: sys::Vector,

	/// Whether a particle effect takes [`Self::control_point_1_offset`] as its
	/// second control point (`m_bControlPoint1`).
	#[doc(alias("m_bControlPoint1"))]
	pub control_point_1: bool,

	/// How a particle effect's second control point attaches
	/// (`m_ControlPoint1.m_eParticleAttachment`), which TF2's clients do not
	/// read.
	#[doc(alias("m_eParticleAttachment"))]
	pub control_point_1_attachment: c_int,

	/// A particle effect's second control point
	/// (`m_ControlPoint1.m_vecOffset`).
	#[doc(alias("m_vecOffset"))]
	pub control_point_1_offset: sys::Vector,

	/// The effect's entry in the `EffectDispatch` table (`m_iEffectName`),
	/// which `DispatchEffect` sets itself.
	effect_name: c_int,
}

impl EffectData {
	/// The data as `CEffectData`'s constructor leaves it: everything zero but
	/// [`Self::scale`], 1, and [`Self::control_point_1_attachment`],
	/// [`PATTACH_ABSORIGIN`].
	pub const fn new() -> Self {
		const ZERO: sys::Vector = sys::Vector {
			x: 0.0,
			y: 0.0,
			z: 0.0,
		};

		Self {
			origin: ZERO,
			start: ZERO,
			normal: ZERO,
			angles: sys::QAngle {
				x: 0.0,
				y: 0.0,
				z: 0.0,
			},
			flags: 0,
			entity_index: 0,
			scale: 1.0,
			magnitude: 0.0,
			radius: 0.0,
			attachment_index: 0,
			surface_prop: 0,
			material: 0,
			damage_type: 0,
			hit_box: 0,
			color: 0,
			custom_colors: false,
			custom_color_1: ZERO,
			custom_color_2: ZERO,
			control_point_1: false,
			control_point_1_attachment: PATTACH_ABSORIGIN,
			control_point_1_offset: ZERO,
			effect_name: 0,
		}
	}

	/// The data as the `CEffectData` that `DispatchEffect` takes, valid while
	/// it is borrowed. The game only copies it.
	pub const fn as_raw(&self) -> *const sys::CEffectData {
		(&raw const *self).cast()
	}
}

impl Default for EffectData {
	fn default() -> Self {
		Self::new()
	}
}
