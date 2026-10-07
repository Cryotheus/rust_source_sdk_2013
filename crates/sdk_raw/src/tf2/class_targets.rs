//! Hand-written ABI of class-wide hooks: the search for the primary vtables
//! of TF2's classes by their C++ names, and the names of the classes the SDK
//! knows ahead.
//!
//! The game calls the virtual methods of an entity through the primary vtable
//! of its class. A hook patched into that table covers every entity of the
//! class, those created later included, but not those of the classes deriving
//! from it, which have tables of their own. [`ClassVtables`] finds a class's
//! table by name in the run-time type information of a snapshot of the game
//! module, which needs no entity of the class. [`SlotVtables`] is the same
//! search for the classes holding one function, as each of the hook modules
//! that came before it searches.

use crate::interfaces::CreateInterfaceFn;
use crate::util::{self, Image};
use std::ffi::c_void;
use std::ptr::NonNull;

/// The C++ classes of TF2's buildings (`CBaseObject`), whose entities
/// Engineers build and Spies place: `obj_dispenser`, the payload carts'
/// `mapobj_cart_dispenser`, `obj_sentrygun`, `obj_teleporter` and
/// `obj_attachment_sapper` (`tf_obj_dispenser.h:87,215`,
/// `tf_obj_sentrygun.h:42`, `tf_obj_teleporter.h:31`, `tf_obj_sapper.h:29`).
#[doc(alias("CBaseObject"))]
pub const OBJECT_CLASSES: [&str; 5] = [
	"CObjectDispenser",
	"CObjectCartDispenser",
	"CObjectSentrygun",
	"CObjectTeleporter",
	"CObjectSapper",
];

/// The C++ classes of TF2's players: `CTFPlayer`, the class of human players
/// and of the bots a plugin or the `bot` command adds, and `CTFBot`, the class
/// of the bots `tf_bot_add` adds, which derives from it through
/// `NextBotPlayer<CTFPlayer>` and has a vtable of its own.
#[doc(alias("CTFPlayer", "CTFBot"))]
pub const PLAYER_CLASSES: [&str; 2] = ["CTFPlayer", "CTFBot"];

/// An owned snapshot of TF2's game server module, in which to find the
/// primary vtables of its classes.
#[derive(Debug, Clone)]
pub struct ClassVtables(Image);

impl ClassVtables {
	/// Snapshots the module whose `CreateInterface` export is `factory`, such
	/// as the game server module.
	///
	/// # Safety
	///
	/// `factory` must be the `CreateInterface` export of a module that stays
	/// loaded throughout this call.
	pub unsafe fn load(factory: CreateInterfaceFn) -> Result<Self, util::Error> {
		// SAFETY: The factory is an executable address in its module, which the
		// caller keeps loaded while it is inspected.
		unsafe { Image::load(factory as usize) }.map(Self)
	}

	/// The unique primary vtable of the global C++ class named `class`, such
	/// as `CTFAmmoPack`, whose entry at `slot` is executable, from its
	/// run-time type information. Returns `None` if there is no such table or
	/// more than one.
	///
	/// The search does not check what the class derives from, so what the slot
	/// holds: any class with that many virtual methods passes. The table is
	/// only the class's own: classes deriving from it have tables of their
	/// own. The address is metadata from the snapshot: it does not keep the
	/// module loaded, and the table is the class's only while the module that
	/// [`Self::load`] snapshot stays loaded.
	///
	/// Each search reads the whole snapshot a few times, so find the classes
	/// needed together with [`Self::find_all`].
	pub fn find(&self, class: &str, slot: usize) -> Option<NonNull<*mut c_void>> {
		NonNull::new(self.0.primary_vtable(class, slot)? as *mut *mut c_void)
	}

	/// The tables [`Self::find`] finds for each of `classes`, in their order.
	/// The snapshot is read as often for every class as [`Self::find`] reads
	/// it for one.
	pub fn find_all(&self, classes: &[&str], slot: usize) -> Vec<Option<NonNull<*mut c_void>>> {
		self.0
			.primary_vtables(classes, slot)
			.into_iter()
			.map(|table| NonNull::new(table? as *mut *mut c_void))
			.collect()
	}
}

/// A [`ClassVtables`] snapshot that finds the classes holding a function at
/// `SLOT`, such as [`TOUCH_SLOT`](crate::tf2::touch::TOUCH_SLOT) for the
/// classes [`TouchVtables`](crate::tf2::touch::TouchVtables) finds.
#[derive(Debug, Clone)]
pub struct SlotVtables<const SLOT: usize>(ClassVtables);

impl<const SLOT: usize> SlotVtables<SLOT> {
	/// Snapshots the module whose `CreateInterface` export is `factory`, such
	/// as the game server module.
	///
	/// # Safety
	///
	/// `factory` must be the `CreateInterface` export of a module that stays
	/// loaded throughout this call.
	pub unsafe fn load(factory: CreateInterfaceFn) -> Result<Self, util::Error> {
		// SAFETY: As the caller promises.
		unsafe { ClassVtables::load(factory) }.map(Self)
	}

	/// The unique primary vtable of the global C++ class named `class` whose
	/// entry at `SLOT` is executable, as [`ClassVtables::find`] finds it.
	///
	/// Each search reads the whole snapshot a few times, so find the classes
	/// needed together with [`Self::find_all`].
	pub fn find(&self, class: &str) -> Option<NonNull<*mut c_void>> {
		self.0.find(class, SLOT)
	}

	/// The tables [`Self::find`] finds for each of `classes`, in their order.
	/// The snapshot is read as often for every class as [`Self::find`] reads
	/// it for one.
	pub fn find_all(&self, classes: &[&str]) -> Vec<Option<NonNull<*mut c_void>>> {
		self.0.find_all(classes, SLOT)
	}
}

impl<const SLOT: usize> From<ClassVtables> for SlotVtables<SLOT> {
	/// Searches the snapshot for the classes holding a function at `SLOT`,
	/// without taking another.
	fn from(vtables: ClassVtables) -> Self {
		Self(vtables)
	}
}
