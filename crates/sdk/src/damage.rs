//! Owned damage descriptions and TF2 critical-hit controls.
//!
//! Damage hooks must copy `CTakeDamageInfo` before changing it: the engine
//! passes a C++ const reference, which can also point into another plugin's
//! immutable storage. [`DamageInfo`] preserves the complete native record
//! without forming a Rust reference to engine memory.

use crate::Server;
use crate::entities::{Entity, EntityHandle};
use std::fmt;
use std::mem::{MaybeUninit, offset_of};
use std::ops::{BitAnd, BitOr, BitOrAssign, Not};
use std::ptr::NonNull;

/// The classification in `CTakeDamageInfo::ECritType`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
#[doc(alias = "ECritType")]
#[repr(i32)]
pub enum CriticalHit {
	/// Not a critical hit.
	#[default]
	#[doc(alias = "CRIT_NONE")]
	None = sys::CTakeDamageInfo_ECritType_CRIT_NONE as i32,

	/// A mini critical hit.
	#[doc(alias = "CRIT_MINI")]
	Mini = sys::CTakeDamageInfo_ECritType_CRIT_MINI as i32,

	/// A full critical hit.
	#[doc(alias = "CRIT_FULL")]
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

/// A native damage record owned by Rust. It contains serial-numbered entity
/// handles, not borrowed entity pointers, so a copy can be retained for logs.
/// Setters panic on non-finite floats or handles whose entity index is outside
/// the engine's table. Damage amount must also fit the engine's signed health
/// arithmetic. Damage callbacks contain these panics before returning to C++.
///
/// The SDK's constructor leaves its unused statistics field and padding
/// uninitialized. Keeping a `MaybeUninit` record and copying bytes preserves
/// them without ever claiming the entire native record is initialized.
#[doc(alias = "CTakeDamageInfo")]
pub struct DamageInfo {
	raw: MaybeUninit<sys::CTakeDamageInfo>,
}

/// Source's damage bitmask. Unknown and game-specific bits are preserved.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DamageType(pub u32);

impl DamageType {
	/// Lets any damage type gib the victim on death.
	#[doc(alias = "DMG_ALWAYSGIB")]
	pub const ALWAYS_GIB: Self = Self(1 << 13);

	/// Explosive blast damage.
	#[doc(alias = "DMG_BLAST")]
	pub const BLAST: Self = Self(1 << 6);

	/// Shotgun pellets, distinct from [`Self::BULLET`].
	#[doc(alias = "DMG_BUCKSHOT")]
	pub const BUCKSHOT: Self = Self(1 << 29);

	/// Gunshot damage.
	#[doc(alias = "DMG_BULLET")]
	pub const BULLET: Self = Self(1 << 1);

	/// Heat burns.
	#[doc(alias = "DMG_BURN")]
	pub const BURN: Self = Self(1 << 3);

	/// Blunt impact, such as a crowbar or punch.
	#[doc(alias = "DMG_CLUB")]
	pub const CLUB: Self = Self(1 << 7);

	/// TF2's `DMG_CRITICAL` aliases `DMG_ACID`. Both full and mini critical
	/// hits carry it after TF2 has computed their damage bonus.
	#[doc(alias = "DMG_CRITICAL")]
	#[doc(alias = "DMG_ACID")]
	pub const CRITICAL: Self = Self(1 << 20);

	/// Crushing by a falling or moving object.
	#[doc(alias = "DMG_CRUSH")]
	pub const CRUSH: Self = Self(1 << 0);

	/// Source's `DMG_DIRECT` bit. The SDK's `CEntityFlame` sets it alongside
	/// [`Self::BURN`] for an attached fire's damage.
	#[doc(alias = "DMG_DIRECT")]
	pub const DIRECT: Self = Self(1 << 28);

	/// Drowning.
	#[doc(alias = "DMG_DROWN")]
	pub const DROWN: Self = Self(1 << 14);

	/// Falling too far.
	#[doc(alias = "DMG_FALL")]
	pub const FALL: Self = Self(1 << 5);

