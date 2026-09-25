//! Agabaraya.
#![cfg_attr(docsrs, feature(doc_cfg))]

use std::mem::offset_of;

const SLOT_SIZE: usize = size_of::<*const ()>();

cfg_select! {
	all(target_os = "linux", target_arch = "x86", target_env = "gnu") => {
		compile_error!("Not yet supported");
	}

	all(target_os = "linux", target_arch = "x86_64", target_env = "gnu") => {
		mod linux_64;

		pub use linux_64::*;

		/// `CBaseEntity::GetDataDescMap` in the Source SDK 2013 primary vtable.
		/// Derived from `game/server/cbase.h` with the Itanium ABI model.
		pub const CBASEENTITY_DATAMAP_VTABLE_SLOT: usize = 12;
		/// `CBaseEntity::Teleport` in the Source SDK 2013 primary vtable.
		/// Derived from `game/server/cbase.h` with the Itanium ABI model.
		pub const CBASEENTITY_TELEPORT_VTABLE_SLOT: usize = 111;
		/// `CBaseEntity::Teleport` in TF2's game DLL.
		/// Verified against SourceMod's `sdktools.games/game.tf.txt` gamedata.
		pub const CBASEENTITY_TF2_TELEPORT_VTABLE_SLOT: usize = 115;

		const _: () = {
			assert!(
				offset_of!(
					IGameEventVisitor2__bindgen_vtable,
					IGameEventVisitor2_VisitLocal
				) == 0
			);
			assert!(
				offset_of!(
					IGameEventVisitor2__bindgen_vtable,
					IGameEventVisitor2_VisitBool
				) == SLOT_SIZE * 6
			);
			assert!(size_of::<IGameEventVisitor2__bindgen_vtable>() == SLOT_SIZE * 7);

			assert!(offset_of!(IGameEvent__bindgen_vtable, IGameEvent_complete_destructor) == 0);
			assert!(
				offset_of!(IGameEvent__bindgen_vtable, IGameEvent_deleting_destructor) == SLOT_SIZE
			);
			assert!(
				offset_of!(IGameEvent__bindgen_vtable, IGameEvent_ForEventData) == SLOT_SIZE * 21
			);
			assert!(size_of::<IGameEvent__bindgen_vtable>() == SLOT_SIZE * 22);

			assert!(
				offset_of!(
					IGameEventListener2__bindgen_vtable,
					IGameEventListener2_complete_destructor
				) == 0
			);
			assert!(
				offset_of!(
					IGameEventListener2__bindgen_vtable,
					IGameEventListener2_deleting_destructor
				) == SLOT_SIZE
			);
			assert!(
				offset_of!(
					IGameEventListener2__bindgen_vtable,
					IGameEventListener2_FireGameEvent
				) == SLOT_SIZE * 2
			);
			assert!(size_of::<IGameEventListener2__bindgen_vtable>() == SLOT_SIZE * 3);

			// The Itanium ABI places the complete-object destructor first and the
			// deleting destructor second, followed by the declared virtual methods.
			assert!(
				offset_of!(
					IGameEventManager2__bindgen_vtable,
					IGameEventManager2_complete_destructor
				) == 0
			);
			assert!(
				offset_of!(
					IGameEventManager2__bindgen_vtable,
					IGameEventManager2_deleting_destructor
				) == SLOT_SIZE
			);
			assert!(
				offset_of!(
					IGameEventManager2__bindgen_vtable,
					IGameEventManager2_LoadEventsFromFile
				) == SLOT_SIZE * 2
			);
			assert!(
				offset_of!(
					IGameEventManager2__bindgen_vtable,
					IGameEventManager2_AddListener
				) == SLOT_SIZE * 4
			);
			assert!(
				offset_of!(
					IGameEventManager2__bindgen_vtable,
					IGameEventManager2_RemoveListener
				) == SLOT_SIZE * 6
			);
			assert!(
				offset_of!(
					IGameEventManager2__bindgen_vtable,
					IGameEventManager2_UnserializeEvent
				) == SLOT_SIZE * 13
			);
			assert!(size_of::<IGameEventManager2__bindgen_vtable>() == SLOT_SIZE * 14);

			//no virtual destructor
			assert!(
				offset_of!(IVEngineServer__bindgen_vtable, IVEngineServer_ServerCommand)
					== SLOT_SIZE * 36
			);

			assert!(size_of::<IServerTools>() == SLOT_SIZE);
			assert!(
				offset_of!(
					IServerTools__bindgen_vtable,
					IServerTools_complete_destructor
				) == 0
			);
			assert!(
				offset_of!(
					IServerTools__bindgen_vtable,
					IServerTools_deleting_destructor
				) == SLOT_SIZE
			);
			assert!(
				offset_of!(IServerTools__bindgen_vtable, IServerTools_FirstEntity) == SLOT_SIZE * 8
			);
			assert!(
				offset_of!(IServerTools__bindgen_vtable, IServerTools_NextEntity) == SLOT_SIZE * 9
			);
			assert!(
				offset_of!(IServerTools__bindgen_vtable, IServerTools_GetKeyValue)
					== SLOT_SIZE * 11
			);
			assert!(
				offset_of!(
					IServerTools__bindgen_vtable,
					IServerTools_FindEntityProcedural
				) == SLOT_SIZE * 52
			);
			assert!(size_of::<IServerTools__bindgen_vtable>() == SLOT_SIZE * 53);

			// The Itanium ABI keeps overloads in declaration order.
			assert!(
				offset_of!(IServerTools__bindgen_vtable, IServerTools_SetKeyValue)
					== SLOT_SIZE * 12
			);
			assert!(
				offset_of!(IServerTools__bindgen_vtable, IServerTools_SetKeyValue2)
					== SLOT_SIZE * 14
			);

			//no virtual destructor
			assert!(
				offset_of!(
					IServerGameDLL__bindgen_vtable,
					IServerGameDLL_GetAllServerClasses
				) == SLOT_SIZE * 11
			);
			assert!(size_of::<IServerGameDLL__bindgen_vtable>() == SLOT_SIZE * 48);
		};
	}

	all(target_os = "windows", target_arch = "x86", target_env = "msvc") => {
		compile_error!("Not yet supported");
	}

	all(target_os = "windows", target_arch = "x86_64", target_env = "msvc") => {
		mod windows_64;

		pub use windows_64::*;

		/// `CBaseEntity::GetDataDescMap` in the Source SDK 2013 primary vtable.
		/// Derived from `game/server/cbase.h` with the MSVC ABI model.
		pub const CBASEENTITY_DATAMAP_VTABLE_SLOT: usize = 11;
		/// `CBaseEntity::Teleport` in the Source SDK 2013 primary vtable.
		/// Derived from `game/server/cbase.h` with the MSVC ABI model.
		pub const CBASEENTITY_TELEPORT_VTABLE_SLOT: usize = 110;
		/// `CBaseEntity::Teleport` in TF2's game DLL.
		/// Verified against SourceMod's `sdktools.games/game.tf.txt` gamedata.
		pub const CBASEENTITY_TF2_TELEPORT_VTABLE_SLOT: usize = 114;

		const _: () = {
			assert!(
				offset_of!(
					IGameEventVisitor2__bindgen_vtable,
					IGameEventVisitor2_VisitLocal
				) == 0
			);
			assert!(
				offset_of!(
					IGameEventVisitor2__bindgen_vtable,
					IGameEventVisitor2_VisitBool
				) == SLOT_SIZE * 6
			);
			assert!(size_of::<IGameEventVisitor2__bindgen_vtable>() == SLOT_SIZE * 7);

			assert!(offset_of!(IGameEvent__bindgen_vtable, IGameEvent_destructor) == 0);
			assert!(
				offset_of!(IGameEvent__bindgen_vtable, IGameEvent_ForEventData) == SLOT_SIZE * 20
			);
			assert!(size_of::<IGameEvent__bindgen_vtable>() == SLOT_SIZE * 21);

			assert!(
				offset_of!(
					IGameEventListener2__bindgen_vtable,
					IGameEventListener2_destructor
				) == 0
			);
			assert!(
				offset_of!(
					IGameEventListener2__bindgen_vtable,
					IGameEventListener2_FireGameEvent
				) == SLOT_SIZE
			);
			assert!(size_of::<IGameEventListener2__bindgen_vtable>() == SLOT_SIZE * 2);

			// IGameEventManager2 overrides IBaseInterface's virtual destructor,
			// followed by the twelve methods declared in igameevents.h.
			assert!(
				offset_of!(
					IGameEventManager2__bindgen_vtable,
					IGameEventManager2_destructor
				) == 0
			);
			assert!(
				offset_of!(
					IGameEventManager2__bindgen_vtable,
					IGameEventManager2_LoadEventsFromFile
				) == SLOT_SIZE
			);
			assert!(
				offset_of!(
					IGameEventManager2__bindgen_vtable,
					IGameEventManager2_AddListener
				) == SLOT_SIZE * 3
			);
			assert!(
				offset_of!(
					IGameEventManager2__bindgen_vtable,
					IGameEventManager2_RemoveListener
				) == SLOT_SIZE * 5
			);
			assert!(
				offset_of!(
					IGameEventManager2__bindgen_vtable,
					IGameEventManager2_UnserializeEvent
				) == SLOT_SIZE * 12
			);
			assert!(size_of::<IGameEventManager2__bindgen_vtable>() == SLOT_SIZE * 13);

			//no virtual destructor
			assert!(
				offset_of!(IVEngineServer__bindgen_vtable, IVEngineServer_ServerCommand)
					== SLOT_SIZE * 36
			);

			assert!(size_of::<IServerTools>() == SLOT_SIZE);
			assert!(offset_of!(IServerTools__bindgen_vtable, IServerTools_destructor) == 0);
			assert!(
				offset_of!(IServerTools__bindgen_vtable, IServerTools_FirstEntity) == SLOT_SIZE * 7
			);
			assert!(
				offset_of!(IServerTools__bindgen_vtable, IServerTools_NextEntity) == SLOT_SIZE * 8
			);
			assert!(
				offset_of!(IServerTools__bindgen_vtable, IServerTools_GetKeyValue)
					== SLOT_SIZE * 10
			);
			assert!(
				offset_of!(
					IServerTools__bindgen_vtable,
					IServerTools_FindEntityProcedural
				) == SLOT_SIZE * 51
			);
			assert!(size_of::<IServerTools__bindgen_vtable>() == SLOT_SIZE * 52);

			// MSVC emits each new overload set in reverse declaration order, so
			// `SetKeyValue(const char *, const Vector &)` precedes the `const
			// char *` overload. SourceMod's `DispatchKeyValue*` gamedata shows
			// the same reversal for `CBaseEntity::KeyValue`.
			assert!(
				offset_of!(IServerTools__bindgen_vtable, IServerTools_SetKeyValue2)
					== SLOT_SIZE * 11
			);
			assert!(
				offset_of!(IServerTools__bindgen_vtable, IServerTools_SetKeyValue)
					== SLOT_SIZE * 13
			);

			//no virtual destructor
			assert!(
				offset_of!(
					IServerGameDLL__bindgen_vtable,
					IServerGameDLL_GetAllServerClasses
				) == SLOT_SIZE * 11
			);
			assert!(size_of::<IServerGameDLL__bindgen_vtable>() == SLOT_SIZE * 48);
		};
	}

	_ => {
		compile_error!("Unsupported target");
	}
}
