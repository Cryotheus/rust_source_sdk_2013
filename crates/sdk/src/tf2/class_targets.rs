//! TF2's entity classes, whose primary vtables class-wide hooks patch, found
//! with or without an entity of them.
//!
//! The game calls the virtual methods of an entity through the primary vtable
//! of its class. A hook on that table covers every entity of the class, those
//! created later included, but not those of the classes deriving from it,
//! which have tables of their own: TF2's `tf_bot_add` bots, of the class
//! `CTFBot`, have a table apart from that of its human players, `CTFPlayer`.
//!
//! `metamod_source`'s typed hooks take a [`ClassTarget`]: a class's table,
//! known to belong to a class of the [`ClassKind`] whose methods they hook,
//! such as [`TfPlayer`]. There are three ways to one:
//!
//! - [`ClassTargets`] snapshots the game module, and finds the classes the
//!   SDK knows by name before any entity of them exists, such as the players'
//!   with [`ClassTargets::players`] and the buildings' with
//!   [`ClassTargets::objects`].
//! - [`ClassTarget::of`] gives the class of a live entity, if its data
//!   description maps show it is of the kind. Hooks then cover each class as
//!   its first entity appears, for kinds of too many classes to name ahead,
//!   such as TF2's weapons.
//! - [`ClassTargets::find`] finds any class by name as a [`ClassVtable`],
//!   whose kind the search cannot tell: [`ClassTarget::from_vtable`] trusts
//!   the caller to know it.
//!
//! The snapshot reads the whole game module, a few times per search however
//! many classes it finds, so take one, and find every class needed with it,
//! as the plugin loads.

#[cfg(test)]
#[path = "../tests/tf2/class_targets.rs"]
mod tests;

use crate::entities::Entity;
use crate::{Game, Server};
use sdk_raw::abi::VTABLE_SLOT_SIZE;
use sdk_raw::tf2::class_targets::{ClassVtables, OBJECT_CLASSES, PLAYER_CLASSES};
use sdk_raw::util;
use sdk_raw::util::vtable::vtable_pointer;
use std::ffi::{CStr, c_void};
use std::fmt::{self, Debug, Formatter};
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::ptr::NonNull;

/// `CBaseEntity`: every entity.
#[doc(alias("CBaseEntity"))]
#[derive(Debug)]
pub enum BaseEntity {}

/// `CBaseObject`: TF2's buildings, those of
/// [`OBJECT_CLASSES`](sdk_raw::tf2::class_targets::OBJECT_CLASSES), which are
/// [`CombatCharacter`]s.
#[doc(alias("CBaseObject"))]
#[derive(Debug)]
pub enum BaseObject {}

/// A C++ class of TF2's entities, by which [`ClassTarget`]s are typed: a
/// target of a kind is the table of the kind's class, or of a class deriving
/// from it through its primary bases.
pub trait ClassKind: sealed::Sealed + Debug + 'static {
	/// The class's C++ name, which its data description map has too, such as
	/// `CTFPlayer`.
	const NAME: &'static CStr;

	/// How many slots the class's primary vtable has, from the generated
	/// binding. The tables of the classes deriving from it have as many, or
	/// more.
	const VTABLE_SLOTS: usize;
}

/// `CBaseCombatCharacter`: the entities that take damage while alive, and
/// can hold weapons. In TF2, its players and buildings.
#[doc(alias("CBaseCombatCharacter"))]
#[derive(Debug)]
pub enum CombatCharacter {}

/// `CBaseCombatWeapon`: the weapons players hold, which in TF2 are all
/// [`TfWeapon`]s.
#[doc(alias("CBaseCombatWeapon"))]
#[derive(Debug)]
pub enum CombatWeapon {}

/// A [`ClassKind`] whose classes are all of the kind `B` too, as they derive
/// from its class through their primary bases.
pub trait DerivesFrom<B: ClassKind>: ClassKind {}

impl<K: ClassKind> DerivesFrom<K> for K {}

/// `CTFPlayer`: TF2's players, of the two classes of
/// [`PLAYER_CLASSES`](sdk_raw::tf2::class_targets::PLAYER_CLASSES).
#[doc(alias("CTFPlayer", "CTFBot"))]
#[derive(Debug)]
pub enum TfPlayer {}

/// `CTFWeaponBase`: TF2's weapons, from the Scattergun to the Engineer's
/// toolbox and the Spy's sapper.
#[doc(alias("CTFWeaponBase"))]
#[derive(Debug)]
pub enum TfWeapon {}

