#include <cstddef>
#include <cstdint>
#include <type_traits>

#if defined(METAMOD_BRIDGE_STABLE)
	namespace SourceHook {
		class ISourceHook;
	}

	// SourceHook's macros expand these names. Bind them to this shell's globals
	// rather than the `g_SHPtr` and `g_PLID` that `PLUGIN_EXPOSE` would define.
	#define SH_GLOB_SHPTR rust_shell_sourcehook
	#define SH_GLOB_PLUGPTR rust_shell_plugin_id

	static SourceHook::ISourceHook *rust_shell_sourcehook = nullptr;
	static int rust_shell_plugin_id = 0;
#endif

#include <ISmmPlugin.h>

#if defined(METAMOD_BRIDGE_STABLE) == defined(METAMOD_BRIDGE_DEV)
	#error "Build each Metamod shell with exactly one channel selected"
#elif defined(METAMOD_BRIDGE_STABLE)
	#if METAMOD_PLAPI_VERSION != 16
		#error "The stable Metamod shell requires the 1.12 build 1226 plugin headers"
	#endif

	static_assert(offsetof(MetamodVersionInfo, pl_min) == 16);
	static_assert(offsetof(MetamodVersionInfo, source_engine) == 24);

	#define RUST_SHELL_CREATE cpp_metamod_plugin_stable
	#define RUST_SHELL_INSTANCE cpp_metamod_plugin_stable_instance
	#define RUST_SHELL_IS_LOADED cpp_metamod_plugin_stable_is_loaded
	#define RUST_SHELL_HOOK_CLIENT_COMMANDS cpp_metamod_hook_client_commands_stable
	#define RUST_SHELL_HOOK_GAME_FRAME cpp_metamod_hook_game_frame_stable
	#define RUST_SHELL_LISTEN_LEVELS cpp_metamod_listen_levels_stable
	#define RUST_SHELL_HOOK_NET_MESSAGES cpp_metamod_hook_net_messages_stable
#else
	#if METAMOD_PLAPI_VERSION != 18
		#error "The dev Metamod shell requires the 2.0 build 1469 plugin headers"
	#endif

	static_assert(offsetof(MetamodVersionInfo, pl_min) == 8);
	static_assert(offsetof(MetamodVersionInfo, source_engine) == 16);

	#define RUST_SHELL_CREATE cpp_metamod_plugin_dev
	#define RUST_SHELL_INSTANCE cpp_metamod_plugin_dev_instance
	#define RUST_SHELL_IS_LOADED cpp_metamod_plugin_dev_is_loaded
	#define RUST_SHELL_HOOK_CLIENT_COMMANDS cpp_metamod_hook_client_commands_dev
	#define RUST_SHELL_HOOK_GAME_FRAME cpp_metamod_hook_game_frame_dev
	#define RUST_SHELL_LISTEN_LEVELS cpp_metamod_listen_levels_dev
	#define RUST_SHELL_HOOK_NET_MESSAGES cpp_metamod_hook_net_messages_dev
#endif

// Just for now.
// Hopefully I'll remember to add 32bit support.
static_assert(sizeof(void *) == 8);
static_assert(sizeof(SourceMM::ISmmAPI) == sizeof(void *));
static_assert(std::is_abstract_v<SourceMM::ISmmAPI>);
static_assert(!std::has_virtual_destructor_v<SourceMM::ISmmAPI>);
static_assert(std::has_virtual_destructor_v<SourceMM::ISmmPlugin>);

// Keep the handwritten Rust vtable declarations tied to the exact signatures
// in the configured Metamod headers. Slot order is asserted on the Rust side.
#define ASSERT_SMM_METHOD(name, return_type, ...) \
	static_assert(std::is_same_v<decltype(&SourceMM::ISmmAPI::name), return_type (SourceMM::ISmmAPI::*)(__VA_ARGS__)>)

ASSERT_SMM_METHOD(LogMsg, void, SourceMM::ISmmPlugin *, const char *, ...);
ASSERT_SMM_METHOD(GetEngineFactory, CreateInterfaceFn, bool);
ASSERT_SMM_METHOD(GetPhysicsFactory, CreateInterfaceFn, bool);
ASSERT_SMM_METHOD(GetFileSystemFactory, CreateInterfaceFn, bool);
ASSERT_SMM_METHOD(GetServerFactory, CreateInterfaceFn, bool);
ASSERT_SMM_METHOD(GetCGlobals, CGlobalVars *);
ASSERT_SMM_METHOD(RegisterConCommandBase, bool, SourceMM::ISmmPlugin *, ConCommandBase *);
ASSERT_SMM_METHOD(UnregisterConCommandBase, void, SourceMM::ISmmPlugin *, ConCommandBase *);
ASSERT_SMM_METHOD(ConPrint, void, const char *);
ASSERT_SMM_METHOD(ConPrintf, void, const char *, ...);
ASSERT_SMM_METHOD(GetApiVersions, void, int &, int &, int &, int &);

