#include <cstddef>
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
#else
	#if METAMOD_PLAPI_VERSION != 18
		#error "The dev Metamod shell requires the 2.0 build 1469 plugin headers"
	#endif

	static_assert(offsetof(MetamodVersionInfo, pl_min) == 8);
	static_assert(offsetof(MetamodVersionInfo, source_engine) == 16);

	#define RUST_SHELL_CREATE cpp_metamod_plugin_dev
	#define RUST_SHELL_INSTANCE cpp_metamod_plugin_dev_instance
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

namespace {
	class RustMetamodPlugin final : public SourceMM::ISmmPlugin {
	public:
		void configure(const RustPluginCallbacks *callbacks, RustPluginMetadata metadata) {
			callbacks_ = callbacks;
			metadata_ = metadata;
		}

		int GetApiVersion() override { return METAMOD_PLAPI_VERSION; }

		bool Load(SourceMM::PluginId, SourceMM::ISmmAPI *api, char *error, std::size_t max_length, bool late) override {
			return callbacks_->load(this, api, error, max_length, late);
		}

		void AllPluginsLoaded() override { callbacks_->all_plugins_loaded(this); }
		bool QueryRunning(char *error, std::size_t max_length) override { return callbacks_->query_running(this, error, max_length); }
		bool Unload(char *error, std::size_t max_length) override { return callbacks_->unload(this, error, max_length); }
		bool Pause(char *error, std::size_t max_length) override { return callbacks_->pause(this, error, max_length); }
		bool Unpause(char *error, std::size_t max_length) override { return callbacks_->unpause(this, error, max_length); }

		const char *GetAuthor() override { return metadata_.author; }
		const char *GetName() override { return metadata_.name; }
		const char *GetDescription() override { return metadata_.description; }
		const char *GetURL() override { return metadata_.url; }
		const char *GetLicense() override { return metadata_.license; }
		const char *GetVersion() override { return metadata_.version; }
		const char *GetDate() override { return metadata_.date; }
		const char *GetLogTag() override { return metadata_.log_tag; }

	private:
		const RustPluginCallbacks *callbacks_ = nullptr;
		RustPluginMetadata metadata_ = {};
	};

	RustMetamodPlugin plugin;
}

extern "C" void *RUST_SHELL_INSTANCE() {
	return &plugin;
}

extern "C" void *RUST_SHELL_CREATE(const RustPluginCallbacks *callbacks, RustPluginMetadata metadata) {
	if (!callbacks)
		return nullptr;

	plugin.configure(callbacks, metadata);

	return &plugin;
}