macro_rules! kinds {
	($($kind:ident: $name:literal, $vtable:ty, [$($base:ident),*];)*) => {$(
		impl sealed::Sealed for $kind {}

		impl ClassKind for $kind {
			const NAME: &'static CStr = $name;
			const VTABLE_SLOTS: usize = size_of::<$vtable>() / VTABLE_SLOT_SIZE;
		}

		$(impl DerivesFrom<$base> for $kind {})*
	)*};
}

kinds! {
	BaseEntity: c"CBaseEntity", sys::CBaseEntity__bindgen_vtable, [];
	BaseObject: c"CBaseObject", sys::CBaseObject__bindgen_vtable, [BaseEntity, CombatCharacter];
	CombatCharacter: c"CBaseCombatCharacter", sys::CBaseCombatCharacter__bindgen_vtable, [BaseEntity];
	CombatWeapon: c"CBaseCombatWeapon", sys::CBaseCombatWeapon__bindgen_vtable, [BaseEntity];
	TfPlayer: c"CTFPlayer", sys::CTFPlayer__bindgen_vtable, [BaseEntity, CombatCharacter];
	TfWeapon: c"CTFWeaponBase", sys::CTFWeaponBase__bindgen_vtable, [BaseEntity, CombatWeapon];
}

/// The primary vtable of a C++ class of the kind `K` in this server's game
/// module, through which the game calls the virtual methods of the class's
/// entities, but not those of the classes deriving from it.
pub struct ClassTarget<'s, K: ClassKind> {
	vtable: NonNull<*mut c_void>,
	_kind: PhantomData<fn() -> K>,
	_scope: PhantomData<&'s Server<'s>>,
}

impl<'s, K: ClassKind> ClassTarget<'s, K> {
	/// The class whose primary vtable `vtable` is.
	///
	/// # Safety
	///
	/// `vtable` must be the primary vtable of a class of the kind `K` in the game
	/// module of this server, which runs TF2, as that of a live entity
	/// [`Self::of`] accepts is.
	pub const unsafe fn from_raw(vtable: NonNull<*mut c_void>) -> Self {
		Self {
			vtable,
			_kind: PhantomData,
			_scope: PhantomData,
		}
	}

	/// The class [`ClassTargets::find`] found, as a class of the kind `K`.
	///
	/// # Safety
	///
	/// The class must be of the kind `K`, deriving from its class through its
	/// primary bases. The search only checked that its table holds code at the
	/// slot it was given.
	pub const unsafe fn from_vtable(vtable: ClassVtable<'s>) -> Self {
		// SAFETY: As the caller promises.
		unsafe { Self::from_raw(vtable.as_ptr()) }
	}

	/// The class of `entity`, if the server runs TF2 and the entity is of the
	/// kind `K`: if the data description map of its class, or of one of its
	/// bases, is named after `K`'s class. An entity class's maps name the
	/// classes it derives from through its primary bases, as `DECLARE_CLASS`
	/// and `BEGIN_DATADESC` chain them.
	pub fn of(server: Server<'s>, entity: Entity<'s>) -> Option<Self> {
		if server.game() != Game::TeamFortress2 || !entity.has_data_map_class(K::NAME) {
			return None;
		}

		// SAFETY: A live entity starts with the pointer to its primary vtable,
		// of which only the address is used.
		let vtable = unsafe { vtable_pointer::<*mut c_void>(entity.as_ptr()) };

		NonNull::new(vtable.cast_mut()).map(|vtable| Self {
			vtable,
			_kind: PhantomData,
			_scope: PhantomData,
		})
	}

	/// The vtable's address in the game module.
	pub const fn as_ptr(self) -> NonNull<*mut c_void> {
		self.vtable
	}

	/// Whether `entity` is of this class, and not of one deriving from it.
	pub fn is_class_of(self, entity: Entity<'_>) -> bool {
		// SAFETY: As for `Self::of`.
		let vtable = unsafe { vtable_pointer::<*mut c_void>(entity.as_ptr()) };

		vtable.cast_mut() == self.vtable.as_ptr()
	}

	/// The same class, as one of the kind `B`, which `K`'s classes are too.
	pub const fn upcast<B: ClassKind>(self) -> ClassTarget<'s, B>
	where
		K: DerivesFrom<B>,
	{
		ClassTarget {
			vtable: self.vtable,
			_kind: PhantomData,
			_scope: PhantomData,
		}
	}
}