#if defined(METAMOD_BRIDGE_STABLE)
	ASSERT_SMM_METHOD(GetShVersions, void, int &, int &);
#endif

ASSERT_SMM_METHOD(AddListener, void, SourceMM::ISmmPlugin *, SourceMM::IMetamodListener *);
ASSERT_SMM_METHOD(MetaFactory, void *, const char *, int *, SourceMM::PluginId *);
ASSERT_SMM_METHOD(FormatIface, int, char *, std::size_t);
ASSERT_SMM_METHOD(InterfaceSearch, void *, CreateInterfaceFn, const char *, int, int *);
ASSERT_SMM_METHOD(GetBaseDir, const char *);
ASSERT_SMM_METHOD(PathFormat, std::size_t, char *, std::size_t, const char *, ...);

#if defined(METAMOD_BRIDGE_STABLE)
	ASSERT_SMM_METHOD(ClientConPrintf, void, edict_t *, const char *, ...);
#else
	ASSERT_SMM_METHOD(ClientConPrintf, void, MMSPlayer_t, const char *, ...);
#endif

ASSERT_SMM_METHOD(VInterfaceMatch, void *, CreateInterfaceFn, const char *, int);
ASSERT_SMM_METHOD(EnableVSPListener, void);
ASSERT_SMM_METHOD(GetGameDLLVersion, int);
ASSERT_SMM_METHOD(GetUserMessageCount, int);
ASSERT_SMM_METHOD(FindUserMessage, int, const char *, int *);
ASSERT_SMM_METHOD(GetUserMessage, const char *, int, int *);
ASSERT_SMM_METHOD(GetVSPVersion, int);
ASSERT_SMM_METHOD(GetSourceEngineBuild, int);
ASSERT_SMM_METHOD(GetVSPInfo, IServerPluginCallbacks *, int *);
ASSERT_SMM_METHOD(Format, std::size_t, char *, std::size_t, const char *, ...);
ASSERT_SMM_METHOD(FormatArgs, std::size_t, char *, std::size_t, const char *, va_list);

#if defined(METAMOD_BRIDGE_DEV)
	ASSERT_SMM_METHOD(RegisterConCommand, bool, SourceMM::ISmmPlugin *, ProviderConCommand *);
	ASSERT_SMM_METHOD(RegisterConVar, bool, SourceMM::ISmmPlugin *, ProviderConVar *);
	ASSERT_SMM_METHOD(UnregisterConCommand, void, SourceMM::ISmmPlugin *, ProviderConCommand *);
	ASSERT_SMM_METHOD(UnregisterConVar, void, SourceMM::ISmmPlugin *, ProviderConVar *);
	ASSERT_SMM_METHOD(GetDetourInterface, void *, SourceMM::PluginId);
#endif

#undef ASSERT_SMM_METHOD

extern "C" {
	struct RustPluginCallbacks {
		bool (*load)(void *, void *, char *, std::size_t, bool);
		void (*all_plugins_loaded)(void *);
		bool (*query_running)(void *, char *, std::size_t);
		bool (*unload)(void *, char *, std::size_t);
		bool (*pause)(void *, char *, std::size_t);
		bool (*unpause)(void *, char *, std::size_t);
	};

	struct RustPluginMetadata {
		const char *author;
		const char *name;
		const char *description;
		const char *url;
		const char *license;
		const char *version;
		const char *date;
		const char *log_tag;
	};
}

static_assert(sizeof(RustPluginCallbacks) == 6 * sizeof(void *));
static_assert(sizeof(RustPluginMetadata) == 8 * sizeof(void *));

extern "C" {
	// Runs a client's string command. Returns true if Rust handled it, which
	// keeps it from reaching the game.
	typedef bool (*RustClientCommandFn)(void *context, edict_t *edict, const void *command);

	// Runs once per server frame, before the game's own frame.
	typedef void (*RustGameFrameFn)(void *context, bool simulating);

	// Metamod's level notifications. `map` is the name of the level loading.
	typedef void (*RustLevelInitFn)(void *context, const char *map);
	typedef void (*RustLevelShutdownFn)(void *context);

	// A net message from a client, before its handler processes it. `kind` is
	// the message's index in the slots the hooks were installed with, and
	// `handler` the engine's `this`. Returns true to block the message, which
	// the engine then treats as processed.
	typedef bool (*RustNetMessageFn)(void *context, int kind, void *handler, void *message);

	enum RustHookStatus : int {
		RUST_HOOK_INSTALLED = 0,
		RUST_HOOK_NOT_BOUND = 1,
		RUST_HOOK_ALREADY_INSTALLED = 2,
		RUST_HOOK_INVALID_ARGUMENT = 3,
		RUST_HOOK_REFUSED = 4,
	};
}

