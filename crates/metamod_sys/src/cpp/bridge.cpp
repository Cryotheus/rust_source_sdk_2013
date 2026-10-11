#include <cstddef>
#include <cstdint>
#include <string_view>
#include <type_traits>

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
	#define RUST_SHELL_STATUS cpp_metamod_plugin_stable_status
	#define RUST_SHELL_LISTEN_LEVELS cpp_metamod_listen_levels_stable
	#define RUST_SHELL_LEVEL_GENERATION cpp_metamod_level_generation_stable
#else
	#if METAMOD_PLAPI_VERSION != 18
		#error "The dev Metamod shell requires the plugin API 18 headers of 2.0, as in builds 1469 through 1472"
	#endif

	static_assert(offsetof(MetamodVersionInfo, pl_min) == 8);
	static_assert(offsetof(MetamodVersionInfo, source_engine) == 16);

	#define RUST_SHELL_CREATE cpp_metamod_plugin_dev
	#define RUST_SHELL_INSTANCE cpp_metamod_plugin_dev_instance
	#define RUST_SHELL_IS_LOADED cpp_metamod_plugin_dev_is_loaded
	#define RUST_SHELL_STATUS cpp_metamod_plugin_dev_status
	#define RUST_SHELL_LISTEN_LEVELS cpp_metamod_listen_levels_dev
	#define RUST_SHELL_LEVEL_GENERATION cpp_metamod_level_generation_dev
#endif

// Just for now.
// Hopefully I'll remember to add 32bit support.
static_assert(sizeof(void *) == 8);
static_assert(sizeof(SourceMM::ISmmAPI) == sizeof(void *));
static_assert(std::is_abstract_v<SourceMM::ISmmAPI>);
static_assert(!std::has_virtual_destructor_v<SourceMM::ISmmAPI>);
static_assert(std::has_virtual_destructor_v<SourceMM::ISmmPlugin>);
static_assert(std::is_same_v<SourceMM::PluginId, int>);

// Keep the handwritten Rust vtable declarations tied to the exact signatures
// in the configured Metamod headers. Slot order is asserted on the Rust side.
#define ASSERT_METHOD(type, name, return_type, ...) \
	static_assert(std::is_same_v<decltype(&type::name), return_type (type::*)(__VA_ARGS__)>)

#define ASSERT_SMM_METHOD(name, return_type, ...) ASSERT_METHOD(SourceMM::ISmmAPI, name, return_type, __VA_ARGS__)

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