	/// The empty mask. Every mask [contains](Self::contains) it.
	#[doc(alias = "DMG_GENERIC")]
	pub const GENERIC: Self = Self(0);

	/// Stops any damage type from gibbing the victim on death.
	#[doc(alias = "DMG_NEVERGIB")]
	pub const NEVER_GIB: Self = Self(1 << 12);

	/// Prevents the damage from applying a physics force.
	#[doc(alias = "DMG_PREVENT_PHYSICS_FORCE")]
	pub const PREVENT_PHYSICS_FORCE: Self = Self(1 << 11);

	/// Electric shock.
	#[doc(alias = "DMG_SHOCK")]
	pub const SHOCK: Self = Self(1 << 8);

	/// Cutting, clawing or stabbing.
	#[doc(alias = "DMG_SLASH")]
	pub const SLASH: Self = Self(1 << 2);

	/// TF2's `DMG_USEDISTANCEMOD`, which aliases `DMG_SLOWBURN`.
	#[doc(alias = "DMG_USEDISTANCEMOD")]
	#[doc(alias = "DMG_SLOWBURN")]
	pub const USE_DISTANCE_MOD: Self = Self(1 << 21);

	/// TF2's `DMG_USE_HITLOCATIONS`, which aliases `DMG_AIRBOAT`.
	#[doc(alias = "DMG_USE_HITLOCATIONS")]
	#[doc(alias = "DMG_AIRBOAT")]
	pub const USE_HIT_LOCATIONS: Self = Self(1 << 25);

	/// Whether every bit of `other` is set in `self`.
	pub const fn contains(self, other: Self) -> bool {
		self.0 & other.0 == other.0
	}
}

impl BitAnd for DamageType {
	type Output = Self;

	fn bitand(self, rhs: Self) -> Self {
		Self(self.0 & rhs.0)
	}
}

impl BitOr for DamageType {
	type Output = Self;

	fn bitor(self, rhs: Self) -> Self {
		Self(self.0 | rhs.0)
	}
}

impl BitOrAssign for DamageType {
	fn bitor_assign(&mut self, rhs: Self) {
		self.0 |= rhs.0;
	}
}

impl Not for DamageType {
	type Output = Self;

	fn not(self) -> Self {
		Self(!self.0)
	}
}

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
		result.set_base_damage(f32::MAX);
		result.set_damage_type(damage_type);
		result.set_attacker(EntityHandle::INVALID);
		result.set_inflictor(EntityHandle::INVALID);
		result.set_weapon(EntityHandle::INVALID);
		result.set_bonus_provider(EntityHandle::INVALID);
		result.set_ammo_type(-1);
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

	/// Native read-only pointer, valid until this value is moved or dropped.
	pub fn as_ptr(&self) -> *const sys::CTakeDamageInfo {
		self.raw.as_ptr()
	}