namespace {
	// A layout mirror of `CCommand` from `public/tier1/convar.h`, which hooks
	// only pass by reference. Its size is what SourceHook and KHook record for
	// the parameter, matching the `ClientCommand` hooks of Metamod and SourceMod.
	struct RustShellCCommand {
		int argc;
		int argv0_size;
		char arg_string[512];
		char argv_buffer[512];
		const char *argv[64];
	};

	static_assert(sizeof(RustShellCCommand) == 1544);

	// `IServerGameClients` declares no virtual destructor, so `ClientCommand`
	// has this slot under the MSVC and Itanium ABIs alike.
	constexpr int kClientCommandSlot = 5;

	// Where the hook sends client commands. It is cleared when the plugin
	// unloads, since Metamod removes the hook itself, possibly later.
	struct ClientCommandRoute {
		RustClientCommandFn callback = nullptr;
		void *context = nullptr;
		void *clients = nullptr;
		bool paused = false;
	};

	ClientCommandRoute client_command_route;

	// Asks Rust to run a command, unless the plugin is paused or unloaded.
	bool route_client_command(void *self, edict_t *edict, const RustShellCCommand &command) {
		const ClientCommandRoute &route = client_command_route;

		if (route.callback == nullptr || route.paused || self != route.clients)
			return false;

		return route.callback(route.context, edict, &command);
	}

	// `IServerGameDLL` declares no virtual destructor, so `GameFrame` has this
	// slot under the MSVC and Itanium ABIs alike.
	constexpr int kGameFrameSlot = 5;

	struct GameFrameRoute {
		RustGameFrameFn callback = nullptr;
		void *context = nullptr;
		void *game_dll = nullptr;
		bool paused = false;
	};

	GameFrameRoute game_frame_route;

	void route_game_frame(void *self, bool simulating) {
		const GameFrameRoute &route = game_frame_route;

		if (route.callback != nullptr && !route.paused && self == route.game_dll)
			route.callback(route.context, simulating);
	}

	struct LevelRoute {
		RustLevelInitFn init = nullptr;
		RustLevelShutdownFn shutdown = nullptr;
		void *context = nullptr;
		bool paused = false;
	};

	LevelRoute level_route;

	// Registered once per load, and removed by Metamod when it unloads the
	// plugin. It outlives every load, as a static of the library.
	class RustLevelListener final : public SourceMM::IMetamodListener {
	public:
		void OnLevelInit(char const *map, char const *, char const *, char const *, bool, bool) override {
			const LevelRoute &route = level_route;

			if (route.init != nullptr && !route.paused)
				route.init(route.context, map);
		}

		void OnLevelShutdown() override {
			const LevelRoute &route = level_route;

			if (route.shutdown != nullptr && !route.paused)
				route.shutdown(route.context);
		}
	};

	RustLevelListener level_listener;
	bool level_listener_added = false;

	// Every handler slot a client's net messages can arrive through: the
	// `Process*` methods of `IClientMessageHandler` with `REPLAY_ENABLED`.
	constexpr int kMaxNetMessageKinds = 14;

	struct NetMessageRoute {
		RustNetMessageFn callback = nullptr;
		void *context = nullptr;
		// The vtable every hooked handler shares.
		void **vtable = nullptr;
		int kinds = 0;
		bool paused = false;
	};

	NetMessageRoute net_message_route;

	// Asks Rust whether to block a message, unless the plugin is paused or
	// unloaded, or the handler is some other class with its own vtable.
	bool route_net_message(int kind, void *self, void *message) {
		const NetMessageRoute &route = net_message_route;

		if (route.callback == nullptr || route.paused || kind < 0 || kind >= route.kinds)
			return false;

		if (self == nullptr || *static_cast<void ***>(self) != route.vtable)
			return false;

		return route.callback(route.context, kind, self, message);
	}

	void set_routes_paused(bool paused) {
		client_command_route.paused = paused;
		game_frame_route.paused = paused;
		level_route.paused = paused;
		net_message_route.paused = paused;
	}

	void clear_routes() {
		client_command_route = {};
		game_frame_route = {};
		level_route = {};
		net_message_route = {};
	}
}

#if defined(METAMOD_BRIDGE_STABLE)
namespace {
	// The same prototype as Metamod's and SourceMod's own hooks on this method,
	// so SourceHook shares one hook manager among them.
	SH_DECL_MANUALHOOK2_void(RustShell_ClientCommand, kClientCommandSlot, 0, 0, edict_t *, const RustShellCCommand &);

	// SourceHook itself outlives every plugin, so `rust_shell_sourcehook` stays
	// set for hooks that run after an unload; this tracks the binding instead.
	bool hooks_bound = false;
	int client_command_hook = 0;

	void on_client_command(edict_t *edict, const RustShellCCommand &command) {
		// An earlier hook, such as a SourceMod command listener, blocked it.
		if (META_RESULT_STATUS >= MRES_SUPERCEDE)
			RETURN_META(MRES_IGNORED);

		if (route_client_command(META_IFACEPTR(void), edict, command))
			RETURN_META(MRES_SUPERCEDE);

		RETURN_META(MRES_IGNORED);
	}

