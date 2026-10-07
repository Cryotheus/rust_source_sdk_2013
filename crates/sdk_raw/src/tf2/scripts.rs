//! Hand-written ABI of the VScripts TF2's entities run: the vtable slot of
//! `CBaseEntity::RunVScripts`, through which the game runs the scripts an
//! entity's `vscripts` key value names, its signature, and the search for an
//! entity class's vtable.
//!
//! The game calls `RunVScripts` as it spawns an entity (`DispatchSpawn`),
//! before the entity's own `Spawn`: for the map's entities as a level loads,
//! again for those a round's restart creates, and for those spawned later.
//! Unless the entity names no script, it gives the entity its script scope,
//! runs each script in it, and starts the entity's `thinkfunction`. The
//! scripts' `Precache` and `OnPostSpawn` run later in the spawn, also only for
//! an entity that names a script. `logic_script`'s `CLogicScript` overrides
//! it, to give its scripts the entities its group key values name first.

use crate::tf2::class_targets::SlotVtables;
use crate::vtable_slot;

/// The signature of `CBaseEntity::RunVScripts`, `void ()`, which runs the
/// scripts the entity's `vscripts` key value names.
#[doc(alias("RunVScripts"))]
pub type RunVScriptsFn = unsafe extern "C" fn(this: *mut sys::CBaseEntity);

/// An owned snapshot of TF2's game server module, in which to find the
/// primary vtables of its entity classes, through which the game calls
/// `RunVScripts` on their entities: those whose [`RUN_VSCRIPTS_SLOT`] entries
/// are executable.
pub type ScriptVtables = SlotVtables<RUN_VSCRIPTS_SLOT>;

// `RunVScripts` follows `ChangeTeam`, which follows the methods `TF_DLL` and
// `NEXT_BOT` declare, so its slot is TF2's alone.
const _: () = assert!(
	vtable_slot!(sys::CBaseEntity__bindgen_vtable, CBaseEntity_ChangeTeam) < RUN_VSCRIPTS_SLOT
);

// The generated method has the hand-written signature.
const _: fn(&sys::CBaseEntity__bindgen_vtable) -> RunVScriptsFn =
	|vtable| vtable.CBaseEntity_RunVScripts;

/// The slot of `CBaseEntity::RunVScripts` in a TF2 entity's primary vtable,
/// from the generated binding. The classes that override it keep the slot.
///
/// Methods `TF_DLL` and `NEXT_BOT` declare come before it, so other games'
/// entities have it elsewhere.
#[doc(alias("RunVScripts"))]
pub const RUN_VSCRIPTS_SLOT: usize =
	vtable_slot!(sys::CBaseEntity__bindgen_vtable, CBaseEntity_RunVScripts);