	scalar!(
		/// The damage amount.
		#[doc(alias = "GetDamage")]
		amount,
		/// Replaces the damage amount.
		///
		/// # Panics
		/// If `value` is not finite or lies outside `0.0..=`[`Self::MAX_DAMAGE`].
		#[doc(alias = "SetDamage")]
		set_amount,
		m_flDamage,
		f32,
		Self::valid_damage
	);
	scalar!(
		/// `m_flMaxDamage`, which native constructors and [`Self::new`]
		/// initialize to the damage amount.
		#[doc(alias = "GetMaxDamage")]
		max_damage,
		/// Replaces `m_flMaxDamage`.
		///
		/// # Panics
		/// If `value` is not finite or lies outside `0.0..=`[`Self::MAX_DAMAGE`].
		#[doc(alias = "SetMaxDamage")]
		set_max_damage,
		m_flMaxDamage,
		f32,
		Self::valid_damage
	);
	scalar!(
		/// The damage before skill-level adjustments, or `f32::MAX`
		/// (`BASEDAMAGE_NOT_SPECIFIED`) when unspecified. Unlike the native
		/// `GetBaseDamage`, this returns the sentinel instead of the amount.
		#[doc(alias = "GetBaseDamage")]
		#[doc(alias = "m_flBaseDamage")]
		base_damage,
		/// Replaces the base damage. `f32::MAX` marks it as unspecified.
		///
		/// # Panics
		/// If `value` is neither `f32::MAX` nor a finite value within
		/// `0.0..=`[`Self::MAX_DAMAGE`].
		#[doc(alias = "m_flBaseDamage")]
		set_base_damage,
		m_flBaseDamage,
		f32,
		|value: f32| value == f32::MAX || Self::valid_damage(value)
	);
	scalar!(
		/// The recorded damage increase, such as TF2's critical-hit bonus.
		#[doc(alias = "GetDamageBonus")]
		damage_bonus,
		/// Replaces the recorded damage increase. Unlike the native
		/// `SetDamageBonus`, this leaves the bonus provider unchanged.
		///
		/// # Panics
		/// If `value` is not finite or lies outside `0.0..=`[`Self::MAX_DAMAGE`].
		#[doc(alias = "SetDamageBonus")]
		set_damage_bonus,
		m_flDamageBonus,
		f32,
		Self::valid_damage
	);
	scalar!(
		/// The custom damage kind. In TF2 this is an `ETFDmgCustom` value, such
		/// as `sys::ETFDmgCustom_TF_DMG_CUSTOM_HEADSHOT as i32`. The cast is
		/// needed because the sys constant's type differs between ABIs.
		#[doc(alias = "GetDamageCustom")]
		custom_damage,
		/// Replaces the custom damage kind.
		#[doc(alias = "SetDamageCustom")]
		set_custom_damage,
		m_iDamageCustom,
		i32
	);
	scalar!(
		/// The ammo type of the weapon that caused the damage, or -1 for none.
		#[doc(alias = "GetAmmoType")]
		ammo_type,
		/// Replaces the ammo type. -1 means none.
		#[doc(alias = "SetAmmoType")]
		set_ammo_type,
		m_iAmmoType,
		i32
	);
	scalar!(
		/// Whether the damage bypasses the game rules' teammate damage check.
		#[doc(alias = "IsForceFriendlyFire")]
		force_friendly_fire,
		/// Sets whether the damage bypasses the teammate damage check.
		#[doc(alias = "SetForceFriendlyFire")]
		set_force_friendly_fire,
		m_bForceFriendlyFire,
		bool
	);

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

	/// The entity that originated the attack, such as a player.
	#[doc(alias = "GetAttacker")]
	pub fn attacker(&self) -> EntityHandle {
		self.handle(offset_of!(sys::CTakeDamageInfo, m_hAttacker))
	}

	/// The critical classification, or `None` if the record holds a value
	/// outside `ECritType`.
	#[doc(alias = "GetCritType")]
	pub fn critical_hit(&self) -> Option<CriticalHit> {
		// SAFETY: Constructor-initialized scalar in owned memory.
		match unsafe { (&raw const (*self.as_ptr()).m_eCritType).read() } {
			sys::CTakeDamageInfo_ECritType_CRIT_NONE => Some(CriticalHit::None),
			sys::CTakeDamageInfo_ECritType_CRIT_MINI => Some(CriticalHit::Mini),
			sys::CTakeDamageInfo_ECritType_CRIT_FULL => Some(CriticalHit::Full),
			_ => None,
		}
	}

	/// The damage bitmask, including unknown and game-specific bits.
	#[doc(alias = "GetDamageType")]
	pub fn damage_type(&self) -> DamageType {
		// SAFETY: Constructor-initialized scalar in owned memory.
		DamageType(unsafe { (&raw const (*self.as_ptr()).m_bitsDamageType).read() } as u32)
	}

	/// Reads the `CBaseHandle` field at byte `offset` in the native record.
	fn handle(&self, offset: usize) -> EntityHandle {
		// SAFETY: Offsets below identify constructor-initialized CBaseHandle
		// fields, whose sole member is a u32 on both supported ABIs.
		EntityHandle::from_raw(unsafe {
			self.as_ptr().cast::<u8>().add(offset).cast::<u32>().read()
		})
	}