	void bind_hooking(SourceMM::PluginId id, SourceMM::ISmmAPI *api) {
		rust_shell_plugin_id = id;
		rust_shell_sourcehook = static_cast<SourceHook::ISourceHook *>(api->MetaFactory(MMIFACE_SOURCEHOOK, nullptr, nullptr));
		hooks_bound = rust_shell_sourcehook != nullptr;
	}

	// SourceHook removes the plugin's hooks when Metamod unloads it.
	bool add_client_command_hook(void *clients) {
		client_command_hook = SH_ADD_MANUALHOOK(RustShell_ClientCommand, clients, SH_STATIC(on_client_command), false);

		return client_command_hook != 0;
	}

	SH_DECL_MANUALHOOK1_void(RustShell_GameFrame, kGameFrameSlot, 0, 0, bool);

	int game_frame_hook = 0;

	void on_game_frame(bool simulating) {
		route_game_frame(META_IFACEPTR(void), simulating);
		RETURN_META(MRES_IGNORED);
	}

	bool add_game_frame_hook(void *game_dll) {
		game_frame_hook = SH_ADD_MANUALHOOK(RustShell_GameFrame, game_dll, SH_STATIC(on_game_frame), false);

		return game_frame_hook != 0;
	}

	bool game_frame_hooked() {
		return game_frame_hook != 0;
	}

	// One manual hook per message kind, each reconfigured to its slot when it
	// is added. A blocked message counts as processed, as if its handler had
	// returned true.
	#define RUST_SHELL_NET_MESSAGE_HOOK(kind)                                                        \
		SH_DECL_MANUALHOOK1(RustShell_NetMessage##kind, 0, 0, 0, bool, void *);                     \
                                                                                                    \
		bool on_net_message_##kind(void *message) {                                                 \
			if (META_RESULT_STATUS >= MRES_SUPERCEDE)                                                \
				RETURN_META_VALUE(MRES_IGNORED, true);                                               \
                                                                                                    \
			if (route_net_message(kind, META_IFACEPTR(void), message))                               \
				RETURN_META_VALUE(MRES_SUPERCEDE, true);                                             \
                                                                                                    \
			RETURN_META_VALUE(MRES_IGNORED, true);                                                   \
		}                                                                                            \
                                                                                                    \
		int add_net_message_hook_##kind(void *handler, int slot) {                                   \
			SH_MANUALHOOK_RECONFIGURE(RustShell_NetMessage##kind, slot, 0, 0);                       \
			return SH_ADD_MANUALVPHOOK(                                                              \
				RustShell_NetMessage##kind, handler, SH_STATIC(on_net_message_##kind), false);       \
		}

	RUST_SHELL_NET_MESSAGE_HOOK(0)
	RUST_SHELL_NET_MESSAGE_HOOK(1)
	RUST_SHELL_NET_MESSAGE_HOOK(2)
	RUST_SHELL_NET_MESSAGE_HOOK(3)
	RUST_SHELL_NET_MESSAGE_HOOK(4)
	RUST_SHELL_NET_MESSAGE_HOOK(5)
	RUST_SHELL_NET_MESSAGE_HOOK(6)
	RUST_SHELL_NET_MESSAGE_HOOK(7)
	RUST_SHELL_NET_MESSAGE_HOOK(8)
	RUST_SHELL_NET_MESSAGE_HOOK(9)
	RUST_SHELL_NET_MESSAGE_HOOK(10)
	RUST_SHELL_NET_MESSAGE_HOOK(11)
	RUST_SHELL_NET_MESSAGE_HOOK(12)
	RUST_SHELL_NET_MESSAGE_HOOK(13)

	#undef RUST_SHELL_NET_MESSAGE_HOOK

	using AddNetMessageHookFn = int (*)(void *handler, int slot);

	constexpr AddNetMessageHookFn kAddNetMessageHooks[kMaxNetMessageKinds] = {
		add_net_message_hook_0, add_net_message_hook_1, add_net_message_hook_2, add_net_message_hook_3,
		add_net_message_hook_4, add_net_message_hook_5, add_net_message_hook_6, add_net_message_hook_7,
		add_net_message_hook_8, add_net_message_hook_9, add_net_message_hook_10, add_net_message_hook_11,
		add_net_message_hook_12, add_net_message_hook_13,
	};

	int net_message_hooks[kMaxNetMessageKinds] = {};
	bool net_message_hooks_added = false;

	// Hooks the vtable `handler` shares with every other handler of its class.
	// On a refusal, removes the hooks already added.
	bool add_net_message_hooks(void *handler, const int *slots, int kinds) {
		for (int kind = 0; kind < kinds; ++kind) {
			net_message_hooks[kind] = kAddNetMessageHooks[kind](handler, slots[kind]);

			if (net_message_hooks[kind] == 0) {
				for (int added = 0; added < kind; ++added) {
					SH_REMOVE_HOOK_ID(net_message_hooks[added]);
					net_message_hooks[added] = 0;
				}

				return false;
			}
		}

		net_message_hooks_added = true;
		return true;
	}

	bool net_messages_hooked() {
		return net_message_hooks_added;
	}

	void unbind_hooking() {
		hooks_bound = false;
		client_command_hook = 0;
		game_frame_hook = 0;
		net_message_hooks_added = false;

		for (int &hook : net_message_hooks)
			hook = 0;
	}

	bool hooking_bound() {
		return hooks_bound;
	}

	bool client_command_hooked() {
		return client_command_hook != 0;
	}
}
#else
namespace {
	// Metamod gives each plugin its own `IKHook`, which it frees right after
	// the plugin's `Unload`, and removes the plugin's hooks later, while they
	// can still run. The hook therefore never calls through that object after
	// binding. Metamod's implementation forwards each call to KHook's globals
	// and ignores `this`, so the functions are cached from its vtable instead.
	// `IKHook` declares no virtual destructor and no overloads, so its methods
	// occupy slots in declaration order under both ABIs.
	constexpr std::size_t kGetContextPtrSlot = 3;
	constexpr std::size_t kGetOriginalFunctionSlot = 4;
	constexpr std::size_t kGetCurrentValuePtrSlot = 7;
	constexpr std::size_t kDestroyReturnValueSlot = 8;
	constexpr std::size_t kSaveReturnValueSlot = 12;