impl<K: ClassKind> Clone for ClassTarget<'_, K> {
	fn clone(&self) -> Self {
		*self
	}
}

impl<K: ClassKind> Copy for ClassTarget<'_, K> {}

impl<K: ClassKind> Debug for ClassTarget<'_, K> {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		f.debug_struct("ClassTarget")
			.field("kind", &K::NAME)
			.field("vtable", &self.vtable)
			.finish()
	}
}

impl<K: ClassKind> Eq for ClassTarget<'_, K> {}

impl<K: ClassKind> Hash for ClassTarget<'_, K> {
	fn hash<H: Hasher>(&self, state: &mut H) {
		self.vtable.hash(state);
	}
}

impl<K: ClassKind> PartialEq for ClassTarget<'_, K> {
	fn eq(&self, other: &Self) -> bool {
		self.vtable == other.vtable
	}
}

/// Why the game module could not be searched for classes, or a class the SDK
/// knows was not found in it.
#[derive(Debug, thiserror::Error)]
pub enum ClassTargetError {
	/// The server does not run Team Fortress 2.
	#[error("class targets require Team Fortress 2")]
	WrongGame,

	/// The game module could not be read.
	#[error("the game module could not be inspected")]
	Image(#[from] std::io::Error),

	/// The game module is not an executable image the vtable search supports.
	#[error("the game module has an unsupported executable image")]
	InvalidImage,

	/// The class named so has no unique primary vtable in the game module.
	#[error("no unique primary vtable found for TF2's class `{0}`")]
	NotFound(&'static str),
}

impl From<util::Error> for ClassTargetError {
	fn from(error: util::Error) -> Self {
		match error {
			util::Error::InvalidImage => Self::InvalidImage,
			util::Error::Io(error) => Self::Image(error),
		}
	}
}

/// A snapshot of TF2's game module, in which to find the vtables of its
/// classes.
#[derive(Debug, Clone)]
pub struct ClassTargets<'s> {
	vtables: ClassVtables,
	_scope: PhantomData<&'s Server<'s>>,
}

impl<'s> ClassTargets<'s> {
	/// Snapshots the server's game module.
	pub fn load(server: Server<'s>) -> Result<Self, ClassTargetError> {
		if server.game() != Game::TeamFortress2 {
			return Err(ClassTargetError::WrongGame);
		}

		// SAFETY: The game server factory is the game module's `CreateInterface`,
		// and the Server's callback scope keeps the module loaded while its sections
		// are inspected (`Server::new` condition 1).
		let vtables = unsafe { ClassVtables::load(server.game_server_factory().as_raw()) }?;

		Ok(Self {
			vtables,
			_scope: PhantomData,
		})
	}

	/// The vtable of the global C++ class named `class`, such as
	/// `CTFAmmoPack` for `tf_ammo_pack`, from its run-time type information,
	/// if it holds code at `slot`. Returns `None` if the module has no such
	/// class, or more than one. What the class derives from, and so what the
	/// slot holds, is the caller's to know.
	///
	/// Each search reads the whole snapshot a few times, so find the classes
	/// needed together with [`Self::find_all`].
	pub fn find(&self, class: &str, slot: usize) -> Option<ClassVtable<'s>> {
		self.vtables.find(class, slot).map(|vtable| ClassVtable {
			vtable,
			_scope: PhantomData,
		})
	}