// The hooking libraries' interfaces, which Rust calls and implements through
// the handwritten declarations of `metamod_source_sys::sourcehook` and
// `metamod_source_sys::khook`.
#if defined(METAMOD_BRIDGE_STABLE)
	static_assert(std::string_view(MMIFACE_SOURCEHOOK) == "ISourceHook");
	static_assert(SH_IFACE_VERSION == 5);
	static_assert(SH_IMPL_VERSION == 5);
	static_assert(SH_HOOKMAN_VERSION == 1);

	static_assert(sizeof(META_RES) == sizeof(int));
	static_assert(MRES_IGNORED == 0 && MRES_HANDLED == 1 && MRES_OVERRIDE == 2 && MRES_SUPERCEDE == 3);
	static_assert(sizeof(SourceHook::ISourceHook::AddHookMode) == sizeof(int));
	static_assert(SourceHook::ISourceHook::Hook_Normal == 0);
	static_assert(SourceHook::ISourceHook::Hook_VP == 1);
	static_assert(SourceHook::ISourceHook::Hook_DVP == 2);
	static_assert(std::is_same_v<SourceHook::Plugin, int>);
	static_assert(std::is_same_v<SourceHook::HookManagerPubFunc, int (*)(bool, SourceHook::IHookManagerInfo *)>);

	static_assert(sizeof(SourceHook::PassInfo) == 16);
	static_assert(offsetof(SourceHook::PassInfo, size) == 0);
	static_assert(offsetof(SourceHook::PassInfo, type) == 8);
	static_assert(offsetof(SourceHook::PassInfo, flags) == 12);
	static_assert(sizeof(SourceHook::PassInfo::V2Info) == 32);
	static_assert(sizeof(SourceHook::ProtoInfo) == 80);
	static_assert(offsetof(SourceHook::ProtoInfo, numOfParams) == 0);
	static_assert(offsetof(SourceHook::ProtoInfo, retPassInfo) == 8);
	static_assert(offsetof(SourceHook::ProtoInfo, paramsPassInfo) == 24);
	static_assert(offsetof(SourceHook::ProtoInfo, convention) == 32);
	static_assert(offsetof(SourceHook::ProtoInfo, retPassInfo2) == 40);
	static_assert(offsetof(SourceHook::ProtoInfo, paramsPassInfo2) == 72);
	static_assert(SourceHook::PassInfo::PassFlag_ByVal == 1);
	static_assert(SourceHook::ProtoInfo::CallConv_ThisCall == 1);

	static_assert(!std::has_virtual_destructor_v<SourceHook::ISourceHook>);
	static_assert(!std::has_virtual_destructor_v<SourceHook::IHookContext>);
	static_assert(!std::has_virtual_destructor_v<SourceHook::ISHDelegate>);
	static_assert(!std::has_virtual_destructor_v<SourceHook::IHookManagerInfo>);

	#define ASSERT_SH_METHOD(name, return_type, ...) ASSERT_METHOD(SourceHook::ISourceHook, name, return_type, __VA_ARGS__)

	ASSERT_SH_METHOD(GetIfaceVersion, int);
	ASSERT_SH_METHOD(GetImplVersion, int);
	ASSERT_SH_METHOD(AddHook, int, SourceHook::Plugin, SourceHook::ISourceHook::AddHookMode, void *, int,
		SourceHook::HookManagerPubFunc, SourceHook::ISHDelegate *, bool);
	ASSERT_SH_METHOD(RemoveHook, bool, SourceHook::Plugin, void *, int, SourceHook::HookManagerPubFunc,
		SourceHook::ISHDelegate *, bool);
	ASSERT_SH_METHOD(RemoveHookByID, bool, int);
	ASSERT_SH_METHOD(PauseHookByID, bool, int);
	ASSERT_SH_METHOD(UnpauseHookByID, bool, int);
	ASSERT_SH_METHOD(SetRes, void, META_RES);
	ASSERT_SH_METHOD(GetPrevRes, META_RES);
	ASSERT_SH_METHOD(GetStatus, META_RES);
	ASSERT_SH_METHOD(GetOrigRet, const void *);
	ASSERT_SH_METHOD(GetOverrideRet, const void *);
	ASSERT_SH_METHOD(GetIfacePtr, void *);
	ASSERT_SH_METHOD(GetOverrideRetPtr, void *);
	ASSERT_SH_METHOD(RemoveHookManager, void, SourceHook::Plugin, SourceHook::HookManagerPubFunc);
	ASSERT_SH_METHOD(SetIgnoreHooks, void, void *);
	ASSERT_SH_METHOD(ResetIgnoreHooks, void, void *);
	ASSERT_SH_METHOD(GetOrigVfnPtrEntry, void *, void *);
	ASSERT_SH_METHOD(DoRecall, void);
	ASSERT_SH_METHOD(SetupHookLoop, SourceHook::IHookContext *, SourceHook::IHookManagerInfo *, void *, void *, void **,
		META_RES *, META_RES *, META_RES *, const void *, void *);
	ASSERT_SH_METHOD(EndContext, void, SourceHook::IHookContext *);
	ASSERT_SH_METHOD(LogDebug, void, const char *, ...);

	#undef ASSERT_SH_METHOD

	ASSERT_METHOD(SourceHook::IHookContext, GetNext, SourceHook::ISHDelegate *);
	ASSERT_METHOD(SourceHook::IHookContext, GetOverrideRetPtr, void *);
	ASSERT_METHOD(SourceHook::IHookContext, GetOrigRetPtr, const void *);
	ASSERT_METHOD(SourceHook::IHookContext, ShouldCallOrig, bool);
	ASSERT_METHOD(SourceHook::ISHDelegate, IsEqual, bool, SourceHook::ISHDelegate *);
	ASSERT_METHOD(SourceHook::ISHDelegate, DeleteThis, void);
	ASSERT_METHOD(SourceHook::IHookManagerInfo, SetInfo, void, int, int, int, SourceHook::ProtoInfo *, void *);