	/// The entity that dealt the damage: a weapon, projectile or player.
	#[doc(alias = "GetInflictor")]
	pub fn inflictor(&self) -> EntityHandle {
		self.handle(offset_of!(sys::CTakeDamageInfo, m_hInflictor))
	}

	/// Multiplies the damage amount by `factor`.
	///
	/// # Panics
	/// If the product is rejected by [`Self::set_amount`]. The amount is then
	/// unchanged.
	#[doc(alias = "ScaleDamage")]
	pub fn scale_amount(&mut self, factor: f32) {
		self.set_amount(self.amount() * factor);
	}

	/// Replaces the attacker.
	///
	/// # Panics
	/// If `handle`'s index lies outside the entity table.
	#[doc(alias = "SetAttacker")]
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

	/// Replaces the damage bitmask, keeping all 32 bits.
	#[doc(alias = "SetDamageType")]
	pub fn set_damage_type(&mut self, value: DamageType) {
		// SAFETY: Scalar field in our allocated record; retain all 32 bits.
		unsafe { (&raw mut (*self.raw.as_mut_ptr()).m_bitsDamageType).write(value.0 as i32) };
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
			self.raw
				.as_mut_ptr()
				.cast::<u8>()
				.add(offset)
				.cast::<u32>()
				.write(handle.to_raw())
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
	#[doc(alias = "SetCritType")]
	pub fn set_incoming_critical(&mut self, critical: CriticalHit) {
		self.write_critical(critical);
		let ordinary = self.damage_type() & !DamageType::CRITICAL;
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
	#[doc(alias = "SetInflictor")]
	pub fn set_inflictor(&mut self, handle: EntityHandle) {
		self.set_handle(offset_of!(sys::CTakeDamageInfo, m_hInflictor), handle);
	}

	/// Replaces the weapon.
	///
	/// # Panics
	/// If `handle`'s index lies outside the entity table.
	#[doc(alias = "SetWeapon")]
	pub fn set_weapon(&mut self, handle: EntityHandle) {
		self.set_handle(offset_of!(sys::CTakeDamageInfo, m_hWeapon), handle);
	}

	/// The weapon that made the attack. For a projectile this is the weapon
	/// that fired it, while the projectile is the [inflictor](Self::inflictor).
	#[doc(alias = "GetWeapon")]
	pub fn weapon(&self) -> EntityHandle {
		self.handle(offset_of!(sys::CTakeDamageInfo, m_hWeapon))
	}

	/// Writes `m_eCritType` without changing the damage bitmask.
	fn write_critical(&mut self, critical: CriticalHit) {
		// SAFETY: Scalar field in our owned record, with a valid native value.
		unsafe { (&raw mut (*self.raw.as_mut_ptr()).m_eCritType).write(critical as _) };
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn all_editable_damage_scalars_are_bounded_except_the_base_damage_sentinel() {
		let setters: [fn(&mut DamageInfo, f32); 4] = [
			DamageInfo::set_amount,
			DamageInfo::set_max_damage,
			DamageInfo::set_base_damage,
			DamageInfo::set_damage_bonus,
		];
		for setter in setters {
			let mut damage = DamageInfo::new(10.0, DamageType::GENERIC);
			for value in [
				f32::NAN,
				f32::INFINITY,
				f32::NEG_INFINITY,
				-1.0,
				DamageInfo::MAX_DAMAGE + 1.0,
				1.0e30,
			] {
				assert!(
					std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| setter(
						&mut damage,
						value
					)))
					.is_err()
				);
			}
			setter(&mut damage, 0.0);
			setter(&mut damage, DamageInfo::MAX_DAMAGE);
		}
		let mut damage = DamageInfo::new(1.0, DamageType::GENERIC);
		damage.set_base_damage(f32::MAX);
		assert_eq!(damage.base_damage(), f32::MAX);
		for setter in [
			DamageInfo::set_amount,
			DamageInfo::set_max_damage,
			DamageInfo::set_damage_bonus,
		] {
			assert!(
				std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| setter(
					&mut damage,
					f32::MAX
				)))
				.is_err()
			);
		}
		assert!(
			std::panic::catch_unwind(std::panic::AssertUnwindSafe(
				|| damage.scale_amount(f32::MAX)
			))
			.is_err()
		);
		assert_eq!(damage.amount(), 1.0);
	}

	#[test]
	fn copies_change_owned_data_and_preserve_unknown_bits_and_handles() {
		let original = DamageInfo::new(24.0, DamageType::BULLET | DamageType(1 << 31));
		let mut copy = original.clone();
		copy.scale_amount(2.0);
		copy.set_weapon(EntityHandle::from_raw(17 | 3 << 16));
		assert_eq!(copy.amount(), 48.0);
		assert_eq!(original.amount(), 24.0);
		assert_eq!(copy.damage_type().0, (1 << 31) | 2);
		assert_eq!(original.weapon(), EntityHandle::INVALID);
		assert_eq!(copy.weapon().serial_number(), 3);
	}

	#[test]
	fn critical_policy_clamps_overlarge_recorded_bonus() {
		let mut damage = DamageInfo::new(10.0, DamageType::CRITICAL);
		damage.write_critical(CriticalHit::Full);
		damage.set_damage_bonus(50.0);
		assert!(damage.apply_critical_policy(CriticalPolicy::DISABLE_ALL));
		assert_eq!(damage.amount(), 0.0);
	}

	#[test]
	fn incoming_crit_changes_are_exclusive_and_do_not_scale_damage() {
		let mut damage = DamageInfo::new(10.0, DamageType::BLAST);
		for critical in [CriticalHit::Full, CriticalHit::Mini, CriticalHit::None] {
			damage.set_incoming_critical(critical);
			assert_eq!(damage.critical_hit(), Some(critical));
			assert_eq!(
				damage.damage_type().contains(DamageType::CRITICAL),
				critical == CriticalHit::Full
			);
			assert!(damage.damage_type().contains(DamageType::BLAST));
			assert_eq!(damage.amount(), 10.0);
		}
	}

	#[test]
	fn independent_policies_remove_bonus_only_once() {
		for (critical, policy, denied) in [
			(
				CriticalHit::Full,
				CriticalPolicy {
					full: false,
					mini: true,
				},
				true,
			),
			(
				CriticalHit::Mini,
				CriticalPolicy {
					full: false,
					mini: true,
				},
				false,
			),
			(
				CriticalHit::Full,
				CriticalPolicy {
					full: true,
					mini: false,
				},
				false,
			),
			(
				CriticalHit::Mini,
				CriticalPolicy {
					full: true,
					mini: false,
				},
				true,
			),
		] {
			let mut damage = DamageInfo::new(135.0, DamageType::BULLET | DamageType::CRITICAL);
			damage.write_critical(critical);
			damage.set_damage_bonus(35.0);
			assert_eq!(damage.apply_critical_policy(policy), denied);
			assert_eq!(damage.amount(), if denied { 100.0 } else { 135.0 });
			assert!(!damage.apply_critical_policy(policy));
			if denied {
				assert_eq!(damage.critical_hit(), Some(CriticalHit::None));
				assert!(!damage.damage_type().contains(DamageType::CRITICAL));
			}
		}
	}

	#[test]
	fn invalid_edits_leave_the_prior_record_intact() {
		let mut damage = DamageInfo::new(10.0, DamageType::GENERIC);
		for value in [f32::NAN, f32::INFINITY, -1.0, i32::MAX as f32] {
			assert!(
				std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| damage.set_amount(value)))
					.is_err()
			);
			assert_eq!(damage.amount(), 10.0);
		}
		assert!(
			std::panic::catch_unwind(std::panic::AssertUnwindSafe(
				|| damage.set_attacker(EntityHandle::from_raw(EntityHandle::SLOTS as u32))
			))
			.is_err()
		);
		assert_eq!(damage.attacker(), EntityHandle::INVALID);
	}
}