	using GetContextPtrFn = void *(*)(void *self);
	using GetOriginalFunctionFn = void *(*)(void *self);
	using GetCurrentValuePtrFn = void *(*)(void *self, bool pop);
	using DestroyReturnValueFn = void (*)(void *self);
	using SaveReturnValueFn = void (*)(void *self, KHook::Action action, void *value, std::size_t size, void *init, void *deinit, bool original);
	using ClientCommandFn = void (*)(void *self, edict_t *edict, const RustShellCCommand &command);
	using GameFrameFn = void (*)(void *self, bool simulating);
	using NetMessageFn = bool (*)(void *self, void *message);

	struct CachedKHook {
		void *self = nullptr;
		GetContextPtrFn get_context_ptr = nullptr;
		GetOriginalFunctionFn get_original_function = nullptr;
		GetCurrentValuePtrFn get_current_value_ptr = nullptr;
		DestroyReturnValueFn destroy_return_value = nullptr;
		SaveReturnValueFn save_return_value = nullptr;
	};

	// The interface is live only between `Load` and `Unload`; the cache stays
	// usable for as long as Metamod's code is loaded.
	KHook::IKHook *khook = nullptr;
	CachedKHook cached_khook;

	// Identifies this load's hook: a hook of an earlier load of the same
	// library can still run until Metamod has removed it.
	std::uintptr_t hook_generation = 0;
	KHook::HookID_t client_command_hook = KHook::INVALID_HOOK;

	// KHook calls these like member functions of the hooked object, which on
	// x86-64 is a call with `this` as the first argument under both ABIs.
	void client_command_pre(void *self, edict_t *edict, const RustShellCCommand &command) {
		KHook::Action action = KHook::Action::Ignore;
		void *generation = reinterpret_cast<void *>(hook_generation);

		if (cached_khook.get_context_ptr(cached_khook.self) == generation && route_client_command(self, edict, command))
			action = KHook::Action::Supersede;

		cached_khook.save_return_value(cached_khook.self, action, nullptr, 0, nullptr, nullptr, false);
	}

	void client_command_make_return(void *, edict_t *, const RustShellCCommand &) {
		cached_khook.destroy_return_value(cached_khook.self);
	}

	void client_command_call_original(void *self, edict_t *edict, const RustShellCCommand &command) {
		auto original = reinterpret_cast<ClientCommandFn>(cached_khook.get_original_function(cached_khook.self));

		original(self, edict, command);
		cached_khook.save_return_value(cached_khook.self, KHook::Action::Ignore, nullptr, 0, nullptr, nullptr, true);
	}

	void bind_hooking(SourceMM::PluginId id, SourceMM::ISmmAPI *api) {
		khook = static_cast<KHook::IKHook *>(api->GetDetourInterface(id));

		if (khook == nullptr)
			return;

		void **vtable = *reinterpret_cast<void ***>(khook);

		cached_khook = {
			khook,
			reinterpret_cast<GetContextPtrFn>(vtable[kGetContextPtrSlot]),
			reinterpret_cast<GetOriginalFunctionFn>(vtable[kGetOriginalFunctionSlot]),
			reinterpret_cast<GetCurrentValuePtrFn>(vtable[kGetCurrentValuePtrSlot]),
			reinterpret_cast<DestroyReturnValueFn>(vtable[kDestroyReturnValueSlot]),
			reinterpret_cast<SaveReturnValueFn>(vtable[kSaveReturnValueSlot]),
		};

		++hook_generation;
	}