#else
	static_assert(std::is_same_v<KHook::HookID_t, std::uint32_t>);
	static_assert(KHook::INVALID_HOOK == UINT32_MAX);
	static_assert(sizeof(KHook::Action) == 1);
	static_assert(static_cast<int>(KHook::Action::Ignore) == 0);
	static_assert(static_cast<int>(KHook::Action::Override) == 1);
	static_assert(static_cast<int>(KHook::Action::Supersede) == 2);
	static_assert(!std::has_virtual_destructor_v<KHook::IKHook>);

	#define ASSERT_KHOOK_METHOD(name, return_type, ...) ASSERT_METHOD(KHook::IKHook, name, return_type, __VA_ARGS__)

	ASSERT_KHOOK_METHOD(SetupHook, KHook::HookID_t, void *, void *, void *, void *, void *, void *, void *, unsigned int, bool);
	ASSERT_KHOOK_METHOD(SetupVirtualHook, KHook::HookID_t, void **, int, void *, void *, void *, void *, void *, void *,
		unsigned int, bool);
	ASSERT_KHOOK_METHOD(RemoveHook, void, KHook::HookID_t, bool, void (*)(KHook::HookID_t, void *), void *);
	ASSERT_KHOOK_METHOD(GetContextPtr, void *);
	ASSERT_KHOOK_METHOD(GetOriginalFunction, void *);
	ASSERT_KHOOK_METHOD(GetOriginalValuePtr, void *);
	ASSERT_KHOOK_METHOD(GetOverrideValuePtr, void *);
	ASSERT_KHOOK_METHOD(GetCurrentValuePtr, void *, bool);
	ASSERT_KHOOK_METHOD(DestroyReturnValue, void);
	ASSERT_KHOOK_METHOD(FindOriginal, void *, void *);
	ASSERT_KHOOK_METHOD(FindOriginalVirtual, void *, void **, int);
	ASSERT_KHOOK_METHOD(DoRecall, void *, KHook::Action, void *, std::size_t, void *, void *);
	ASSERT_KHOOK_METHOD(SaveReturnValue, void, KHook::Action, void *, std::size_t, void *, void *, bool);
	ASSERT_KHOOK_METHOD(LookupSignature, void *, void *, std::size_t, const char *);
	ASSERT_KHOOK_METHOD(WasOriginalFunctionSkipped, bool);

	#undef ASSERT_KHOOK_METHOD
#endif

#undef ASSERT_METHOD

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

	// What Rust's hooks check before calling back: Metamod 2.0 neither pauses
	// a plugin's hooks nor removes them before they can run after `Unload`.
	struct RustPluginStatus {
		// Counts `Load` calls, telling apart what an earlier load of a library
		// that stayed mapped left behind.
		std::uint64_t generation;
		SourceMM::PluginId id;
		bool loaded;
		bool paused;
	};
}

static_assert(sizeof(RustPluginCallbacks) == 6 * sizeof(void *));
static_assert(sizeof(RustPluginMetadata) == 8 * sizeof(void *));
static_assert(sizeof(RustPluginStatus) == 16);
static_assert(offsetof(RustPluginStatus, id) == 8);
static_assert(offsetof(RustPluginStatus, loaded) == 12);
static_assert(offsetof(RustPluginStatus, paused) == 13);

