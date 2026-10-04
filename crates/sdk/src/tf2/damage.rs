//! Owned damage descriptions and TF2 critical-hit controls.
//!
//! Damage hooks must copy `CTakeDamageInfo` before changing it: the engine
//! passes a C++ const reference, which can also point into another plugin's
//! immutable storage. [`DamageInfo`] preserves the complete native record
//! without forming a Rust reference to engine memory.

use crate::Server;
use crate::entities::{Entity, EntityHandle};
use crate::math::Vector;
use sdk_raw::tf2::damage;
use std::fmt;
use std::mem::{MaybeUninit, offset_of};
use std::ptr::NonNull;

/// The custom damage kind, an `ETFDmgCustom` value, of plasma, as in an
/// uncharged Cow Mangler 5000 shot, the Righteous Bison's and Pomson 6000's,
/// and the Monoculus' emergence. The game also gives it to the ragdoll of a
/// player killed by a weapon with the `ragdolls_plasma_effect` attribute.
///
/// Damage carries it as [`DamageInfo::custom_damage`], and a `tf_ragdoll` as
/// its networked `m_iDamageCustom`. Clients dissolve such a ragdoll, unless it
/// turns to ash or the player was a miniboss, so the player leaves no body
/// (`c_tf_player.cpp:1202-1210`).
#[doc(alias("TF_DMG_CUSTOM_PLASMA"))]
#[allow(
	clippy::unnecessary_cast,
	reason = "`ETFDmgCustom` is `c_int` on Windows but `c_uint` on Linux"
)]
pub const CUSTOM_DAMAGE_PLASMA: i32 = sys::ETFDmgCustom_TF_DMG_CUSTOM_PLASMA as i32;

/// The custom damage kind, an `ETFDmgCustom` value, of a charged Cow Mangler
/// 5000 shot. Clients dissolve the ragdoll of a player it kills as for
/// [`CUSTOM_DAMAGE_PLASMA`], and also mark it to gib
/// (`c_tf_player.cpp:1212-1221`).
#[doc(alias("TF_DMG_CUSTOM_PLASMA_CHARGED"))]
#[allow(
	clippy::unnecessary_cast,
	reason = "`ETFDmgCustom` is `c_int` on Windows but `c_uint` on Linux"
)]
pub const CUSTOM_DAMAGE_PLASMA_CHARGED: i32 = sys::ETFDmgCustom_TF_DMG_CUSTOM_PLASMA_CHARGED as i32;