	// The stack KHook copies for each callback, as `KHook::Virtual` computes it.
	constexpr unsigned int client_command_stack_size() {
	#if defined(_WIN64)
		return 32;
	#else
		return sizeof(void *) + sizeof(edict_t *) + sizeof(RustShellCCommand);
	#endif
	}

	// The hook is never removed here: Metamod removes it after `Unload`, and
	// unloads the library once KHook reports the removal. Removing it first
	// would keep Metamod waiting for a report that never comes.
	bool add_client_command_hook(void *clients) {
		client_command_hook = khook->SetupVirtualHook(
			*reinterpret_cast<void ***>(clients),
			kClientCommandSlot,
			reinterpret_cast<void *>(hook_generation),
			nullptr,
			reinterpret_cast<void *>(&client_command_pre),
			nullptr,
			reinterpret_cast<void *>(&client_command_make_return),
			reinterpret_cast<void *>(&client_command_call_original),
			client_command_stack_size(),
			// As `KHook::Virtual` does, since a detour may be running.
			true
		);

		return client_command_hook != KHook::INVALID_HOOK;
	}

	KHook::HookID_t game_frame_hook = KHook::INVALID_HOOK;

	void game_frame_pre(void *self, bool simulating) {
		void *generation = reinterpret_cast<void *>(hook_generation);

		if (cached_khook.get_context_ptr(cached_khook.self) == generation)
			route_game_frame(self, simulating);

		cached_khook.save_return_value(cached_khook.self, KHook::Action::Ignore, nullptr, 0, nullptr, nullptr, false);
	}

	void game_frame_make_return(void *, bool) {
		cached_khook.destroy_return_value(cached_khook.self);
	}

	void game_frame_call_original(void *self, bool simulating) {
		auto original = reinterpret_cast<GameFrameFn>(cached_khook.get_original_function(cached_khook.self));

		original(self, simulating);
		cached_khook.save_return_value(cached_khook.self, KHook::Action::Ignore, nullptr, 0, nullptr, nullptr, true);
	}

	// As `KHook::Virtual` computes it for `void (*)(bool)`: MSVC's shadow
	// space, or `this` and the flag, each in a full slot.
	constexpr unsigned int game_frame_stack_size() {
	#if defined(_WIN64)
		return 32;
	#else
		return sizeof(void *) + sizeof(void *);
	#endif
	}

	bool add_game_frame_hook(void *game_dll) {
		game_frame_hook = khook->SetupVirtualHook(
			*reinterpret_cast<void ***>(game_dll),
			kGameFrameSlot,
			reinterpret_cast<void *>(hook_generation),
			nullptr,
			reinterpret_cast<void *>(&game_frame_pre),
			nullptr,
			reinterpret_cast<void *>(&game_frame_make_return),
			reinterpret_cast<void *>(&game_frame_call_original),
			game_frame_stack_size(),
			true
		);

		return game_frame_hook != KHook::INVALID_HOOK;
	}

	bool game_frame_hooked() {
		return game_frame_hook != KHook::INVALID_HOOK;
	}

	// Each message hook's context packs this load's generation with the kind,
	// so a hook left by an earlier load of the library routes nothing.
	constexpr unsigned int kNetMessageKindBits = 5;
	constexpr std::uintptr_t kNetMessageKindMask = (std::uintptr_t(1) << kNetMessageKindBits) - 1;

	static_assert(kMaxNetMessageKinds <= int(kNetMessageKindMask) + 1);

	KHook::HookID_t net_message_hooks[kMaxNetMessageKinds] = {};
	bool net_message_hooks_added = false;

	void *net_message_context(int kind) {
		return reinterpret_cast<void *>((hook_generation << kNetMessageKindBits) | std::uintptr_t(kind));
	}

	void save_bool_return(KHook::Action action, bool value, bool original) {
		cached_khook.save_return_value(
			cached_khook.self,
			action,
			&value,
			sizeof(bool),
			reinterpret_cast<void *>(&KHook::init_operator<bool>),
			reinterpret_cast<void *>(&KHook::deinit_operator<bool>),
			original
		);
	}

	// A blocked message counts as processed, as if its handler returned true.
	bool net_message_pre(void *self, void *message) {
		auto context = reinterpret_cast<std::uintptr_t>(cached_khook.get_context_ptr(cached_khook.self));
		KHook::Action action = KHook::Action::Ignore;

		if ((context >> kNetMessageKindBits) == hook_generation
			&& route_net_message(int(context & kNetMessageKindMask), self, message))
			action = KHook::Action::Supersede;

		save_bool_return(action, true, false);
		return true;
	}

	bool net_message_make_return(void *, void *) {
		bool value = *static_cast<bool *>(cached_khook.get_current_value_ptr(cached_khook.self, true));

		cached_khook.destroy_return_value(cached_khook.self);
		return value;
	}