extern "C" {
	// Metamod's level notifications. `map` is the name of the level loading.
	typedef void (*RustLevelInitFn)(void *context, const char *map);
	typedef void (*RustLevelShutdownFn)(void *context);

	enum RustHookStatus : int {
		RUST_HOOK_INSTALLED = 0,
		RUST_HOOK_NOT_BOUND = 1,
		RUST_HOOK_ALREADY_INSTALLED = 2,
		RUST_HOOK_INVALID_ARGUMENT = 3,
	};
}

namespace {
	struct LevelRoute {
		RustLevelInitFn init = nullptr;
		RustLevelShutdownFn shutdown = nullptr;
		void *context = nullptr;
		bool paused = false;
		std::uint64_t generation = 0;
		bool exhausted = false;

		void advance() noexcept {
			if (generation == UINT64_MAX)
				exhausted = true;
			else
				++generation;
		}
	};

	LevelRoute level_route;

	// Registered once per load, and removed by Metamod when it unloads the
	// plugin. It outlives every load, as a static of the library.
	class RustLevelListener final : public SourceMM::IMetamodListener {
	public:
		void OnLevelInit(char const *map, char const *, char const *, char const *, bool, bool) override {
			level_route.advance();
			const LevelRoute &route = level_route;

			if (route.init != nullptr && !route.paused)
				route.init(route.context, map);
		}

		void OnLevelShutdown() override {
			level_route.advance();
			const LevelRoute &route = level_route;

			if (route.shutdown != nullptr && !route.paused)
				route.shutdown(route.context);
		}
	};

	RustLevelListener level_listener;
	bool level_listener_added = false;

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

		RustPluginStatus status() const { return {generation_, id_, loaded_, paused_}; }

		int GetApiVersion() override { return METAMOD_PLAPI_VERSION; }

		bool Load(SourceMM::PluginId id, SourceMM::ISmmAPI *api, char *error, std::size_t max_length, bool late) override {
			// Drop what an earlier load of a library that stayed mapped left, such
			// as after a forced unload that followed a refused one.
			unload();
			++generation_;
			loaded_ = true;
			id_ = id;
			api_ = api;

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
			// the hooks are removed then, so they no longer find any.
			if (unloaded)
				unload();

			return unloaded;
		}

		bool Pause(char *error, std::size_t max_length) override {
			bool paused = callbacks_->pause(this, error, max_length);

			if (paused)
				set_paused(true);

			return paused;
		}

		bool Unpause(char *error, std::size_t max_length) override {
			bool unpaused = callbacks_->unpause(this, error, max_length);

			if (unpaused)
				set_paused(false);

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
		void set_paused(bool paused) {
			paused_ = paused;
			level_route.paused = paused;
		}

		void unload() {
			loaded_ = false;
			paused_ = false;
			id_ = 0;
			api_ = nullptr;
			// Metamod removes the level listener itself after `Unload`.
			level_listener_added = false;
			level_route = {};
		}

		const RustPluginCallbacks *callbacks_ = nullptr;
		SourceMM::ISmmAPI *api_ = nullptr;
		RustPluginMetadata metadata_ = {};
		std::uint64_t generation_ = 0;
		SourceMM::PluginId id_ = 0;
		bool loaded_ = false;
		bool paused_ = false;
	};

	RustMetamodPlugin plugin;
}

extern "C" void *RUST_SHELL_INSTANCE() {
	return &plugin;
}

extern "C" bool RUST_SHELL_IS_LOADED() {
	return plugin.loaded();
}

extern "C" RustPluginStatus RUST_SHELL_STATUS() {
	return plugin.status();
}

extern "C" void *RUST_SHELL_CREATE(const RustPluginCallbacks *callbacks, RustPluginMetadata metadata) {
	if (!callbacks)
		return nullptr;

	plugin.configure(callbacks, metadata);

	return &plugin;
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

// Main-thread-only owned counter: notifications advance it even while paused.
// Unavailable before registration, after unload, or after exhausting the counter.
extern "C" bool RUST_SHELL_LEVEL_GENERATION(std::uint64_t *generation) noexcept {
	if (generation == nullptr || !plugin.loaded() || !level_listener_added || level_route.exhausted)
		return false;
	*generation = level_route.generation;
	return true;
}
