//! TF2 bullet hooks, which run before the game's `CBaseEntity::FireBullets`
//! for the entities of the classes they cover, and may fire other bullets in
//! place of those.
//!
//! TF2's sentry guns fire their bullets through `FireBullets`, one a shot,
//! straight at their target, unless their builder aims them with the Wrangler.
//! Its players' weapons fire through `FX_FireBullets`, which does not call it.
//! `FireBullets` traces each bullet, draws the tracers its info asks for and
//! the impacts, and deals the bullets' damage once all of them are traced,
//! each entity they hit taking theirs as one hit.
//!
//! The callback is told the bullets the game fires, and may return others to
//! fire in their place, each [`Bullets`] by a call of its own, so that each
//! deals its damage as a hit of its own. Those calls keep everything of the
//! game's info but what [`Bullets`] holds, such as the attacker and the ammo
//! type, and do not reach the callback again. The hooks cover classes as
//! [`crate::hooks::tf2::class`] describes, such as the sentry gun's class, which
//! [`ClassTargets::objects`](source_sdk_2013::tf2::class_targets::ClassTargets::objects)
//! finds.
//!
//! Under SourceHook, `FireBullets` takes a hook manager, which every handle
//! shares.

use crate::MetamodApi;
use crate::hook::{HookAction, HookTiming, VirtualFunction};
use crate::hooks::tf2::class::ClassHooks;
use source_sdk_2013::entities::Entity;
use source_sdk_2013::math::Vector;
use source_sdk_2013::math::glam::Vec3;
use source_sdk_2013::raw::tf2::virtuals::{FIRE_BULLETS_SLOT, FireBulletsFn};
use source_sdk_2013::raw::vcall;
use source_sdk_2013::tf2::class_targets::BaseEntity;
use source_sdk_2013::{Server, ServerBinding, sys};
use std::cell::Cell;
use std::ffi::c_int;

/// A callback-scoped server, the entity that fires, and the bullets the game
/// fires, returning the bullets to fire in their place, if any: none for no
/// bullet at all. A panic is contained by the hook dispatcher, and fires the
/// game's bullets.
pub type BulletsFn = for<'s> fn(Server<'s>, Entity<'s>, &Bullets) -> Option<Vec<Bullets>>;

/// `FireBullets` in an entity's primary vtable.
const FIRE_BULLETS: VirtualFunction<FireBulletsFn> = VirtualFunction::new(FIRE_BULLETS_SLOT);

thread_local! {
	/// Whether the hook fires a callback's bullets, which it lets through.
	static REPLACING: Cell<bool> = const { Cell::new(false) };
}

/// Bullets fired by one call of `FireBullets`, as its `FireBulletsInfo_t`
/// describes them.
#[doc(alias("FireBulletsInfo_t"))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bullets {
	/// How many bullets (`m_iShots`).
	pub shots: c_int,

	/// Where they start (`m_vecSrc`).
	pub source: Vec3,

	/// Which way they go (`m_vecDirShooting`), a unit vector.
	pub direction: Vec3,

	/// How far each strays from it at most (`m_vecSpread`): the sines of the
	/// half-angles of the cone, across and up, as `VECTOR_CONE_3DEGREES` is.
	/// A sentry's are zero but while its builder aims it.
	pub spread: Vec3,

	/// How far they reach (`m_flDistance`).
	pub distance: f32,

	/// The damage of each (`m_flDamage`).
	pub damage: f32,

	/// One in how many of them draws a tracer, counting from the first
	/// (`m_iTracerFreq`): 1 for all, 0 for none.
	pub tracer_frequency: c_int,

	/// What the push of their damage is scaled by (`m_flDamageForceScale`).
	pub damage_force_scale: f32,
}

impl Bullets {
	/// The bullets of `info`.
	fn of(info: &sys::FireBulletsInfo_t) -> Self {
		Self {
			shots: info.m_iShots,
			source: Vector::from(info.m_vecSrc).0,
			direction: Vector::from(info.m_vecDirShooting).0,
			spread: Vector::from(info.m_vecSpread).0,
			distance: info.m_flDistance,
			damage: info.m_flDamage,
			tracer_frequency: info.m_iTracerFreq,
			damage_force_scale: info.m_flDamageForceScale,
		}
	}

	/// Writes them over what `info` describes.
	fn write(&self, info: &mut sys::FireBulletsInfo_t) {
		info.m_iShots = self.shots;
		info.m_vecSrc = Vector(self.source).into();
		info.m_vecDirShooting = Vector(self.direction).into();
		info.m_vecSpread = Vector(self.spread).into();
		info.m_flDistance = self.distance;
		info.m_flDamage = self.damage;
		info.m_iTracerFreq = self.tracer_frequency;
		info.m_flDamageForceScale = self.damage_force_scale;
	}
}

impl MetamodApi<'_> {
	/// Runs `callback` before each `CBaseEntity::FireBullets` of the entities
	/// of the classes the returned hooks cover, which are none until
	/// [`ClassHooks::cover`] covers them, and fires the bullets it returns in
	/// place of the game's; see the [module documentation](crate::hooks::tf2::bullets).
	///
	/// `binding` must describe the running server. The callback must not
	/// delete entities immediately, as [`Server::new`] requires.
	pub fn hook_bullets(
		self,
		binding: ServerBinding,
		callback: BulletsFn,
	) -> ClassHooks<BaseEntity> {
		ClassHooks::new(
			binding,
			callback,
			FIRE_BULLETS,
			&[HookTiming::Pre],
			|server, callback, entity, call| {
				if call.superseded() == Some(true) || REPLACING.get() {
					return HookAction::Ignore;
				}

				let (info,) = call.args();

				if info.is_null() {
					return HookAction::Ignore;
				}

				// SAFETY: `FireBullets` takes its info by reference, which the
				// game keeps alive through the call. It is copied by bytes, and
				// its pointers are only copied on.
				let game = unsafe { info.read() };

				let Some(volley) = callback(server, entity, &Bullets::of(&game)) else {
					return HookAction::Ignore;
				};

				let this = entity.as_ptr();

				REPLACING.set(true);

				for bullets in &volley {
					let mut info = game;

					bullets.write(&mut info);

					// SAFETY: The hooked method is the entity's own `FireBullets`,
					// and the entity is live for the call. The info is a copy of the
					// game's, whose pointers stay valid through it, with only plain
					// values changed.
					unsafe {
						vcall!(this as sys::CBaseEntity__bindgen_vtable => CBaseEntity_FireBullets(&raw const info))
					};
				}

				REPLACING.set(false);

				HookAction::Supersede(())
			},
		)
	}
}