	bool net_message_call_original(void *self, void *message) {
		auto original = reinterpret_cast<NetMessageFn>(cached_khook.get_original_function(cached_khook.self));
		bool value = original(self, message);

		save_bool_return(KHook::Action::Ignore, value, true);
		return value;
	}

	// As `KHook::Virtual` computes it for `bool (*)(void *)`: MSVC's shadow
	// space, or the widened return value, `this`, and the message.
	constexpr unsigned int net_message_stack_size() {
	#if defined(_WIN64)
		return 32;
	#else
		return sizeof(void *) * 3;
	#endif
	}

	// Hooks the vtable `handler` shares with every other handler of its class.
	// Hooks are never removed here: if any is refused, the caller clears the
	// route instead, which leaves the others inert.
	bool add_net_message_hooks(void *handler, const int *slots, int kinds) {
		void **vtable = *static_cast<void ***>(handler);

		for (int kind = 0; kind < kinds; ++kind) {
			net_message_hooks[kind] = khook->SetupVirtualHook(
				vtable,
				slots[kind],
				net_message_context(kind),
				nullptr,
				reinterpret_cast<void *>(&net_message_pre),
				nullptr,
				reinterpret_cast<void *>(&net_message_make_return),
				reinterpret_cast<void *>(&net_message_call_original),
				net_message_stack_size(),
				true
			);

			if (net_message_hooks[kind] == KHook::INVALID_HOOK)
				return false;
		}

		net_message_hooks_added = true;
		return true;
	}

	bool net_messages_hooked() {
		return net_message_hooks_added;
	}

	void unbind_hooking() {
		khook = nullptr;
		client_command_hook = KHook::INVALID_HOOK;
		game_frame_hook = KHook::INVALID_HOOK;
		net_message_hooks_added = false;

		for (KHook::HookID_t &hook : net_message_hooks)
			hook = KHook::INVALID_HOOK;
	}

	bool hooking_bound() {
		return khook != nullptr;
	}

	bool client_command_hooked() {
		return client_command_hook != KHook::INVALID_HOOK;
	}
}
#endif

namespace {
	class RustMetamodPlugin final : public SourceMM::ISmmPlugin {
	public:
		void configure(const RustPluginCallbacks *callbacks, RustPluginMetadata metadata) {
			callbacks_ = callbacks;
			metadata_ = metadata;
		}

		// Whether Metamod has loaded this plugin object and not yet unloaded it.
		// Metamod tracks what a plugin registers by the object it loaded, from
		// before `Load` until after `Unload` or a refused `Load`.
		bool loaded() const { return loaded_; }

		// Metamod's API for this load, while loaded.
		SourceMM::ISmmAPI *api() const { return api_; }

		int GetApiVersion() override { return METAMOD_PLAPI_VERSION; }

		bool Load(SourceMM::PluginId id, SourceMM::ISmmAPI *api, char *error, std::size_t max_length, bool late) override {
			// Drop what an earlier load of a library that stayed mapped left, such
			// as after a forced unload that followed a refused one.
			unload();
			loaded_ = true;
			api_ = api;

			// Rust may install hooks while loading.
			bind_hooking(id, api);

			bool loaded = callbacks_->load(this, api, error, max_length, late);

			// Metamod calls no `Unload` after a refused `Load`.
			if (!loaded)
				unload();

			return loaded;
		}

		void AllPluginsLoaded() override { callbacks_->all_plugins_loaded(this); }
		bool QueryRunning(char *error, std::size_t max_length) override { return callbacks_->query_running(this, error, max_length); }

		bool Unload(char *error, std::size_t max_length) override {
			// Rust saw its `Load` refused and has nothing to tear down. Metamod 2.0
			// keeps a refused plugin listed, and removes it only through here.
			if (!loaded_)
				return true;

			bool unloaded = callbacks_->unload(this, error, max_length);

			// Metamod keeps a plugin that refuses, unless it unloads it by force,
			// which it does without calling back. Commands are unlinked before
			// the hook is removed then, so the route no longer finds any.
			if (unloaded)
				unload();

			return unloaded;
		}

		bool Pause(char *error, std::size_t max_length) override {
			bool paused = callbacks_->pause(this, error, max_length);

			if (paused)
				set_routes_paused(true);

			return paused;
		}

		bool Unpause(char *error, std::size_t max_length) override {
			bool unpaused = callbacks_->unpause(this, error, max_length);

			if (unpaused)
				set_routes_paused(false);

			return unpaused;
		}

		const char *GetAuthor() override { return metadata_.author; }
		const char *GetName() override { return metadata_.name; }
		const char *GetDescription() override { return metadata_.description; }
		const char *GetURL() override { return metadata_.url; }
		const char *GetLicense() override { return metadata_.license; }
		const char *GetVersion() override { return metadata_.version; }
		const char *GetDate() override { return metadata_.date; }
		const char *GetLogTag() override { return metadata_.log_tag; }