/// Defines a documented getter and setter for one scalar `CTakeDamageInfo`
/// field. The setter panics unless the optional validity predicate accepts
/// the value.
macro_rules! scalar {
	(
		$(#[$get_meta:meta])* $get:ident,
		$(#[$set_meta:meta])* $set:ident,
		$field:ident,
		$ty:ty
	) => {
		scalar!(
			$(#[$get_meta])* $get,
			$(#[$set_meta])* $set,
			$field,
			$ty,
			|_: $ty| true
		);
	};

	(
		$(#[$get_meta:meta])* $get:ident,
		$(#[$set_meta:meta])* $set:ident,
		$field:ident,
		$ty:ty,
		$valid:expr
	) => {
		$(#[$get_meta])*
		pub fn $get(&self) -> $ty {
			// SAFETY: Native constructors initialize this field, and our
			// constructor initializes the complete record. This is owned memory.
			unsafe { (&raw const (*self.raw.as_ptr()).$field).read() }
		}

		$(#[$set_meta])*
		pub fn $set(&mut self, value: $ty) {
			assert!(
				($valid)(value),
				"invalid damage field: {}",
				stringify!($field)
			);

			// SAFETY: This field lies in our allocated native record.
			unsafe { (&raw mut (*self.raw.as_mut_ptr()).$field).write(value) };
		}
	};
}

/// The classification in `CTakeDamageInfo::ECritType`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
#[doc(alias("ECritType"))]
#[allow(
	clippy::unnecessary_cast,
	reason = "`ECritType` is `c_int` on Windows but `c_uint` on Linux"
)]
#[repr(i32)]
pub enum CriticalHit {
	/// Not a critical hit.
	#[default]
	#[doc(alias("CRIT_NONE"))]
	None = sys::CTakeDamageInfo_ECritType_CRIT_NONE as i32,

	/// A mini critical hit.
	#[doc(alias("CRIT_MINI"))]
	Mini = sys::CTakeDamageInfo_ECritType_CRIT_MINI as i32,

	/// A full critical hit.
	#[doc(alias("CRIT_FULL"))]
	Full = sys::CTakeDamageInfo_ECritType_CRIT_FULL as i32,
}

/// Independently permit full critical hits and mini critical hits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CriticalPolicy {
	/// Whether full critical hits keep their bonus.
	pub full: bool,

	/// Whether mini critical hits keep their bonus.
	pub mini: bool,
}

impl CriticalPolicy {
	/// Permits both kinds of critical hit. This is the default.
	pub const ALLOW_ALL: Self = Self {
		full: true,
		mini: true,
	};

	/// Denies both kinds of critical hit.
	pub const DISABLE_ALL: Self = Self {
		full: false,
		mini: false,
	};
}

impl Default for CriticalPolicy {
	fn default() -> Self {
		Self::ALLOW_ALL
	}
}

/// A scoped victim plus an owned copy of its damage arguments.
#[derive(Debug)]
pub struct DamageEvent<'s> {
	/// The entity taking damage.
	pub victim: Entity<'s>,

	/// An owned copy of the native damage arguments. Editing it does not
	/// change the record the engine passed in; a damage hook can instead
	/// submit the edited copy to the game through [`DamageInfo::as_ptr`].
	pub info: DamageInfo,
}

impl<'s> DamageEvent<'s> {
	/// Adapts native damage-hook arguments to scoped Rust values.
	///
	/// # Safety
	/// `victim` must be a live entity belonging to `server` and remain live
	/// for `'s`. `info` must satisfy [`DamageInfo::copy_from_raw`]. The call
	/// must obey the main-thread and reentrancy contract of [`Server::new`].
	pub unsafe fn from_raw(
		_server: Server<'s>,
		victim: NonNull<sys::CBaseEntity>,
		info: NonNull<sys::CTakeDamageInfo>,
	) -> Self {
		Self {
			// SAFETY: The caller supplies the entity's callback lifetime.
			victim: unsafe { Entity::from_raw(victim) },
			// SAFETY: The caller vouches for the native damage arguments.
			info: unsafe { DamageInfo::copy_from_raw(info) },
		}
	}
}

/// A native damage record owned by Rust. It contains serial-numbered entity
/// handles, not borrowed entity pointers, so a copy can be retained for logs.
/// Setters panic on non-finite floats or handles whose entity index is outside
/// the engine's table. Damage amount must also fit the engine's signed health
/// arithmetic. Damage callbacks contain these panics before returning to C++.
///
/// The SDK's constructor leaves its unused statistics field and padding
/// uninitialized. Keeping a `MaybeUninit` record and copying bytes preserves
/// them without ever claiming the entire native record is initialized.
#[doc(alias("CTakeDamageInfo"))]
pub struct DamageInfo {
	raw: MaybeUninit<sys::CTakeDamageInfo>,
}

impl DamageInfo {
	/// Conservative per-hit limit for values edited through this API. This is
	/// an API limit, not a TF2 limit: one million still leaves more than 700
	/// times headroom below `i32::MAX` after TF2's ordinary 3x critical bonus.
	/// It keeps edited values away from float-to-int conversion boundaries and
	/// ordinary health subtraction overflow. Later game/attribute multipliers
	/// are outside this record's control and must themselves remain valid.
	pub const MAX_DAMAGE: f32 = 1_000_000.0;

	/// Constructs a damage record with no attacker, weapon, force or position.
	/// Set the relevant handles before submitting it to the game.
	///
	/// # Panics
	/// If `amount` is rejected by [`Self::set_amount`].
	pub fn new(amount: f32, damage_type: DamageType) -> Self {
		// All-zero bytes are valid for this POD record. Native handles need
		// their explicit invalid sentinel, and base damage uses FLT_MAX.
		let mut result = Self {
			raw: MaybeUninit::zeroed(),
		};
		result.set_amount(amount);
		result.set_max_damage(amount);
		result.set_base_damage(damage::BASEDAMAGE_NOT_SPECIFIED);
		result.set_damage_type(damage_type);
		result.set_attacker(EntityHandle::INVALID);
		result.set_inflictor(EntityHandle::INVALID);
		result.set_weapon(EntityHandle::INVALID);
		result.set_bonus_provider(EntityHandle::INVALID);
		result.set_ammo_type(sdk_raw::tf2::damage::NO_AMMO_TYPE);
		result
	}

	/// Copies a live native record, including bytes not exposed by this API.
	///
	/// # Safety
	/// `raw` must point to a live `CTakeDamageInfo` matching the target SDK ABI,
	/// with its constructor-initialized fields valid, for the duration of the
	/// copy. No other thread may write it concurrently.
	pub unsafe fn copy_from_raw(raw: NonNull<sys::CTakeDamageInfo>) -> Self {
		let mut result = Self {
			raw: MaybeUninit::uninit(),
		};
		// SAFETY: Both allocations hold a complete native record and cannot
		// overlap. A byte copy preserves uninitialized bytes as such.
		unsafe { std::ptr::copy_nonoverlapping(raw.as_ptr(), result.raw.as_mut_ptr(), 1) };
		result
	}

	/// Whether `value` is a finite damage amount within `0.0..=MAX_DAMAGE`.
	fn valid_damage(value: f32) -> bool {
		value.is_finite() && (0.0..=Self::MAX_DAMAGE).contains(&value)
	}

	/// Removes a disallowed hit's recorded critical bonus after TF2's rules
	/// have computed it. Returns whether the record changed. Full and mini
	/// critical hits are independently controlled.
	///
	/// Call this at `OnTakeDamage_Alive`, not on incoming, unscaled damage.
	/// TF2's recorded bonus includes its critical distance compensation. This
	/// removes that bonus; it does not rerun earlier attribute, distance,
	/// assist-statistics or audiovisual processing. Later resistance rules
	/// can still change the resulting health loss.
	///
	/// # Panics
	/// If the amount left after removing the bonus is rejected by
	/// [`Self::set_amount`], for example a native amount above
	/// [`Self::MAX_DAMAGE`] plus the bonus. The record is then unchanged.
	pub fn apply_critical_policy(&mut self, policy: CriticalPolicy) -> bool {
		let denied = match self.critical_hit() {
			Some(CriticalHit::Full) => !policy.full,
			Some(CriticalHit::Mini) => !policy.mini,
			_ => false,
		};

		if denied {
			self.set_amount((self.amount() - self.damage_bonus()).max(0.0));
			self.set_damage_bonus(0.0);
			self.set_bonus_provider(EntityHandle::INVALID);
			self.set_incoming_critical(CriticalHit::None);
		}

		denied
	}

	/// Native read-only pointer, valid until this value is moved or dropped.
	pub fn as_ptr(&self) -> *const sys::CTakeDamageInfo {
		self.raw.as_ptr()
	}

	/// The entity that originated the attack, such as a player.
	#[doc(alias("GetAttacker"))]
	pub fn attacker(&self) -> EntityHandle {
		self.handle(offset_of!(sys::CTakeDamageInfo, m_hAttacker))
	}

	/// The critical classification, or `None` if the record holds a value
	/// outside `ECritType`.
	#[doc(alias("GetCritType"))]
	pub fn critical_hit(&self) -> Option<CriticalHit> {
		// SAFETY: Constructor-initialized scalar in owned memory.
		match unsafe { (&raw const (*self.as_ptr()).m_eCritType).read() } {
			sys::CTakeDamageInfo_ECritType_CRIT_NONE => Some(CriticalHit::None),
			sys::CTakeDamageInfo_ECritType_CRIT_MINI => Some(CriticalHit::Mini),
			sys::CTakeDamageInfo_ECritType_CRIT_FULL => Some(CriticalHit::Full),
			_ => None,
		}
	}

	/// The force the damage applies (`m_vecDamageForce`), which pushes the
	/// victim, or its ragdoll, in its direction.
	#[doc(alias("GetDamageForce"))]
	pub fn damage_force(&self) -> Vector {
		// SAFETY: Constructor-initialized vector in owned memory.
		unsafe { (&raw const (*self.as_ptr()).m_vecDamageForce).read() }.into()
	}

	/// Where the damage was dealt (`m_vecDamagePosition`), at which its force
	/// is applied.
	#[doc(alias("GetDamagePosition"))]
	pub fn damage_position(&self) -> Vector {
		// SAFETY: Constructor-initialized vector in owned memory.
		unsafe { (&raw const (*self.as_ptr()).m_vecDamagePosition).read() }.into()
	}

	/// The damage bitmask, including unknown and game-specific bits.
	#[doc(alias("GetDamageType"))]
	pub fn damage_type(&self) -> DamageType {
		// SAFETY: Constructor-initialized scalar in owned memory.
		DamageType::from_bits_retain(unsafe {
			(&raw const (*self.as_ptr()).m_bitsDamageType).read()
		} as u32)
	}

	/// Reads the `CBaseHandle` field at byte `offset` in the native record.
	fn handle(&self, offset: usize) -> EntityHandle {
		// SAFETY: Offsets below identify constructor-initialized handle fields
		// of the owned record.
		EntityHandle::from_raw(unsafe {
			let handle = self.as_ptr().byte_add(offset).cast::<sys::EHANDLE>();

			(&raw const (*handle)._base.m_Index).read()
		})
	}

	/// The entity that dealt the damage: a weapon, projectile or player.
	#[doc(alias("GetInflictor"))]
	pub fn inflictor(&self) -> EntityHandle {
		self.handle(offset_of!(sys::CTakeDamageInfo, m_hInflictor))
	}

	/// The position the victim is told the damage came from
	/// (`m_vecReportedPosition`), which its client's damage indicator points
	/// to.
	#[doc(alias("GetReportedPosition"))]
	pub fn reported_position(&self) -> Vector {
		// SAFETY: Constructor-initialized vector in owned memory.
		unsafe { (&raw const (*self.as_ptr()).m_vecReportedPosition).read() }.into()
	}

	/// Multiplies the damage amount by `factor`.
	///
	/// # Panics
	/// If the product is rejected by [`Self::set_amount`]. The amount is then
	/// unchanged.
	#[doc(alias("ScaleDamage"))]
	pub fn scale_amount(&mut self, factor: f32) {
		self.set_amount(self.amount() * factor);
	}

	/// Replaces the attacker.
	///
	/// # Panics
	/// If `handle`'s index lies outside the entity table.
	#[doc(alias("SetAttacker"))]
	pub fn set_attacker(&mut self, handle: EntityHandle) {
		self.set_handle(offset_of!(sys::CTakeDamageInfo, m_hAttacker), handle);
	}

	/// Replaces `m_hDamageBonusProvider`, the entity credited with the bonus.
	fn set_bonus_provider(&mut self, handle: EntityHandle) {
		self.set_handle(
			offset_of!(sys::CTakeDamageInfo, m_hDamageBonusProvider),
			handle,
		);
	}

	/// Replaces the force the damage applies.
	///
	/// # Panics
	/// If a component of `force` is not finite.
	#[doc(alias("SetDamageForce"))]
	pub fn set_damage_force(&mut self, force: Vector) {
		assert!(force.is_finite(), "invalid damage field: m_vecDamageForce");
		// SAFETY: Vector field in our allocated record.
		unsafe { (&raw mut (*self.raw.as_mut_ptr()).m_vecDamageForce).write(force.into()) };
	}

	/// Replaces where the damage was dealt.
	///
	/// # Panics
	/// If a component of `position` is not finite.
	#[doc(alias("SetDamagePosition"))]
	pub fn set_damage_position(&mut self, position: Vector) {
		assert!(
			position.is_finite(),
			"invalid damage field: m_vecDamagePosition"
		);
		// SAFETY: Vector field in our allocated record.
		unsafe { (&raw mut (*self.raw.as_mut_ptr()).m_vecDamagePosition).write(position.into()) };
	}

	/// Replaces the damage bitmask, keeping all 32 bits.
	#[doc(alias("SetDamageType"))]
	pub fn set_damage_type(&mut self, value: DamageType) {
		// SAFETY: Scalar field in our allocated record; retain all 32 bits.
		unsafe { (&raw mut (*self.raw.as_mut_ptr()).m_bitsDamageType).write(value.bits() as i32) };
	}

	/// Writes `handle` to the `CBaseHandle` field at byte `offset`.
	fn set_handle(&mut self, offset: usize, handle: EntityHandle) {
		assert!(
			handle
				.index()
				.is_none_or(|index| index < EntityHandle::SLOTS),
			"damage handle index exceeds the entity table"
		);
		// SAFETY: As above, in uniquely owned storage.
		unsafe {
			let field = self
				.raw
				.as_mut_ptr()
				.byte_add(offset)
				.cast::<sys::EHANDLE>();

			(&raw mut (*field)._base.m_Index).write(handle.to_raw());
		};
	}

	/// Selects an incoming critical classification before TF2 calculates the
	/// bonus. This does not multiply damage. Game conditions and attributes
	/// can still promote or suppress the hit later; use a late damage hook to
	/// enforce a policy on the computed hit.
	///
	/// Unlike the native `SetCritType`, this replaces a full classification
	/// with a mini one. It also sets [`DamageType::CRITICAL`] for
	/// [`CriticalHit::Full`] and clears it otherwise.
	#[doc(alias("SetCritType"))]
	pub fn set_incoming_critical(&mut self, critical: CriticalHit) {
		self.write_critical(critical);
		let ordinary = self.damage_type().difference(DamageType::CRITICAL);
		self.set_damage_type(if critical == CriticalHit::Full {
			ordinary | DamageType::CRITICAL
		} else {
			ordinary
		});
	}

	/// Replaces the inflictor.
	///
	/// # Panics
	/// If `handle`'s index lies outside the entity table.
	#[doc(alias("SetInflictor"))]
	pub fn set_inflictor(&mut self, handle: EntityHandle) {
		self.set_handle(offset_of!(sys::CTakeDamageInfo, m_hInflictor), handle);
	}

	/// Replaces the position the victim is told the damage came from.
	///
	/// # Panics
	/// If a component of `position` is not finite.
	#[doc(alias("SetReportedPosition"))]
	pub fn set_reported_position(&mut self, position: Vector) {
		assert!(
			position.is_finite(),
			"invalid damage field: m_vecReportedPosition"
		);
		// SAFETY: Vector field in our allocated record.
		unsafe { (&raw mut (*self.raw.as_mut_ptr()).m_vecReportedPosition).write(position.into()) };
	}

	/// Replaces the weapon.
	///
	/// # Panics
	/// If `handle`'s index lies outside the entity table.
	#[doc(alias("SetWeapon"))]
	pub fn set_weapon(&mut self, handle: EntityHandle) {
		self.set_handle(offset_of!(sys::CTakeDamageInfo, m_hWeapon), handle);
	}

	/// The weapon that made the attack. For a projectile this is the weapon
	/// that fired it, while the projectile is the [inflictor](Self::inflictor).
	#[doc(alias("GetWeapon"))]
	pub fn weapon(&self) -> EntityHandle {
		self.handle(offset_of!(sys::CTakeDamageInfo, m_hWeapon))
	}

	/// Writes `m_eCritType` without changing the damage bitmask.
	fn write_critical(&mut self, critical: CriticalHit) {
		// SAFETY: Scalar field in our owned record, with a valid native value.
		unsafe { (&raw mut (*self.raw.as_mut_ptr()).m_eCritType).write(critical as _) };
	}

	scalar! {
		/// The damage amount.
		#[doc(alias("GetDamage"))]
		amount,
		/// Replaces the damage amount.
		///
		/// # Panics
		/// If `value` is not finite or lies outside `0.0..=`[`Self::MAX_DAMAGE`].
		#[doc(alias("SetDamage"))]
		set_amount,
		m_flDamage,
		f32,
		Self::valid_damage
	}

	scalar! {
		/// `m_flMaxDamage`, which native constructors and [`Self::new`]
		/// initialize to the damage amount.
		#[doc(alias("GetMaxDamage"))]
		max_damage,
		/// Replaces `m_flMaxDamage`.
		///
		/// # Panics
		/// If `value` is not finite or lies outside `0.0..=`[`Self::MAX_DAMAGE`].
		#[doc(alias("SetMaxDamage"))]
		set_max_damage,
		m_flMaxDamage,
		f32,
		Self::valid_damage
	}

	scalar! {
		/// The damage before skill-level adjustments, or `f32::MAX`
		/// (`BASEDAMAGE_NOT_SPECIFIED`) when unspecified. Unlike the native
		/// `GetBaseDamage`, this returns the sentinel instead of the amount.
		#[doc(alias("GetBaseDamage", "m_flBaseDamage"))]
		base_damage,
		/// Replaces the base damage. `f32::MAX` marks it as unspecified.
		///
		/// # Panics
		/// If `value` is neither `f32::MAX` nor a finite value within
		/// `0.0..=`[`Self::MAX_DAMAGE`].
		#[doc(alias("m_flBaseDamage"))]
		set_base_damage,
		m_flBaseDamage,
		f32,
		|value: f32| value == damage::BASEDAMAGE_NOT_SPECIFIED || Self::valid_damage(value)
	}

	scalar! {
		/// The recorded damage increase, such as TF2's critical-hit bonus.
		#[doc(alias("GetDamageBonus"))]
		damage_bonus,
		/// Replaces the recorded damage increase. Unlike the native
		/// `SetDamageBonus`, this leaves the bonus provider unchanged.
		///
		/// # Panics
		/// If `value` is not finite or lies outside `0.0..=`[`Self::MAX_DAMAGE`].
		#[doc(alias("SetDamageBonus"))]
		set_damage_bonus,
		m_flDamageBonus,
		f32,
		Self::valid_damage
	}

	scalar! {
		/// The custom damage kind. In TF2 this is an `ETFDmgCustom` value, such
		/// as [`CUSTOM_DAMAGE_PLASMA`] or
		/// `sys::ETFDmgCustom_TF_DMG_CUSTOM_HEADSHOT as i32`. The cast is
		/// needed because the sys constant's type differs between ABIs.
		#[doc(alias("GetDamageCustom"))]
		custom_damage,
		/// Replaces the custom damage kind.
		#[doc(alias("SetDamageCustom"))]
		set_custom_damage,
		m_iDamageCustom,
		i32
	}

	scalar! {
		/// The ammo type of the weapon that caused the damage, or
		/// [`NO_AMMO_TYPE`](sdk_raw::tf2::damage::NO_AMMO_TYPE) for none.
		#[doc(alias("GetAmmoType"))]
		ammo_type,
		/// Replaces the ammo type.
		/// [`NO_AMMO_TYPE`](sdk_raw::tf2::damage::NO_AMMO_TYPE) means none.
		#[doc(alias("SetAmmoType"))]
		set_ammo_type,
		m_iAmmoType,
		i32
	}

	scalar! {
		/// Whether the damage bypasses the game rules' teammate damage check.
		#[doc(alias("IsForceFriendlyFire"))]
		force_friendly_fire,
		/// Sets whether the damage bypasses the teammate damage check.
		#[doc(alias("SetForceFriendlyFire"))]
		set_force_friendly_fire,
		m_bForceFriendlyFire,
		bool
	}
}

impl Clone for DamageInfo {
	fn clone(&self) -> Self {
		// SAFETY: Our record was constructed here or copied under this contract.
		unsafe { Self::copy_from_raw(NonNull::from(&self.raw).cast()) }
	}
}

impl fmt::Debug for DamageInfo {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("DamageInfo")
			.field("amount", &self.amount())
			.field("damage_type", &self.damage_type())
			.field("critical_hit", &self.critical_hit())
			.field("attacker", &self.attacker())
			.field("weapon", &self.weapon())
			.finish_non_exhaustive()
	}
}

bitflags::bitflags! {
	/// Source's damage bitmask. Unknown and game-specific bits are preserved.
	///
	/// Every bit counts as a known flag, so [`all`](Self::all) sets all 32,
	/// and `!` and [`from_bits_truncate`](Self::from_bits_truncate) keep
	/// unnamed bits too.
	#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
	pub struct DamageType: u32 {
		/// Lets any damage type gib the victim on death.
		#[doc(alias("DMG_ALWAYSGIB"))]
		const ALWAYS_GIB = damage::DMG_ALWAYSGIB as u32;

		/// Explosive blast damage.
		#[doc(alias("DMG_BLAST"))]
		const BLAST = damage::DMG_BLAST as u32;

		/// Shotgun pellets, distinct from [`Self::BULLET`].
		#[doc(alias("DMG_BUCKSHOT"))]
		const BUCKSHOT = damage::DMG_BUCKSHOT as u32;

		/// Gunshot damage.
		#[doc(alias("DMG_BULLET"))]
		const BULLET = damage::DMG_BULLET as u32;

		/// Heat burns.
		#[doc(alias("DMG_BURN"))]
		const BURN = damage::DMG_BURN as u32;

		/// Blunt impact, such as a crowbar or punch.
		#[doc(alias("DMG_CLUB"))]
		const CLUB = damage::DMG_CLUB as u32;

		/// TF2's `DMG_CRITICAL` aliases `DMG_ACID`. Both full and mini critical
		/// hits carry it after TF2 has computed their damage bonus.
		#[doc(alias("DMG_CRITICAL", "DMG_ACID"))]
		const CRITICAL = damage::DMG_CRITICAL as u32;

		/// Crushing by a falling or moving object.
		#[doc(alias("DMG_CRUSH"))]
		const CRUSH = damage::DMG_CRUSH as u32;

		/// Source's `DMG_DIRECT` bit. The SDK's `CEntityFlame` sets it
		/// alongside [`Self::BURN`] for an attached fire's damage.
		#[doc(alias("DMG_DIRECT"))]
		const DIRECT = damage::DMG_DIRECT as u32;

		/// Drowning.
		#[doc(alias("DMG_DROWN"))]
		const DROWN = damage::DMG_DROWN as u32;

		/// Falling too far.
		#[doc(alias("DMG_FALL"))]
		const FALL = damage::DMG_FALL as u32;

		/// The empty mask. Every mask [contains](Self::contains) it.
		#[doc(alias("DMG_GENERIC"))]
		const GENERIC = damage::DMG_GENERIC as u32;

		/// Stops any damage type from gibbing the victim on death.
		#[doc(alias("DMG_NEVERGIB"))]
		const NEVER_GIB = damage::DMG_NEVERGIB as u32;

		/// Prevents the damage from applying a physics force.
		#[doc(alias("DMG_PREVENT_PHYSICS_FORCE"))]
		const PREVENT_PHYSICS_FORCE = damage::DMG_PREVENT_PHYSICS_FORCE as u32;

		/// Electric shock.
		#[doc(alias("DMG_SHOCK"))]
		const SHOCK = damage::DMG_SHOCK as u32;

		/// Cutting, clawing or stabbing.
		#[doc(alias("DMG_SLASH"))]
		const SLASH = damage::DMG_SLASH as u32;

		/// TF2's `DMG_USEDISTANCEMOD`, which aliases `DMG_SLOWBURN`.
		#[doc(alias("DMG_USEDISTANCEMOD", "DMG_SLOWBURN"))]
		const USE_DISTANCE_MOD = damage::DMG_USEDISTANCEMOD as u32;

		/// TF2's `DMG_USE_HITLOCATIONS`, which aliases `DMG_AIRBOAT`.
		#[doc(alias("DMG_USE_HITLOCATIONS", "DMG_AIRBOAT"))]
		const USE_HIT_LOCATIONS = damage::DMG_USE_HITLOCATIONS as u32;

		// Bits without a constant here, which the engine and games may set.
		const _ = !0;
	}
}

impl DamageType {
	/// For healing, TF2's `DMG_IGNORE_DEBUFFS`, the bits of [`Self::SLASH`]:
	/// healing marked as exempt from TF2's healing debuffs, which
	/// `CTFPlayer::TakeHealth` no longer checks. It is no flag of its own, so
	/// formatting a mask names it `SLASH`.
	#[doc(alias("DMG_IGNORE_DEBUFFS"))]
	pub const IGNORE_DEBUFFS: Self = Self::from_bits_retain(damage::DMG_IGNORE_DEBUFFS as u32);

	/// For healing, TF2's `DMG_IGNORE_MAXHEALTH`, the bits of
	/// [`Self::BULLET`]: healing a player beyond its maximum health, as
	/// overheal does, through
	/// [`Entity::take_health`](crate::entities::Entity::take_health). It is no
	/// flag of its own, so formatting a mask names it `BULLET`.
	#[doc(alias("DMG_IGNORE_MAXHEALTH"))]
	pub const IGNORE_MAX_HEALTH: Self = Self::from_bits_retain(damage::DMG_IGNORE_MAXHEALTH as u32);
}