	/// The vtables [`Self::find`] finds for each of `classes`, in their order.
	/// The snapshot is read as often for every class as [`Self::find`] reads
	/// it for one.
	pub fn find_all(&self, classes: &[&str], slot: usize) -> Vec<Option<ClassVtable<'s>>> {
		self.vtables
			.find_all(classes, slot)
			.into_iter()
			.map(|vtable| {
				vtable.map(|vtable| ClassVtable {
					vtable,
					_scope: PhantomData,
				})
			})
			.collect()
	}

	/// The classes of each of `classes`, in their order, as classes of the
	/// kind `K`: [`Self::find_all`]'s, whose tables hold code at the last of
	/// `K`'s slots.
	///
	/// # Safety
	///
	/// Each class found must be of the kind `K`, deriving from its class
	/// through its primary bases.
	pub unsafe fn find_kind<K: ClassKind>(
		&self,
		classes: &[&str],
	) -> Vec<Option<ClassTarget<'s, K>>> {
		self.find_all(classes, K::VTABLE_SLOTS - 1)
			.into_iter()
			// SAFETY: As the caller promises.
			.map(|vtable| vtable.map(|vtable| unsafe { ClassTarget::from_vtable(vtable) }))
			.collect()
	}

	/// The classes of TF2's buildings, those of
	/// [`OBJECT_CLASSES`](sdk_raw::tf2::class_targets::OBJECT_CLASSES), in its
	/// order. A class without a unique vtable is an error.
	pub fn objects(
		&self,
	) -> Result<[ClassTarget<'s, BaseObject>; OBJECT_CLASSES.len()], ClassTargetError> {
		// SAFETY: Each of TF2's building classes derives from `CBaseObject`.
		let found = unsafe { self.find_kind(&OBJECT_CLASSES) };

		known(found, &OBJECT_CLASSES)
	}

	/// The classes of TF2's players, those of
	/// [`PLAYER_CLASSES`](sdk_raw::tf2::class_targets::PLAYER_CLASSES): the
	/// humans', then the bots'. A class without a unique vtable is an error.
	pub fn players(
		&self,
	) -> Result<[ClassTarget<'s, TfPlayer>; PLAYER_CLASSES.len()], ClassTargetError> {
		// SAFETY: `CTFPlayer` is TF2's player class, and `CTFBot` derives from
		// it through its primary bases.
		let found = unsafe { self.find_kind(&PLAYER_CLASSES) };

		known(found, &PLAYER_CLASSES)
	}
}

/// The primary vtable of a C++ class in this server's game module, found by
/// name with [`ClassTargets::find`]. For an entity class, the game calls the
/// virtual methods of the class's entities through it, but not those of the
/// classes deriving from it.
///
/// The search finds any polymorphic class by name: hooking it as an entity
/// class is only sound for one deriving from `CBaseEntity`, and as a class of
/// a [`ClassKind`] for one of that kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClassVtable<'s> {
	vtable: NonNull<*mut c_void>,
	_scope: PhantomData<&'s Server<'s>>,
}

impl ClassVtable<'_> {
	/// The vtable's address in the game module.
	pub const fn as_ptr(self) -> NonNull<*mut c_void> {
		self.vtable
	}
}

/// A [`ClassTargets`] snapshot that finds the classes holding a function at
/// `SLOT`, as the target types of the hook modules that came before it do,
/// such as [`TouchTargets`](super::touch::TouchTargets).
#[derive(Debug, Clone)]
pub struct SlotTargets<'s, const SLOT: usize>(ClassTargets<'s>);

impl<'s, const SLOT: usize> SlotTargets<'s, SLOT> {
	/// Snapshots the server's game module.
	pub fn load(server: Server<'s>) -> Result<Self, ClassTargetError> {
		ClassTargets::load(server).map(Self)
	}

	/// The vtable of the global C++ class named `class` that holds code at
	/// `SLOT`, as [`ClassTargets::find`] finds it. Returns `None` if the
	/// module has no such class, or more than one. Whether the class is an
	/// entity class is the caller's to know.
	///
	/// Each search reads the whole snapshot a few times, so find the classes
	/// needed together with [`Self::find_all`].
	pub fn find(&self, class: &str) -> Option<ClassVtable<'s>> {
		self.0.find(class, SLOT)
	}

	/// The vtables [`Self::find`] finds for each of `classes`, in their order.
	/// The snapshot is read as often for every class as [`Self::find`] reads
	/// it for one.
	pub fn find_all(&self, classes: &[&str]) -> Vec<Option<ClassVtable<'s>>> {
		self.0.find_all(classes, SLOT)
	}
}

impl<'s, const SLOT: usize> From<ClassTargets<'s>> for SlotTargets<'s, SLOT> {
	/// Searches the snapshot for the classes holding a function at `SLOT`,
	/// without taking another.
	fn from(targets: ClassTargets<'s>) -> Self {
		Self(targets)
	}
}

/// The classes the SDK knows, in the order of their `names`, or an error
/// naming the first one missing.
fn known<'s, K: ClassKind, const N: usize>(
	found: Vec<Option<ClassTarget<'s, K>>>,
	names: &[&'static str; N],
) -> Result<[ClassTarget<'s, K>; N], ClassTargetError> {
	let found: Vec<_> = found
		.into_iter()
		.zip(names)
		.map(|(target, name)| target.ok_or(ClassTargetError::NotFound(name)))
		.collect::<Result<_, _>>()?;

	Ok(found
		.try_into()
		.unwrap_or_else(|_| unreachable!("one class is found for each name")))
}

mod sealed {
	pub trait Sealed {}
}