	private:
		void unload() {
			loaded_ = false;
			api_ = nullptr;
			// Metamod removes the level listener itself after `Unload`.
			level_listener_added = false;
			clear_routes();
			unbind_hooking();
		}

		const RustPluginCallbacks *callbacks_ = nullptr;
		SourceMM::ISmmAPI *api_ = nullptr;
		RustPluginMetadata metadata_ = {};
		bool loaded_ = false;
	};

	RustMetamodPlugin plugin;
}

extern "C" void *RUST_SHELL_INSTANCE() {
	return &plugin;
}

extern "C" bool RUST_SHELL_IS_LOADED() {
	return plugin.loaded();
}

extern "C" void *RUST_SHELL_CREATE(const RustPluginCallbacks *callbacks, RustPluginMetadata metadata) {
	if (!callbacks)
		return nullptr;

	plugin.configure(callbacks, metadata);

	return &plugin;
}

// Routes clients' string commands to `callback` until the plugin unloads.
//
// Call it between the shell's `Load` and `Unload`, on the server's main thread,
// with the game's `IServerGameClients`. `callback(context, ...)` must stay
// callable for as long as the library is loaded.
extern "C" int RUST_SHELL_HOOK_CLIENT_COMMANDS(void *clients, RustClientCommandFn callback, void *context) noexcept {
	if (!hooking_bound())
		return RUST_HOOK_NOT_BOUND;

	if (client_command_hooked())
		return RUST_HOOK_ALREADY_INSTALLED;

	if (clients == nullptr || callback == nullptr)
		return RUST_HOOK_INVALID_ARGUMENT;

	// KHook may call the hook as soon as it is added.
	client_command_route = {callback, context, clients, false};

	if (!add_client_command_hook(clients)) {
		client_command_route = {};
		return RUST_HOOK_REFUSED;
	}

	return RUST_HOOK_INSTALLED;
}

// Runs `callback` once per server frame, before the game's own frame, until
// the plugin unloads. The same conditions as for client commands apply, with
// the game's `IServerGameDLL`.
extern "C" int RUST_SHELL_HOOK_GAME_FRAME(void *game_dll, RustGameFrameFn callback, void *context) noexcept {
	if (!hooking_bound())
		return RUST_HOOK_NOT_BOUND;

	if (game_frame_hooked())
		return RUST_HOOK_ALREADY_INSTALLED;

	if (game_dll == nullptr || callback == nullptr)
		return RUST_HOOK_INVALID_ARGUMENT;

	// KHook may call the hook as soon as it is added.
	game_frame_route = {callback, context, game_dll, false};

	if (!add_game_frame_hook(game_dll)) {
		game_frame_route = {};
		return RUST_HOOK_REFUSED;
	}

	return RUST_HOOK_INSTALLED;
}

// Passes Metamod's level notifications to the callbacks until the plugin
// unloads. Call it between the shell's `Load` and `Unload`, on the main thread.
extern "C" int RUST_SHELL_LISTEN_LEVELS(RustLevelInitFn init, RustLevelShutdownFn shutdown, void *context) noexcept {
	SourceMM::ISmmAPI *api = plugin.api();

	if (api == nullptr)
		return RUST_HOOK_NOT_BOUND;

	if (level_listener_added)
		return RUST_HOOK_ALREADY_INSTALLED;

	if (init == nullptr && shutdown == nullptr)
		return RUST_HOOK_INVALID_ARGUMENT;

	level_route = {init, shutdown, context, false};
	api->AddListener(&plugin, &level_listener);
	level_listener_added = true;

	return RUST_HOOK_INSTALLED;
}

// Passes each net message the engine hands a client's message handler to
// `callback`, before the handler runs, until the plugin unloads.
//
// `handler` is one such handler: every handler sharing its vtable is hooked.
// `slots[kind]` is the vtable slot of each kind's `Process*` method. The same
// conditions as for client commands apply.
extern "C" int RUST_SHELL_HOOK_NET_MESSAGES(void *handler, const int *slots, int kinds, RustNetMessageFn callback, void *context) noexcept {
	if (!hooking_bound())
		return RUST_HOOK_NOT_BOUND;

	if (net_messages_hooked())
		return RUST_HOOK_ALREADY_INSTALLED;

	if (handler == nullptr || slots == nullptr || callback == nullptr || kinds <= 0 || kinds > kMaxNetMessageKinds)
		return RUST_HOOK_INVALID_ARGUMENT;

	for (int kind = 0; kind < kinds; ++kind) {
		if (slots[kind] < 0)
			return RUST_HOOK_INVALID_ARGUMENT;
	}

	// KHook may call the hooks as soon as they are added.
	net_message_route = {callback, context, *static_cast<void ***>(handler), kinds, false};

	if (!add_net_message_hooks(handler, slots, kinds)) {
		net_message_route = {};
		return RUST_HOOK_REFUSED;
	}

	return RUST_HOOK_INSTALLED;
}
