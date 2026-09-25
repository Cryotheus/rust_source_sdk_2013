//! C++ record metadata and synthetic vtable probes.
//!
//! Bindgen deliberately leaves several C++ vtables opaque, most notably when
//! inheritance or a virtual destructor is involved.  This module collects the
//! small amount of libclang metadata needed to describe those tables without
//! asking bindgen to emit callable C++ methods.
//!
//! Records are keyed by their qualified C++ name. Class template
//! specializations include their template arguments in that key, so
//! `CEntityOutputTemplate<class Vector, FIELD_VECTOR>` and the primary
//! `CEntityOutputTemplate` pattern are distinct records. Anything whose primary
//! vtable cannot be derived faithfully (template-dependent layouts, several
//! polymorphic bases, virtual bases, covariant overriders) is reported as
//! [`VtableModel::Unsupported`] rather than guessed.
#![allow(dead_code, non_upper_case_globals)]

use clang_sys::*;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{CStr, CString};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::ptr::{null_mut, slice_from_raw_parts};
use std::sync::{Arc, OnceLock};

/// A direct base class declaration.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct BaseRecord {
	/// Record key of the base, including template arguments for a class
	/// template specialization. For a dependent base, the type spelling.
	pub qualified_name: String,
	pub type_spelling: String,
	pub is_virtual: bool,
	pub target: BaseTarget,
}

/// What a base class specifier refers to.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum BaseTarget {
	/// A record definition collected under [`BaseRecord::qualified_name`].
	Record,

	/// A class template specialization. An explicit specialization is
	/// collected under [`BaseRecord::qualified_name`]; an implicit
	/// instantiation is only described by its template or partial
	/// specialization, collected under `pattern`.
	Specialization { pattern: String },

	/// A template parameter, or a specialization which depends on one.
	Dependent,
}

struct Collector {
	records: BTreeMap<String, Record>,
	typedef_named_anonymous_records: BTreeSet<String>,
	error: Option<CppVtableError>,
	source_root: Option<SourceRootFilter>,
}

impl Collector {
	fn new(source_root: Option<&Path>) -> Self {
		Self {
			records: BTreeMap::new(),
			typedef_named_anonymous_records: BTreeSet::new(),
			error: None,
			source_root: source_root.map(SourceRootFilter::new),
		}
	}

	fn includes_source_path(&mut self, source_path: Option<&Path>) -> bool {
		match &mut self.source_root {
			None => true,
			Some(filter) => source_path.is_some_and(|path| filter.includes(path)),
		}
	}
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CppVtableError {
	#[error("could not load libclang: {0}")]
	LoadLibclang(String),

	#[error("{context} contains an interior NUL byte")]
	InteriorNul { context: String },

	#[error("libclang failed to parse {path:?} (CXErrorCode {code})")]
	ParseTranslationUnit { path: PathBuf, code: CXErrorCode },

	#[error("libclang rejected {path:?}:\n{diagnostics}")]
	Diagnostics { path: PathBuf, diagnostics: String },

	#[error("libclang traversal panicked: {0}")]
	TraversalPanic(String),

	#[error("duplicate complete C++ record definition for {0:?}")]
	DuplicateRecord(String),

	#[error("missing complete C++ record definition for {0:?}")]
	MissingRecord(String),

	#[error("the vtable of {record:?} cannot be modeled: {reason}")]
	Unsupported { record: String, reason: String },

	#[error("inheritance cycle while computing the vtable for {0:?}")]
	InheritanceCycle(String),

	#[error("virtual method {method:?} on {record:?} overrides no slot in its primary base")]
	MissingOverrideSlot { record: String, method: String },

	#[error(
		"virtual method {method:?} on {record:?} ambiguously overrides {count} primary-base slots"
	)]
	AmbiguousOverrideSlots {
		record: String,
		method: String,
		count: usize,
	},
}

/// Borrowed AST cursor. It is only valid while its translation unit is alive.
///
/// The wrapper does not encode that lifetime, so libclang queries remain
/// `unsafe` and must be made before the owning [`TranslationUnit`] is dropped.
#[derive(Clone, Copy)]
struct Cursor(CXCursor);

impl Cursor {
	unsafe fn kind(self) -> CXCursorKind {
		unsafe { clang_getCursorKind(self.0) }
	}

	unsafe fn is_null(self) -> bool {
		unsafe { clang_Cursor_isNull(self.0) != 0 }
	}

	unsafe fn is_definition(self) -> bool {
		unsafe { clang_isCursorDefinition(self.0) != 0 }
	}

	unsafe fn is_anonymous(self) -> bool {
		unsafe { clang_Cursor_isAnonymous(self.0) != 0 }
	}

	unsafe fn is_virtual_method(self) -> bool {
		unsafe { clang_CXXMethod_isVirtual(self.0) != 0 }
	}

	unsafe fn semantic_parent(self) -> Self {
		Self(unsafe { clang_getCursorSemanticParent(self.0) })
	}

	unsafe fn specialized_template(self) -> Self {
		Self(unsafe { clang_getSpecializedCursorTemplate(self.0) })
	}

	unsafe fn cursor_type(self) -> Type {
		Type(unsafe { clang_getCursorType(self.0) })
	}

	unsafe fn usr(self) -> String {
		unsafe { cx_string(clang_getCursorUSR(self.0)) }
	}

	unsafe fn display_name(self) -> String {
		unsafe { cx_string(clang_getCursorDisplayName(self.0)) }
	}

	/// Whether this record is a class template specialization (explicit, or
	/// the instantiation named by a base specifier), or a member class of one.
	unsafe fn is_template_specialization(self) -> bool {
		unsafe {
			matches!(
				self.kind(),
				CXCursor_ClassDecl | CXCursor_StructDecl | CXCursor_UnionDecl
			) && !self.specialized_template().is_null()
		}
	}

	/// Whether a class template or partial specialization encloses this cursor.
	unsafe fn has_dependent_context(self) -> bool {
		let mut current = unsafe { self.semantic_parent() };

		while !unsafe { current.is_null() } {
			match unsafe { current.kind() } {
				CXCursor_TranslationUnit => return false,
				CXCursor_ClassTemplate | CXCursor_ClassTemplatePartialSpecialization => {
					return true;
				}
				_ => {}
			}

			current = unsafe { current.semantic_parent() };
		}

		false
	}

	unsafe fn template_role(self, children: &RecordChildren) -> TemplateRole {
		if unsafe {
			matches!(
				self.kind(),
				CXCursor_ClassTemplate | CXCursor_ClassTemplatePartialSpecialization
			)
		} {
			return TemplateRole::Pattern;
		}

		if unsafe { self.has_dependent_context() } {
			return TemplateRole::DependentMember;
		}

		if !unsafe { self.is_template_specialization() } {
			return TemplateRole::None;
		}

		// libclang exposes the members of an explicit specialization, but none
		// of an explicit instantiation (`template class X<int>;`), whose layout
		// comes from its template. Only an empty body needs the tokens.
		if children.has_members {
			return TemplateRole::ExplicitSpecialization;
		}

		match unsafe { self.leading_tokens(2) }.as_slice() {
			[template, angle] if template == "template" && angle == "<" => {
				TemplateRole::ExplicitSpecialization
			}
			[template, key]
				if template == "template"
					&& matches!(key.as_str(), "class" | "struct" | "union") =>
			{
				TemplateRole::ExplicitInstantiation
			}
			_ => TemplateRole::UnclassifiedSpecialization,
		}
	}

	unsafe fn leading_tokens(self, count: usize) -> Vec<String> {
		let translation_unit = unsafe { clang_Cursor_getTranslationUnit(self.0) };
		let extent = unsafe { clang_getCursorExtent(self.0) };
		let mut tokens = null_mut();
		let mut token_count = 0;

		unsafe {
			clang_tokenize(
				translation_unit,
				extent,
				&raw mut tokens,
				&raw mut token_count,
			)
		};

		if tokens.is_null() {
			return Vec::new();
		}

		let spellings = unsafe { &*slice_from_raw_parts(tokens, token_count as usize) }
			.iter()
			.take(count)
			.map(|token| unsafe { cx_string(clang_getTokenSpelling(translation_unit, *token)) })
			.collect();

		unsafe { clang_disposeTokens(translation_unit, tokens, token_count) };

		spellings
	}

	unsafe fn collect_record(
		self,
		source_path: Option<PathBuf>,
	) -> Result<Option<Record>, CppVtableError> {
		let name = unsafe { self.spelling() };
		if name.is_empty() {
			return Ok(None);
		}

		let qualified_name = unsafe { self.qualified_name() };
		let mut children = RecordChildren::default();
		unsafe {
			clang_visitChildren(
				self.0,
				collect_record_child_visitor,
				(&raw mut children).cast(),
			)
		};
		if let Some(error) = children.error.take() {
			return Err(error);
		}

		Ok(Some(Record {
			usr: unsafe { self.usr() },
			name,
			qualified_name,
			namespaces: unsafe { self.namespaces() },
			source_path,
			template: unsafe { self.template_role(&children) },
			bases: children.bases,
			virtual_methods: children.virtual_methods,
			member_names: children.member_names,
		}))
	}

	unsafe fn collect_base(self) -> BaseRecord {
		let base_type = unsafe { self.cursor_type() };
		let canonical_type = unsafe { base_type.canonical() };
		let type_spelling = unsafe { base_type.spelling() };
		let is_virtual = unsafe { clang_isVirtualBase(self.0) != 0 };

		// Inside a template pattern a base may be a template parameter or a
		// specialization that depends on one. Neither names a record yet.
		let mut declaration = unsafe { canonical_type.declaration() };

		if canonical_type.0.kind != CXType_Record || unsafe { declaration.is_null() } {
			return BaseRecord {
				qualified_name: type_spelling.clone(),
				type_spelling,
				is_virtual,
				target: BaseTarget::Dependent,
			};
		}

		let definition = Self(unsafe { clang_getCursorDefinition(declaration.0) });
		if !unsafe { definition.is_null() } {
			declaration = definition;
		}

		let target = if unsafe { declaration.is_template_specialization() } {
			BaseTarget::Specialization {
				pattern: unsafe { declaration.specialized_template().qualified_name() },
			}
		} else {
			BaseTarget::Record
		};

		BaseRecord {
			qualified_name: unsafe { declaration.qualified_name() },
			type_spelling,
			is_virtual,
			target,
		}
	}

	unsafe fn collect_virtual_method(
		self,
		kind: VirtualMethodKind,
	) -> Result<VirtualMethod, CppVtableError> {
		let name = unsafe { self.spelling() };
		let parent_name = unsafe { self.semantic_parent().qualified_name() };
		let qualified_name = if parent_name.is_empty() {
			name.clone()
		} else {
			format!("{parent_name}::{name}")
		};
		let method_type = unsafe { self.cursor_type() };
		let argument_count = unsafe { clang_Cursor_getNumArguments(self.0) };
		let mut parameters = Vec::with_capacity(argument_count.max(0) as usize);
		for index in 0..argument_count.max(0) as u32 {
			let argument = Self(unsafe { clang_Cursor_getArgument(self.0, index) });
			parameters.push(Parameter {
				name: unsafe { argument.spelling() },
				type_spelling: unsafe { argument.cursor_type().globally_qualified_spelling() },
			});
		}

		let mut overridden_cursors = null_mut();
		let mut overridden_count = 0;
		unsafe {
			clang_getOverriddenCursors(
				self.0,
				&raw mut overridden_cursors,
				&raw mut overridden_count,
			);
		}
		let mut overrides = Vec::with_capacity(overridden_count as usize);
		if overridden_count != 0 && !overridden_cursors.is_null() {
			let cursors =
				unsafe { &*slice_from_raw_parts(overridden_cursors, overridden_count as usize) };
			for overridden in cursors {
				overrides.push(unsafe { Self(*overridden).method_reference() });
			}
		}
		if !overridden_cursors.is_null() {
			unsafe { clang_disposeOverriddenCursors(overridden_cursors) };
		}

		let (result_type, canonical_result_type) = if kind == VirtualMethodKind::Destructor {
			("void".to_owned(), "void".to_owned())
		} else {
			let result = Type(unsafe { clang_getCursorResultType(self.0) });
			unsafe {
				(
					result.globally_qualified_spelling(),
					result.canonical().spelling(),
				)
			}
		};

		Ok(VirtualMethod {
			usr: unsafe { self.usr() },
			name,
			qualified_name,
			display_name: unsafe { self.display_name() },
			kind,
			is_const: kind == VirtualMethodKind::Method
				&& unsafe { clang_CXXMethod_isConst(self.0) != 0 },
			result_type,
			canonical_result_type,
			parameters,
			is_variadic: unsafe { clang_isFunctionTypeVariadic(method_type.0) != 0 },
			overrides,
		})
	}

	unsafe fn method_reference(self) -> MethodReference {
		let name = unsafe { self.spelling() };
		let parent = unsafe { self.semantic_parent().qualified_name() };
		MethodReference {
			usr: unsafe { self.usr() },
			qualified_name: if parent.is_empty() {
				name
			} else {
				format!("{parent}::{name}")
			},
			display_name: unsafe { self.display_name() },
		}
	}

	/// The record key: the `::`-joined scopes and name, where every class
	/// template specialization keeps its template arguments.
	unsafe fn qualified_name(self) -> String {
		let mut parts = Vec::new();
		let mut current = self;
		while !unsafe { current.is_null() } {
			if unsafe { current.kind() } == CXCursor_TranslationUnit {
				break;
			}

			if let Some(segment) = unsafe { current.qualified_name_segment() } {
				parts.push(segment);
			}

			current = unsafe { current.semantic_parent() };
		}

		parts.reverse();
		parts.join("::")
	}

	unsafe fn qualified_name_segment(self) -> Option<String> {
		let segment = match unsafe { self.kind() } {
			CXCursor_Namespace | CXCursor_ClassTemplate => unsafe { self.spelling() },
			CXCursor_ClassTemplatePartialSpecialization => unsafe { self.display_name() },
			CXCursor_ClassDecl | CXCursor_StructDecl | CXCursor_UnionDecl
				if unsafe { self.is_template_specialization() } =>
			unsafe { self.display_name() },
			CXCursor_ClassDecl | CXCursor_StructDecl | CXCursor_UnionDecl => unsafe {
				self.spelling()
			},
			_ => return None,
		};

		(!segment.is_empty()).then_some(segment)
	}

	unsafe fn namespaces(self) -> Vec<String> {
		let mut namespaces = Vec::new();
		let mut current = unsafe { self.semantic_parent() };
		while !unsafe { current.is_null() } {
			let kind = unsafe { current.kind() };
			if kind == CXCursor_TranslationUnit {
				break;
			}
			if kind == CXCursor_Namespace {
				let spelling = unsafe { current.spelling() };
				if !spelling.is_empty() {
					namespaces.push(spelling);
				}
			}
			current = unsafe { current.semantic_parent() };
		}
		namespaces.reverse();
		namespaces
	}

	unsafe fn source_path(self) -> Option<PathBuf> {
		let location = unsafe { clang_getCursorLocation(self.0) };
		let mut file = null_mut();
		unsafe {
			clang_getSpellingLocation(location, &raw mut file, null_mut(), null_mut(), null_mut());
		}
		if file.is_null() {
			return None;
		}

		let file_name = unsafe { cx_string(clang_getFileName(file)) };
		(!file_name.is_empty()).then(|| PathBuf::from(file_name))
	}

	/// A spelling of this type declaration that is valid at global scope.
	unsafe fn qualified_type_declaration_name(self) -> String {
		let name = unsafe { self.spelling() };
		if name.is_empty() {
			return String::new();
		}

		let mut parts = vec![name];
		let mut current = unsafe { self.semantic_parent() };
		while !unsafe { current.is_null() } {
			let kind = unsafe { current.kind() };
			if kind == CXCursor_TranslationUnit {
				break;
			}
			let segment = match kind {
				CXCursor_ClassDecl | CXCursor_StructDecl | CXCursor_UnionDecl
					if unsafe { current.is_template_specialization() } =>
				unsafe { current.display_name() },
				CXCursor_Namespace
				| CXCursor_ClassDecl
				| CXCursor_StructDecl
				| CXCursor_UnionDecl
				| CXCursor_ClassTemplate
				| CXCursor_ClassTemplatePartialSpecialization => unsafe { current.spelling() },
				_ => String::new(),
			};
			if !segment.is_empty() {
				parts.push(segment);
			}
			current = unsafe { current.semantic_parent() };
		}

		parts.reverse();
		parts.join("::")
	}

	unsafe fn spelling(self) -> String {
		unsafe { cx_string(clang_getCursorSpelling(self.0)) }
	}
}

/// The C++ ABI rules which affect virtual destructor slots.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum CppAbi {
	/// The ABI used by GCC and Clang on the supported Linux targets.
	Itanium,

	/// The Microsoft C++ ABI used by 64-bit MSVC-compatible Windows targets.
	///
	/// The x86-64 Windows calling convention is uniform, so ordinary function
	/// pointer aliases model virtual slots correctly.
	Msvc,

	/// The Microsoft C++ ABI used by 32-bit x86 MSVC-compatible targets.
	///
	/// Virtual member functions use `__thiscall` on this target. Keeping this
	/// distinct from [`Self::Msvc`] prevents silently generating cdecl slots.
	MsvcX86,
}

impl CppAbi {
	const fn is_microsoft(self) -> bool {
		matches!(self, Self::Msvc | Self::MsvcX86)
	}
}

/// RAII owner for libclang's translation-unit creation context.
///
/// An index groups parsing work and must remain alive for every translation
/// unit created from it. In [`collect_records_with_source_root`] it is declared
/// before the [`TranslationUnit`], so Rust's reverse drop order disposes the
/// translation unit first and the index afterward.
struct Index(CXIndex);

impl Index {
	unsafe fn new() -> Self {
		Self(unsafe { clang_createIndex(0, 0) })
	}
}

impl Drop for Index {
	fn drop(&mut self) {
		unsafe { clang_disposeIndex(self.0) };
	}
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct MethodReference {
	pub usr: String,
	pub qualified_name: String,
	pub display_name: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct Parameter {
	pub name: String,
	pub type_spelling: String,
}

/// One complete C++ class, struct, class template, or partial specialization definition.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct Record {
	pub usr: String,
	pub name: String,
	pub qualified_name: String,
	pub namespaces: Vec<String>,
	pub source_path: Option<PathBuf>,
	pub template: TemplateRole,
	pub bases: Vec<BaseRecord>,
	pub virtual_methods: Vec<VirtualMethod>,

	/// Names of the record's own member declarations, in order of first
	/// declaration. The Microsoft ABI groups new virtual overloads by it.
	pub member_names: Vec<String>,
}

/// How a record relates to C++ templates.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum TemplateRole {
	/// An ordinary record, including a member of an explicit specialization.
	None,

	/// The definition of a class template or of a partial specialization.
	Pattern,

	/// A record nested in a [`Self::Pattern`]. Its layout depends on the
	/// enclosing template's arguments.
	DependentMember,

	/// A full explicit specialization, `template <> class X<int> { ... };`.
	ExplicitSpecialization,

	/// An explicit instantiation, `template class X<int>;`. libclang exposes
	/// no members for it; its template describes its layout.
	ExplicitInstantiation,

	/// A specialization without members whose kind is hidden by a macro.
	UnclassifiedSpecialization,
}

#[derive(Default)]
struct RecordChildren {
	bases: Vec<BaseRecord>,
	virtual_methods: Vec<VirtualMethod>,
	member_names: Vec<String>,
	has_members: bool,
	error: Option<CppVtableError>,
}

/// A collection of complete C++ record definitions, keyed by qualified name.
#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub(crate) struct RecordIndex {
	records: BTreeMap<String, Record>,
	typedef_named_anonymous_records: BTreeSet<String>,
}

/// Whether and how the primary vtable of one record can be described.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum VtableModel {
	/// Neither the record nor any base declares a virtual function.
	NotPolymorphic,

	/// Every primary vtable slot, in ABI order.
	Modeled(VtableLayout),

	/// The record may have a vtable, but its layout cannot be derived
	/// faithfully from the collected metadata.
	Unsupported { reason: String },
}

/// Primary-vtable slots, or why they are unknown.
enum Slots {
	Known(Vec<VtableSlot>),
	Unknown(String),
}

impl RecordIndex {
	/// Returns whether `qualified_name` declares or inherits any virtual slots.
	///
	/// This is ABI-independent. Records whose bases cannot be resolved report
	/// an [`CppVtableError::Unsupported`] error.
	pub(crate) fn has_virtual_slots(&self, qualified_name: &str) -> Result<bool, CppVtableError> {
		match self.vtable_model(qualified_name, CppAbi::Itanium)? {
			VtableModel::NotPolymorphic => Ok(false),
			VtableModel::Modeled(_) => Ok(true),
			VtableModel::Unsupported { reason } => Err(CppVtableError::Unsupported {
				record: qualified_name.to_owned(),
				reason,
			}),
		}
	}

	pub(crate) fn record(&self, qualified_name: &str) -> Option<&Record> {
		self.records.get(qualified_name)
	}

	pub(crate) fn records(&self) -> &BTreeMap<String, Record> {
		&self.records
	}

	/// Typedef names of unnamed records anywhere in the translation unit,
	/// such as `typedef union { ... } pthread_mutex_t;`.
	pub(crate) fn typedef_named_anonymous_records(&self) -> &BTreeSet<String> {
		&self.typedef_named_anonymous_records
	}

	/// Computes the primary vtable address-point layout for `qualified_name`,
	/// or explains why it cannot be derived.
	pub(crate) fn vtable_model(
		&self,
		qualified_name: &str,
		abi: CppAbi,
	) -> Result<VtableModel, CppVtableError> {
		if !self.records.contains_key(qualified_name) {
			return Err(CppVtableError::MissingRecord(qualified_name.to_owned()));
		}

		let mut active = BTreeSet::new();

		Ok(match self.record_slots(qualified_name, abi, &mut active)? {
			Slots::Known(slots) if slots.is_empty() => VtableModel::NotPolymorphic,
			Slots::Known(slots) => VtableModel::Modeled(VtableLayout {
				record: qualified_name.to_owned(),
				abi,
				slots,
			}),
			Slots::Unknown(reason) => VtableModel::Unsupported { reason },
		})
	}

	/// Computes the primary vtable address-point layout for `qualified_name`.
	///
	/// A record without virtual functions has an empty layout.
	pub(crate) fn vtable_layout(
		&self,
		qualified_name: &str,
		abi: CppAbi,
	) -> Result<VtableLayout, CppVtableError> {
		match self.vtable_model(qualified_name, abi)? {
			VtableModel::Modeled(layout) => Ok(layout),
			VtableModel::NotPolymorphic => Ok(VtableLayout {
				record: qualified_name.to_owned(),
				abi,
				slots: Vec::new(),
			}),
			VtableModel::Unsupported { reason } => Err(CppVtableError::Unsupported {
				record: qualified_name.to_owned(),
				reason,
			}),
		}
	}

	fn record_slots(
		&self,
		qualified_name: &str,
		abi: CppAbi,
		active: &mut BTreeSet<String>,
	) -> Result<Slots, CppVtableError> {
		let Some(record) = self.records.get(qualified_name) else {
			return Ok(Slots::Unknown(format!(
				"the definition of `{qualified_name}` was not collected"
			)));
		};

		match record.template {
			TemplateRole::None | TemplateRole::ExplicitSpecialization => {}

			// A template's own struct is generic, so it never gets a probe. It is
			// still non-polymorphic when no instantiation can have a vtable.
			TemplateRole::Pattern | TemplateRole::DependentMember => {
				return Ok(
					match self.instantiation_slots(qualified_name, abi, active)? {
						Slots::Known(slots) if slots.is_empty() => Slots::Known(slots),
						Slots::Known(_) | Slots::Unknown(_) => Slots::Unknown(format!(
							"`{qualified_name}` depends on template arguments"
						)),
					},
				);
			}

			TemplateRole::ExplicitInstantiation | TemplateRole::UnclassifiedSpecialization => {
				return Ok(Slots::Unknown(format!(
					"libclang exposes no members of `{qualified_name}`"
				)));
			}
		}

		Self::guarded(active, qualified_name, |active| {
			self.derived_slots(record, &record.bases, Some(record), abi, active)
		})
	}

	/// The slots of an implicit (or explicit) instantiation of `pattern_name`.
	///
	/// Signatures inside a template depend on its arguments, so only templates
	/// without virtual functions of their own are modeled. Their instantiations
	/// contribute exactly the slots of their non-dependent bases.
	fn instantiation_slots(
		&self,
		pattern_name: &str,
		abi: CppAbi,
		active: &mut BTreeSet<String>,
	) -> Result<Slots, CppVtableError> {
		let Some(pattern) = self.records.get(pattern_name) else {
			return Ok(Slots::Unknown(format!(
				"the definition of template `{pattern_name}` was not collected"
			)));
		};

		if !pattern.virtual_methods.is_empty() {
			return Ok(Slots::Unknown(format!(
				"template `{pattern_name}` declares virtual functions"
			)));
		}

		Self::guarded(active, pattern_name, |active| {
			self.derived_slots(pattern, &pattern.bases, None, abi, active)
		})
	}

	fn base_slots(
		&self,
		base: &BaseRecord,
		abi: CppAbi,
		active: &mut BTreeSet<String>,
	) -> Result<Slots, CppVtableError> {
		match &base.target {
			BaseTarget::Dependent => Ok(Slots::Unknown(
				"it depends on template parameters".to_owned(),
			)),
			BaseTarget::Record => self.record_slots(&base.qualified_name, abi, active),
			BaseTarget::Specialization { pattern } => match self
				.records
				.get(&base.qualified_name)
				.map(|record| record.template)
			{
				Some(TemplateRole::ExplicitSpecialization) => {
					self.record_slots(&base.qualified_name, abi, active)
				}
				Some(TemplateRole::UnclassifiedSpecialization) => {
					self.record_slots(&base.qualified_name, abi, active)
				}
				_ => self.instantiation_slots(pattern, abi, active),
			},
		}
	}

	/// Combines the bases' primary vtables and applies `own` virtual methods.
	///
	/// Exactly one base may be polymorphic: its vtable is the record's primary
	/// vtable under both ABIs. Several polymorphic bases need secondary
	/// vtables, which a single probe cannot describe.
	fn derived_slots(
		&self,
		record: &Record,
		bases: &[BaseRecord],
		own: Option<&Record>,
		abi: CppAbi,
		active: &mut BTreeSet<String>,
	) -> Result<Slots, CppVtableError> {
		const VIRTUAL_BASES: &str = "virtual base classes are not modeled";

		let mut primary = None::<Vec<VtableSlot>>;
		let mut has_virtual_base = false;

		for base in bases {
			match self.base_slots(base, abi, active)? {
				Slots::Unknown(reason) => {
					return Ok(Slots::Unknown(format!(
						"base `{}`: {reason}",
						base.type_spelling
					)));
				}

				// A virtual base without virtual functions adds no function slots,
				// but it does change where the vtable pointers live.
				Slots::Known(slots) if base.is_virtual => {
					if !slots.is_empty() {
						return Ok(Slots::Unknown(VIRTUAL_BASES.to_owned()));
					}

					has_virtual_base = true;
				}

				Slots::Known(slots) if slots.is_empty() => {}

				Slots::Known(slots) => {
					if primary.is_some() {
						return Ok(Slots::Unknown(format!(
							"`{}` has more than one polymorphic base class",
							record.qualified_name
						)));
					}

					primary = Some(slots);
				}
			}
		}

		let mut slots = primary.unwrap_or_default();

		if let Some(own) = own
			&& let Some(reason) = apply_record_methods(&mut slots, own, abi)?
		{
			return Ok(Slots::Unknown(reason));
		}

		if has_virtual_base && !slots.is_empty() {
			return Ok(Slots::Unknown(VIRTUAL_BASES.to_owned()));
		}

		Ok(Slots::Known(slots))
	}

	fn guarded<T>(
		active: &mut BTreeSet<String>,
		qualified_name: &str,
		compute: impl FnOnce(&mut BTreeSet<String>) -> Result<T, CppVtableError>,
	) -> Result<T, CppVtableError> {
		if !active.insert(qualified_name.to_owned()) {
			return Err(CppVtableError::InheritanceCycle(qualified_name.to_owned()));
		}

		let result = compute(active);
		active.remove(qualified_name);
		result
	}
}

struct SourceRootFilter {
	root_components: Vec<String>,
	cache: BTreeMap<PathBuf, bool>,
}

impl SourceRootFilter {
	fn new(source_root: &Path) -> Self {
		Self {
			root_components: comparable_path_components(source_root),
			cache: BTreeMap::new(),
		}
	}

	fn includes(&mut self, source_path: &Path) -> bool {
		if let Some(included) = self.cache.get(source_path) {
			return *included;
		}

		let source_components = comparable_path_components(source_path);
		let included = source_components.len() >= self.root_components.len()
			&& source_components[..self.root_components.len()] == self.root_components;
		self.cache.insert(source_path.to_path_buf(), included);
		included
	}
}

/// RAII owner for one parsed libclang translation unit.
///
/// The handle owns the AST, diagnostics, and storage backing the cursors used
/// during record collection. Dropping it after traversal releases that native
/// state with `clang_disposeTranslationUnit` and prevents it from outliving
/// the [`Index`] that created it.
struct TranslationUnit(CXTranslationUnit);

impl TranslationUnit {
	unsafe fn cursor(&self) -> Cursor {
		Cursor(unsafe { clang_getTranslationUnitCursor(self.0) })
	}

	unsafe fn error_diagnostics(&self) -> Vec<String> {
		let count = unsafe { clang_getNumDiagnostics(self.0) };
		let mut errors = Vec::new();
		for index in 0..count {
			let diagnostic = unsafe { clang_getDiagnostic(self.0, index) };
			let severity = unsafe { clang_getDiagnosticSeverity(diagnostic) };
			if severity >= CXDiagnostic_Error {
				let formatted = unsafe {
					clang_formatDiagnostic(diagnostic, clang_defaultDiagnosticDisplayOptions())
				};
				errors.push(unsafe { cx_string(formatted) });
			}
			unsafe { clang_disposeDiagnostic(diagnostic) };
		}
		errors
	}
}

impl Drop for TranslationUnit {
	fn drop(&mut self) {
		unsafe { clang_disposeTranslationUnit(self.0) };
	}
}

/// Borrowed C++ type. It has the same translation-unit lifetime as its cursor.
#[derive(Clone, Copy)]
struct Type(CXType);

impl Type {
	unsafe fn canonical(self) -> Self {
		Self(unsafe { clang_getCanonicalType(self.0) })
	}

	unsafe fn declaration(self) -> Cursor {
		Cursor(unsafe { clang_getTypeDeclaration(self.0) })
	}

	unsafe fn spelling(self) -> String {
		unsafe { cx_string(clang_getTypeSpelling(self.0)) }
	}

	unsafe fn globally_qualified_spelling(self) -> String {
		let spelling = unsafe { self.spelling() };
		let mut replacements = BTreeMap::new();
		let mut visited = BTreeSet::new();
		unsafe { self.collect_qualifications(&mut replacements, &mut visited) };

		let mut qualified = spelling;
		for (name, qualified_name) in replacements {
			qualified = replace_unqualified_identifier(&qualified, &name, &qualified_name);
		}
		qualified
	}

	unsafe fn collect_qualifications(
		self,
		replacements: &mut BTreeMap<String, String>,
		visited: &mut BTreeSet<String>,
	) {
		let spelling = unsafe { self.spelling() };
		let key = format!("{}:{spelling}", self.0.kind);
		if !visited.insert(key) {
			return;
		}

		let declaration = unsafe { self.declaration() };
		if !unsafe { declaration.is_null() } {
			let name = unsafe { declaration.spelling() };
			let qualified_name = unsafe { declaration.qualified_type_declaration_name() };
			if !name.is_empty() && !qualified_name.is_empty() {
				replacements.insert(name, format!("::{qualified_name}"));
			}
		}

		match self.0.kind {
			CXType_Pointer
			| CXType_BlockPointer
			| CXType_LValueReference
			| CXType_RValueReference
			| CXType_MemberPointer
			| CXType_ObjCObjectPointer => {
				unsafe {
					Self(clang_getPointeeType(self.0)).collect_qualifications(replacements, visited)
				};
			}
			CXType_ConstantArray
			| CXType_IncompleteArray
			| CXType_VariableArray
			| CXType_DependentSizedArray => {
				unsafe {
					Self(clang_getArrayElementType(self.0))
						.collect_qualifications(replacements, visited)
				};
			}
			CXType_FunctionNoProto | CXType_FunctionProto => {
				unsafe {
					Self(clang_getResultType(self.0)).collect_qualifications(replacements, visited)
				};
				let count = unsafe { clang_getNumArgTypes(self.0) };
				for index in 0..count.max(0) as u32 {
					unsafe {
						Self(clang_getArgType(self.0, index))
							.collect_qualifications(replacements, visited)
					};
				}
			}
			CXType_Elaborated => unsafe {
				Self(clang_Type_getNamedType(self.0)).collect_qualifications(replacements, visited);
			},
			_ => {}
		}

		let template_argument_count = unsafe { clang_Type_getNumTemplateArguments(self.0) };
		for index in 0..template_argument_count.max(0) as u32 {
			let argument = unsafe { clang_Type_getTemplateArgumentAsType(self.0, index) };
			if argument.kind != CXType_Invalid {
				unsafe { Self(argument).collect_qualifications(replacements, visited) };
			}
		}
	}
}

/// A virtual member function or virtual destructor directly declared by a record.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct VirtualMethod {
	pub usr: String,
	pub name: String,
	pub qualified_name: String,
	pub display_name: String,
	pub kind: VirtualMethodKind,
	pub is_const: bool,
	pub result_type: String,

	/// The canonical result type, which detects covariant overriders.
	pub canonical_result_type: String,
	pub parameters: Vec<Parameter>,
	pub is_variadic: bool,
	pub overrides: Vec<MethodReference>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum VirtualMethodKind {
	Method,
	Destructor,
}

/// The fully expanded address-point slots for one record and ABI.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct VtableLayout {
	pub record: String,
	pub abi: CppAbi,
	pub slots: Vec<VtableSlot>,
}

/// Rendered C++ declarations that bindgen can consume as ordinary aliases and a struct.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct VtableProbe {
	pub record: String,

	/// Rust/C++ identifier stem corresponding to [`Self::record`].
	pub stem: String,

	/// C++ field identifiers in ABI slot order.
	pub field_names: Vec<String>,
	pub source: String,
	pub layout: VtableLayout,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct VtableSlot {
	pub kind: VtableSlotKind,
	pub method: VirtualMethod,

	/// Position of the introducing declaration among the same-named virtual
	/// functions of the hierarchy, in declaration order. It names the slot
	/// identically under every ABI, even where the slot orders differ.
	pub overload_index: usize,
	lineage: BTreeSet<String>,
}

impl VtableSlot {
	fn method(method: VirtualMethod, overload_index: usize) -> Self {
		let mut slot = Self::new(VtableSlotKind::Method, method);
		slot.overload_index = overload_index;
		slot
	}

	fn new(kind: VtableSlotKind, method: VirtualMethod) -> Self {
		let mut lineage = BTreeSet::from([method.usr.clone()]);
		lineage.extend(
			method
				.overrides
				.iter()
				.map(|overridden| overridden.usr.clone()),
		);

		Self {
			kind,
			method,
			overload_index: 0,
			lineage,
		}
	}
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum VtableSlotKind {
	Method,
	ItaniumCompleteDestructor,
	ItaniumDeletingDestructor,
	MsvcScalarDeletingDestructor,
}

/// Applies one record's own virtual functions to its primary base's slots.
///
/// Overriders replace their slot in place. New virtual functions are appended
/// in declaration order under the Itanium ABI. The Microsoft ABI instead
/// groups them by the first member declaration of their name and emits each
/// group in reverse declaration order, which reorders overload sets.
///
/// Returns why the table cannot be modeled, if it cannot.
fn apply_record_methods(
	slots: &mut Vec<VtableSlot>,
	record: &Record,
	abi: CppAbi,
) -> Result<Option<String>, CppVtableError> {
	let mut introduced = Vec::new();

	for (declaration_index, method) in record.virtual_methods.iter().enumerate() {
		if method.kind == VirtualMethodKind::Destructor {
			let destructor_slots = slots
				.iter()
				.enumerate()
				.filter_map(|(index, slot)| {
					(slot.method.kind == VirtualMethodKind::Destructor).then_some(index)
				})
				.collect::<Vec<_>>();

			if destructor_slots.is_empty() {
				introduced.push(declaration_index);
			} else {
				for index in destructor_slots {
					replace_slot_method(&mut slots[index], method);
				}
			}

			continue;
		}

		let overridden_usrs = method
			.overrides
			.iter()
			.map(|overridden| overridden.usr.as_str())
			.collect::<BTreeSet<_>>();
		if overridden_usrs.is_empty() {
			introduced.push(declaration_index);
			continue;
		}

		let matching_slots = slots
			.iter()
			.enumerate()
			.filter_map(|(index, slot)| {
				slot.lineage
					.iter()
					.any(|usr| overridden_usrs.contains(usr.as_str()))
					.then_some(index)
			})
			.collect::<Vec<_>>();

		match matching_slots.as_slice() {
			[] => {
				return Err(CppVtableError::MissingOverrideSlot {
					record: record.qualified_name.clone(),
					method: method.display_name.clone(),
				});
			}

			[index] => {
				// A covariant overrider may need a return-adjusting slot of its
				// own. Both ABIs decide that from base offsets, which libclang
				// does not expose.
				if slots[*index].method.canonical_result_type != method.canonical_result_type {
					return Ok(Some(format!(
						"`{}` overrides `{}` with a covariant return type",
						method.qualified_name, slots[*index].method.qualified_name
					)));
				}

				replace_slot_method(&mut slots[*index], method);
			}

			_ => {
				return Err(CppVtableError::AmbiguousOverrideSlots {
					record: record.qualified_name.clone(),
					method: method.display_name.clone(),
					count: matching_slots.len(),
				});
			}
		}
	}

	let mut name_counts = BTreeMap::<String, usize>::new();

	for slot in slots
		.iter()
		.filter(|slot| slot.kind == VtableSlotKind::Method)
	{
		*name_counts
			.entry(sanitize_identifier_fragment(&slot.method.name))
			.or_default() += 1;
	}

	let overload_indices = introduced
		.iter()
		.filter(|&&declaration_index| {
			record.virtual_methods[declaration_index].kind == VirtualMethodKind::Method
		})
		.map(|&declaration_index| {
			let count = name_counts
				.entry(sanitize_identifier_fragment(
					&record.virtual_methods[declaration_index].name,
				))
				.or_default();
			let overload_index = *count;
			*count += 1;
			(declaration_index, overload_index)
		})
		.collect::<BTreeMap<_, _>>();

	if abi.is_microsoft() {
		introduced.sort_by_key(|&declaration_index| {
			let name = &record.virtual_methods[declaration_index].name;
			let group = record
				.member_names
				.iter()
				.position(|member| member == name)
				.unwrap_or(usize::MAX);
			(group, Reverse(declaration_index))
		});
	}

	for declaration_index in introduced {
		let method = &record.virtual_methods[declaration_index];

		match method.kind {
			VirtualMethodKind::Destructor => slots.extend(new_destructor_slots(method, abi)),
			VirtualMethodKind::Method => slots.push(VtableSlot::method(
				method.clone(),
				overload_indices[&declaration_index],
			)),
		}
	}

	Ok(None)
}

extern "C" fn collect_record_child_visitor(
	cursor: CXCursor,
	_parent: CXCursor,
	data: CXClientData,
) -> CXChildVisitResult {
	let Some(children) = (unsafe { data.cast::<RecordChildren>().as_mut() }) else {
		return CXChildVisit_Break;
	};

	if children.error.is_some() {
		return CXChildVisit_Break;
	}

	let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
		let cursor = Cursor(cursor);
		let kind = cursor.kind();

		match kind {
			CXCursor_CXXBaseSpecifier => {
				children.has_members = true;
				children.bases.push(cursor.collect_base());
			}
			CXCursor_CXXMethod if cursor.is_virtual_method() => {
				children
					.virtual_methods
					.push(cursor.collect_virtual_method(VirtualMethodKind::Method)?);
			}
			CXCursor_Destructor if cursor.is_virtual_method() => {
				children
					.virtual_methods
					.push(cursor.collect_virtual_method(VirtualMethodKind::Destructor)?);
			}
			_ => {}
		}

		if is_named_member_kind(kind) {
			children.has_members = true;

			if !cursor.is_anonymous() {
				let name = cursor.spelling();

				if !name.is_empty() && !children.member_names.contains(&name) {
					children.member_names.push(name);
				}
			}
		}

		Ok::<_, CppVtableError>(())
	}));

	match result {
		Ok(Ok(())) => CXChildVisit_Continue,
		Ok(Err(error)) => {
			children.error = Some(error);
			CXChildVisit_Break
		}
		Err(payload) => {
			children.error = Some(CppVtableError::TraversalPanic(panic_message(payload)));
			CXChildVisit_Break
		}
	}
}

/// Member declarations which introduce a name into their class, the way
/// Clang's `NamedDecl` members order the Microsoft ABI's overload groups.
fn is_named_member_kind(kind: CXCursorKind) -> bool {
	matches!(
		kind,
		CXCursor_CXXMethod
			| CXCursor_Constructor
			| CXCursor_Destructor
			| CXCursor_ConversionFunction
			| CXCursor_FunctionTemplate
			| CXCursor_FieldDecl
			| CXCursor_VarDecl
			| CXCursor_TypedefDecl
			| CXCursor_TypeAliasDecl
			| CXCursor_TypeAliasTemplateDecl
			| CXCursor_ClassDecl
			| CXCursor_StructDecl
			| CXCursor_UnionDecl
			| CXCursor_EnumDecl
			| CXCursor_ClassTemplate
			| CXCursor_ClassTemplatePartialSpecialization
			| CXCursor_UsingDeclaration
	)
}

extern "C" fn collect_record_visitor(
	cursor: CXCursor,
	_parent: CXCursor,
	data: CXClientData,
) -> CXChildVisitResult {
	let Some(collector) = (unsafe { data.cast::<Collector>().as_mut() }) else {
		return CXChildVisit_Break;
	};

	if collector.error.is_some() {
		return CXChildVisit_Break;
	}

	let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
		let cursor = Cursor(cursor);

		// Clang's USR marks an unnamed record named by a typedef, for example
		// `c:@UA@pthread_mutex_t`, whichever header declares it.
		if matches!(
			cursor.kind(),
			CXCursor_ClassDecl | CXCursor_StructDecl | CXCursor_UnionDecl
		) && cursor.is_definition()
		{
			let usr = cursor.usr();
			let name = cursor.spelling();

			if ["@SA@", "@UA@"]
				.iter()
				.any(|marker| usr.ends_with(&format!("{marker}{name}")))
			{
				collector.typedef_named_anonymous_records.insert(name);
			}
		}

		let is_record = matches!(
			cursor.kind(),
			CXCursor_ClassDecl
				| CXCursor_StructDecl
				| CXCursor_ClassTemplate
				| CXCursor_ClassTemplatePartialSpecialization
		);

		// Anonymous records cannot be named by a probe, and bindgen names them
		// `__bindgen_ty_N`, so they never correspond to a generated vtable.
		if is_record && cursor.is_definition() && !cursor.is_anonymous() {
			let source_path = cursor.source_path();
			if !collector.includes_source_path(source_path.as_deref()) {
				return Ok(());
			}

			let record = cursor.collect_record(source_path)?;
			if let Some(record) = record {
				if let Some(existing) = collector.records.get(&record.qualified_name) {
					// A definition can be reached through more than one AST path. Its
					// USR remains stable; distinct definitions which merely collapse to
					// the same display name must still be diagnosed when they affect a
					// vtable. Repeated layout-only helper names are harmless because this
					// index deliberately does not model data members.
					let both_layout_only = existing.bases.is_empty()
						&& existing.virtual_methods.is_empty()
						&& record.bases.is_empty()
						&& record.virtual_methods.is_empty();
					if !both_layout_only
						&& (existing.usr != record.usr
							|| existing.bases != record.bases
							|| existing.virtual_methods != record.virtual_methods)
					{
						return Err(CppVtableError::DuplicateRecord(record.qualified_name));
					}
				} else {
					collector
						.records
						.insert(record.qualified_name.clone(), record);
				}
			}
		}

		Ok::<_, CppVtableError>(())
	}));

	match result {
		Ok(Ok(())) => CXChildVisit_Recurse,
		Ok(Err(error)) => {
			collector.error = Some(error);
			CXChildVisit_Break
		}
		Err(payload) => {
			collector.error = Some(CppVtableError::TraversalPanic(panic_message(payload)));
			CXChildVisit_Break
		}
	}
}

/// Parse an unsaved C++ source file and collect its complete record definitions.
///
/// `clang_args` are passed directly to libclang. Callers should include their
/// language and target arguments (for example `-x`, `c++`, and `-std=c++17`).
pub(crate) fn collect_records(
	file_name: impl AsRef<Path>,
	contents: &str,
	clang_args: &[String],
) -> Result<RecordIndex, CppVtableError> {
	collect_records_with_source_root(file_name.as_ref(), contents, clang_args, None)
}

/// Parse an unsaved C++ source file and collect complete record definitions
/// whose definition locations are beneath `source_root`.
///
/// This is the production-facing variant: compiler and standard-library
/// headers may expose several implementation records with the same apparent
/// qualified name, while only records owned by the source SDK are candidates
/// for generated vtables.
pub(crate) fn collect_records_under_root(
	file_name: impl AsRef<Path>,
	contents: &str,
	clang_args: &[String],
	source_root: impl AsRef<Path>,
) -> Result<RecordIndex, CppVtableError> {
	collect_records_with_source_root(
		file_name.as_ref(),
		contents,
		clang_args,
		Some(source_root.as_ref()),
	)
}

fn collect_records_with_source_root(
	file_name: &Path,
	contents: &str,
	clang_args: &[String],
	source_root: Option<&Path>,
) -> Result<RecordIndex, CppVtableError> {
	ensure_libclang_is_loaded()?;

	let file_name_string = file_name.to_string_lossy().into_owned();
	let file_name_c = to_c_string(&file_name_string, "unsaved source filename")?;
	let contents_c = to_c_string(contents, "unsaved C++ source")?;
	let argument_strings = clang_args
		.iter()
		.enumerate()
		.map(|(index, argument)| to_c_string(argument, &format!("clang argument {index}")))
		.collect::<Result<Vec<_>, _>>()?;
	let argument_pointers = argument_strings
		.iter()
		.map(|argument| argument.as_ptr())
		.collect::<Vec<_>>();

	let index = unsafe { Index::new() };
	let mut unsaved = CXUnsavedFile {
		Filename: file_name_c.as_ptr(),
		Contents: contents_c.as_ptr(),
		Length: contents.len() as _,
	};
	let mut translation_unit = null_mut();
	let flags = CXTranslationUnit_SkipFunctionBodies | CXTranslationUnit_KeepGoing;
	let error = unsafe {
		clang_parseTranslationUnit2(
			index.0,
			file_name_c.as_ptr(),
			argument_pointers.as_ptr(),
			argument_pointers.len() as _,
			&raw mut unsaved,
			1,
			flags,
			&raw mut translation_unit,
		)
	};

	if error != CXError_Success || translation_unit.is_null() {
		return Err(CppVtableError::ParseTranslationUnit {
			path: file_name.to_path_buf(),
			code: error,
		});
	}

	let translation_unit = TranslationUnit(translation_unit);
	let diagnostics = unsafe { translation_unit.error_diagnostics() };
	if !diagnostics.is_empty() {
		return Err(CppVtableError::Diagnostics {
			path: file_name.to_path_buf(),
			diagnostics: diagnostics.join("\n"),
		});
	}

	let mut collector = Collector::new(source_root);
	let cursor = unsafe { translation_unit.cursor() };
	unsafe {
		clang_visitChildren(
			cursor.0,
			collect_record_visitor,
			(&raw mut collector).cast(),
		);
	}

	if let Some(error) = collector.error {
		return Err(error);
	}

	Ok(RecordIndex {
		records: collector.records,
		typedef_named_anonymous_records: collector.typedef_named_anonymous_records,
	})
}

fn comparable_path_components(path: &Path) -> Vec<String> {
	let path = lexically_absolute_path(path);
	path.components()
		.map(|component| {
			let component = component.as_os_str().to_string_lossy();
			if cfg!(windows) {
				component.to_lowercase()
			} else {
				component.into_owned()
			}
		})
		.collect()
}

unsafe fn cx_string(value: CXString) -> String {
	let pointer = unsafe { clang_getCString(value) };
	let string = if pointer.is_null() {
		String::new()
	} else {
		unsafe { CStr::from_ptr(pointer) }
			.to_string_lossy()
			.into_owned()
	};
	unsafe { clang_disposeString(value) };
	string
}

fn ensure_libclang_is_loaded() -> Result<(), CppVtableError> {
	if is_loaded() {
		return Ok(());
	}

	static LIBCLANG: OnceLock<Result<Arc<SharedLibrary>, String>> = OnceLock::new();

	let library = LIBCLANG.get_or_init(|| load_manually().map(Arc::new));

	match library {
		Ok(library) => {
			set_library(Some(library.clone()));
			Ok(())
		}

		Err(error) => Err(CppVtableError::LoadLibclang(error.clone())),
	}
}

fn is_cpp_identifier_character(character: char) -> bool {
	character == '_' || character.is_alphanumeric()
}

fn lexically_absolute_path(path: &Path) -> PathBuf {
	let absolute = if path.is_absolute() {
		path.to_path_buf()
	} else if let Ok(current_dir) = std::env::current_dir() {
		current_dir.join(path)
	} else {
		path.to_path_buf()
	};

	let mut normalized = PathBuf::new();

	for component in absolute.components() {
		match component {
			std::path::Component::CurDir => {}
			std::path::Component::ParentDir => {
				normalized.pop();
			}
			_ => normalized.push(component.as_os_str()),
		}
	}

	normalized
}

fn new_destructor_slots(method: &VirtualMethod, abi: CppAbi) -> Vec<VtableSlot> {
	match abi {
		CppAbi::Itanium => vec![
			VtableSlot::new(VtableSlotKind::ItaniumCompleteDestructor, method.clone()),
			VtableSlot::new(VtableSlotKind::ItaniumDeletingDestructor, method.clone()),
		],
		CppAbi::Msvc | CppAbi::MsvcX86 => vec![VtableSlot::new(
			VtableSlotKind::MsvcScalarDeletingDestructor,
			method.clone(),
		)],
	}
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
	if let Some(message) = payload.downcast_ref::<&str>() {
		(*message).to_owned()
	} else if let Some(message) = payload.downcast_ref::<String>() {
		message.clone()
	} else {
		"unknown panic payload".to_owned()
	}
}

fn replace_slot_method(slot: &mut VtableSlot, method: &VirtualMethod) {
	slot.lineage.insert(method.usr.clone());
	slot.lineage.extend(
		method
			.overrides
			.iter()
			.map(|overridden| overridden.usr.clone()),
	);
	slot.method = method.clone();
}

fn replace_unqualified_identifier(spelling: &str, name: &str, replacement: &str) -> String {
	let mut rendered = String::with_capacity(spelling.len() + replacement.len());
	let mut remaining = spelling;

	while let Some(index) = remaining.find(name) {
		let before = &remaining[..index];
		let after_index = index + name.len();
		let after = &remaining[after_index..];
		let identifier_before = before
			.chars()
			.next_back()
			.is_some_and(is_cpp_identifier_character);
		let identifier_after = after
			.chars()
			.next()
			.is_some_and(is_cpp_identifier_character);
		let already_qualified = before.ends_with("::");

		rendered.push_str(before);
		if identifier_before || identifier_after || already_qualified {
			rendered.push_str(name);
		} else {
			rendered.push_str(replacement);
		}
		remaining = after;
	}

	rendered.push_str(remaining);
	rendered
}

/// Renders a typed synthetic vtable probe for one modeled layout.
pub(crate) fn render_vtable_probe(
	records: &RecordIndex,
	layout: &VtableLayout,
) -> Result<VtableProbe, CppVtableError> {
	let qualified_record_name = layout.record.as_str();
	let abi = layout.abi;
	let record = records
		.record(qualified_record_name)
		.ok_or_else(|| CppVtableError::MissingRecord(qualified_record_name.to_owned()))?;
	let stem = vtable_probe_stem(qualified_record_name);
	let record_identifier = stem.strip_prefix("__crys_vtable_").unwrap_or(&stem);
	let this_type = if qualified_record_name.starts_with("::") {
		qualified_record_name.to_owned()
	} else {
		format!("::{qualified_record_name}")
	};
	let mut aliases = String::new();
	let mut fields = String::new();
	let mut field_names = Vec::with_capacity(layout.slots.len());

	for (index, slot) in layout.slots.iter().enumerate() {
		let alias = format!("{stem}_slot_{index:03}_type");
		let field = match slot.kind {
			VtableSlotKind::Method => {
				let method_name = sanitize_identifier_fragment(&slot.method.name);
				let suffix = if slot.overload_index == 0 {
					String::new()
				} else {
					slot.overload_index.to_string()
				};
				format!("{record_identifier}_{method_name}{suffix}")
			}
			VtableSlotKind::ItaniumCompleteDestructor => {
				format!("{record_identifier}_complete_destructor")
			}
			VtableSlotKind::ItaniumDeletingDestructor => {
				format!("{record_identifier}_deleting_destructor")
			}
			VtableSlotKind::MsvcScalarDeletingDestructor => {
				format!("{record_identifier}_destructor")
			}
		};
		let (result_type, mut parameters) = match slot.kind {
			VtableSlotKind::Method => {
				let this = if slot.method.is_const {
					format!("const {this_type} *")
				} else {
					format!("{this_type} *")
				};
				let mut parameters = vec![this];
				parameters.extend(
					slot.method
						.parameters
						.iter()
						.map(|parameter| parameter.type_spelling.clone()),
				);
				(slot.method.result_type.clone(), parameters)
			}
			VtableSlotKind::ItaniumCompleteDestructor
			| VtableSlotKind::ItaniumDeletingDestructor => {
				("void".to_owned(), vec![format!("{this_type} *")])
			}
			VtableSlotKind::MsvcScalarDeletingDestructor => (
				"void *".to_owned(),
				vec![format!("{this_type} *"), "unsigned int".to_owned()],
			),
		};

		if slot.kind == VtableSlotKind::Method && slot.method.is_variadic {
			parameters.push("...".to_owned());
		}

		let calling_convention = match abi {
			// MSVC variadic member functions use cdecl so the caller can clean
			// the variable argument list; ordinary x86 member calls use thiscall.
			CppAbi::MsvcX86 if slot.kind == VtableSlotKind::Method && slot.method.is_variadic => {
				"__cdecl "
			}
			CppAbi::MsvcX86 => "__thiscall ",
			CppAbi::Itanium | CppAbi::Msvc => "",
		};
		let _ = writeln!(
			aliases,
			"using {alias} = auto ({calling_convention}*)({}) -> {result_type};",
			parameters.join(", ")
		);
		let _ = writeln!(fields, "    {alias} {field};");
		field_names.push(field);
	}

	let namespace_context = if record.namespaces.is_empty() {
		String::new()
	} else {
		format!("using namespace ::{};\n\n", record.namespaces.join("::"))
	};
	let source = format!("{namespace_context}{aliases}\nstruct {stem} {{\n{fields}}};\n");

	Ok(VtableProbe {
		record: qualified_record_name.to_owned(),
		stem,
		field_names,
		source,
		layout: layout.clone(),
	})
}

fn sanitize_identifier_fragment(value: &str) -> String {
	let mut sanitized = String::new();
	for character in value.chars() {
		if character.is_ascii_alphanumeric() || character == '_' {
			sanitized.push(character);
		} else {
			sanitized.push('_');
		}
	}

	if sanitized.is_empty() {
		"unnamed".to_owned()
	} else {
		sanitized
	}
}

fn to_c_string(value: &str, context: &str) -> Result<CString, CppVtableError> {
	CString::new(value).map_err(|_| CppVtableError::InteriorNul {
		context: context.to_owned(),
	})
}

/// Maps a qualified C++ record name to the deterministic probe identifier stem.
pub(crate) fn vtable_probe_stem(qualified_record_name: &str) -> String {
	let mut sanitized = String::new();
	let mut chars = qualified_record_name.chars().peekable();

	while let Some(character) = chars.next() {
		if character == ':' && chars.peek() == Some(&':') {
			chars.next();
			sanitized.push_str("__");
		} else if character.is_ascii_alphanumeric() || character == '_' {
			sanitized.push(character);
		} else {
			let _ = write!(sanitized, "_u{:x}_", character as u32);
		}
	}

	if sanitized.is_empty() {
		sanitized.push_str("anonymous");
	}

	format!("__crys_vtable_{sanitized}")
}
#[cfg(test)]
mod tests {
	use super::*;
	use std::fs;
	use std::sync::atomic::{AtomicUsize, Ordering};

	static TEMP_FIXTURE_ID: AtomicUsize = AtomicUsize::new(0);

	struct TempFixture(PathBuf);

	impl TempFixture {
		fn new() -> Self {
			let id = TEMP_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
			let path = std::env::temp_dir().join(format!(
				"source-sdk-vtable-test-{}-{id}",
				std::process::id()
			));
			fs::create_dir_all(&path).unwrap();
			Self(path)
		}
	}

	impl Drop for TempFixture {
		fn drop(&mut self) {
			let _ = fs::remove_dir_all(&self.0);
		}
	}

	fn records(source: &str) -> RecordIndex {
		collect_records(
			"cpp_vtable_fixture.cpp",
			source,
			&["-x".to_owned(), "c++".to_owned(), "-std=c++17".to_owned()],
		)
		.unwrap()
	}

	fn probe(records: &RecordIndex, record: &str, abi: CppAbi) -> VtableProbe {
		render_vtable_probe(records, &records.vtable_layout(record, abi).unwrap()).unwrap()
	}

	fn slot_names(records: &RecordIndex, record: &str, abi: CppAbi) -> Vec<String> {
		records
			.vtable_layout(record, abi)
			.unwrap()
			.slots
			.iter()
			.map(|slot| match slot.kind {
				VtableSlotKind::Method => slot.method.display_name.clone(),
				VtableSlotKind::ItaniumCompleteDestructor => "~complete".to_owned(),
				VtableSlotKind::ItaniumDeletingDestructor => "~deleting".to_owned(),
				VtableSlotKind::MsvcScalarDeletingDestructor => "~scalar".to_owned(),
			})
			.collect()
	}

	#[test]
	fn source_root_filter_excludes_external_records_but_keeps_sdk_conflicts_visible() {
		let fixture = TempFixture::new();
		let sdk_root = fixture.0.join("sdk").join("src");
		let external_root = fixture.0.join("toolchain");
		fs::create_dir_all(&sdk_root).unwrap();
		fs::create_dir_all(&external_root).unwrap();
		fs::write(
			external_root.join("external.hpp"),
			r#"
			template <typename T> struct Specialized {};
			template <> struct Specialized<int> { virtual void one() = 0; };
			template <> struct Specialized<double> { virtual int two() = 0; };
			struct ExternalInterface { virtual void ignored() = 0; };
			"#,
		)
		.unwrap();

		let file_name = sdk_root.join("fixture.cpp");
		let clang_args = vec![
			"-x".to_owned(),
			"c++".to_owned(),
			"-std=c++17".to_owned(),
			format!("-I{}", external_root.display()),
		];
		let source = r#"
			#include "external.hpp"
			struct SdkInterface { virtual void kept() = 0; };
			"#;

		let unfiltered = collect_records(&file_name, source, &clang_args).unwrap();
		assert_eq!(
			unfiltered.record("Specialized<int>").unwrap().template,
			TemplateRole::ExplicitSpecialization
		);
		assert_eq!(
			unfiltered.record("Specialized<double>").unwrap().template,
			TemplateRole::ExplicitSpecialization
		);
		assert!(unfiltered.record("ExternalInterface").is_some());

		let filtered =
			collect_records_under_root(&file_name, source, &clang_args, &sdk_root).unwrap();
		assert!(filtered.record("SdkInterface").is_some());
		assert!(filtered.record("ExternalInterface").is_none());
		assert!(filtered.record("Specialized").is_none());
		assert!(filtered.record("Specialized<int>").is_none());

		// Declarations in an anonymous namespace share the enclosing scope's
		// names, so these genuinely conflicting definitions still collide.
		let conflicting_sdk = r#"
			namespace { struct Collision { virtual void one() = 0; }; }
			struct Collision { virtual int two() = 0; };
			"#;
		assert!(matches!(
			collect_records_under_root(&file_name, conflicting_sdk, &clang_args, &sdk_root),
			Err(CppVtableError::DuplicateRecord(ref name)) if name == "Collision"
		));
	}

	#[test]
	fn source_root_filter_respects_component_boundaries() {
		let fixture = TempFixture::new();
		let root = fixture.0.join("SDK").join("Src");
		let mut filter = SourceRootFilter::new(&root);
		assert!(filter.includes(&root.join("public").join("iface.hpp")));
		assert!(!filter.includes(&fixture.0.join("SDK").join("SrcOther").join("iface.hpp")));

		#[cfg(windows)]
		assert!(
			filter.includes(
				&fixture
					.0
					.join("sdk")
					.join("src")
					.join("PUBLIC")
					.join("iface.hpp")
			)
		);
	}

	#[test]
	fn collects_inline_virtual_method_metadata() {
		let records = records(
			r#"
			namespace fixture {
			class Inline {
			public:
				virtual int value(double amount, ...) const { return static_cast<int>(amount); }
			};
			}
			"#,
		);
		let record = records.record("fixture::Inline").unwrap();
		assert_eq!(record.name, "Inline");
		assert!(
			record
				.source_path
				.as_ref()
				.unwrap()
				.ends_with("cpp_vtable_fixture.cpp")
		);
		assert!(record.bases.is_empty());
		assert_eq!(record.virtual_methods.len(), 1);

		let method = &record.virtual_methods[0];
		assert_eq!(method.name, "value");
		assert_eq!(method.qualified_name, "fixture::Inline::value");
		assert_eq!(method.kind, VirtualMethodKind::Method);
		assert!(method.is_const);
		assert_eq!(method.result_type, "int");
		assert_eq!(
			method.parameters,
			[Parameter {
				name: "amount".to_owned(),
				type_spelling: "double".to_owned()
			}]
		);
		assert!(method.is_variadic);
		assert!(method.overrides.is_empty());
	}

	#[test]
	fn expands_destructor_at_its_declaration_position() {
		let records = records(
			r#"
			struct Ordered {
				virtual void first() = 0;
				virtual ~Ordered() {}
				virtual int last(char value) = 0;
			};
			"#,
		);
		let record = records.record("Ordered").unwrap();
		assert_eq!(
			record
				.virtual_methods
				.iter()
				.map(|method| method.name.as_str())
				.collect::<Vec<_>>(),
			["first", "~Ordered", "last"]
		);

		let itanium = records.vtable_layout("Ordered", CppAbi::Itanium).unwrap();
		assert_eq!(
			itanium
				.slots
				.iter()
				.map(|slot| slot.kind)
				.collect::<Vec<_>>(),
			[
				VtableSlotKind::Method,
				VtableSlotKind::ItaniumCompleteDestructor,
				VtableSlotKind::ItaniumDeletingDestructor,
				VtableSlotKind::Method,
			]
		);
		assert_eq!(itanium.slots[3].method.name, "last");

		let msvc = records.vtable_layout("Ordered", CppAbi::Msvc).unwrap();
		assert_eq!(
			msvc.slots.iter().map(|slot| slot.kind).collect::<Vec<_>>(),
			[
				VtableSlotKind::Method,
				VtableSlotKind::MsvcScalarDeletingDestructor,
				VtableSlotKind::Method,
			]
		);
	}

	#[test]
	fn replaces_primary_base_slot_for_override_and_appends_new_virtual() {
		let records = records(
			r#"
			namespace fixture {
			struct Base {
				virtual int keep(double value) = 0;
				virtual int replace(int value) = 0;
			};
			struct Derived : Base {
				int replace(int value) override { return value; }
				virtual void added() = 0;
			};
			}
			"#,
		);
		let derived = records.record("fixture::Derived").unwrap();
		assert_eq!(
			derived.bases,
			[BaseRecord {
				qualified_name: "fixture::Base".to_owned(),
				type_spelling: "Base".to_owned(),
				is_virtual: false,
				target: BaseTarget::Record,
			}]
		);
		assert_eq!(derived.virtual_methods[0].name, "replace");
		assert_eq!(derived.virtual_methods[0].overrides.len(), 1);
		assert_eq!(
			derived.virtual_methods[0].overrides[0].qualified_name,
			"fixture::Base::replace"
		);

		let layout = records
			.vtable_layout("fixture::Derived", CppAbi::Itanium)
			.unwrap();
		assert_eq!(
			layout
				.slots
				.iter()
				.map(|slot| slot.method.qualified_name.as_str())
				.collect::<Vec<_>>(),
			[
				"fixture::Base::keep",
				"fixture::Derived::replace",
				"fixture::Derived::added"
			]
		);

		let probe = probe(&records, "fixture::Derived", CppAbi::Itanium);
		assert_eq!(probe.stem, "__crys_vtable_fixture__Derived");
		assert!(probe.source.contains("::fixture::Derived *"));
		assert!(probe.source.contains("fixture__Derived_keep"));
		assert!(probe.source.contains("fixture__Derived_replace"));
		assert!(probe.source.contains("fixture__Derived_added"));
	}

	#[test]
	fn preserves_overloaded_virtuals_as_distinct_slots() {
		let records = records(
			r#"
			struct Overloaded {
				virtual void call(int value) = 0;
				virtual void call(double value) = 0;
			};
			"#,
		);
		let layout = records
			.vtable_layout("Overloaded", CppAbi::Itanium)
			.unwrap();
		assert_eq!(layout.slots.len(), 2);
		assert_eq!(layout.slots[0].method.parameters[0].type_spelling, "int");
		assert_eq!(layout.slots[1].method.parameters[0].type_spelling, "double");

		let probe = probe(&records, "Overloaded", CppAbi::Itanium);
		assert!(
			probe
				.source
				.contains("__crys_vtable_Overloaded_slot_000_type")
		);
		assert!(
			probe
				.source
				.contains("__crys_vtable_Overloaded_slot_001_type")
		);
		assert!(probe.source.contains("Overloaded_call;"));
		assert!(probe.source.contains("Overloaded_call1;"));
	}

	#[test]
	fn rendered_probe_keeps_user_type_spellings_valid_at_global_scope() {
		let fixture = r#"
			namespace fixture {
			struct Argument {};
			struct Typed {
				virtual Argument * exchange(const Argument & value) = 0;
			};
			}
			"#;
		let index = records(fixture);
		let method = &index.record("fixture::Typed").unwrap().virtual_methods[0];
		assert_eq!(method.result_type, "::fixture::Argument *");
		assert_eq!(
			method.parameters[0].type_spelling,
			"const ::fixture::Argument &"
		);

		let probe = probe(&index, "fixture::Typed", CppAbi::Itanium);
		assert!(probe.source.starts_with("using namespace ::fixture;"));
		let _ = records(&format!("{fixture}\n{}", probe.source));
	}

	#[test]
	fn globally_qualifies_nested_result_and_parameter_types() {
		let fixture = r#"
			struct Outer {
				enum Result { Found, Missing };
				using ResultAlias = Result;
				virtual Result find(ResultAlias previous, const Result *fallback) = 0;
			};
			"#;
		let index = records(fixture);
		let method = &index.record("Outer").unwrap().virtual_methods[0];
		assert_eq!(method.result_type, "::Outer::Result");
		assert_eq!(method.parameters[0].type_spelling, "::Outer::ResultAlias");
		assert_eq!(
			method.parameters[1].type_spelling,
			"const ::Outer::Result *"
		);

		let probe = probe(&index, "Outer", CppAbi::Itanium);
		let _ = records(&format!("{fixture}\n{}", probe.source));
	}

	#[test]
	fn bindgen_can_consume_the_rendered_aliases_and_struct() {
		let fixture = r#"
			struct ProbeTarget {
				virtual int call(double value) const = 0;
			};
			"#;
		let index = records(fixture);
		let probe = probe(&index, "ProbeTarget", CppAbi::Itanium);
		let source = format!("{fixture}\n{}", probe.source);
		let bindings = bindgen::builder()
			.header_contents("cpp_vtable_bindgen_fixture.hpp", &source)
			.clang_args(["-x", "c++", "-std=c++17"])
			.allowlist_type("__crys_vtable_ProbeTarget")
			.generate()
			.unwrap()
			.to_string();

		assert!(bindings.contains("pub type __crys_vtable_ProbeTarget_slot_000_type"));
		assert!(bindings.contains("pub struct __crys_vtable_ProbeTarget"));
		assert!(bindings.contains("pub ProbeTarget_call:"));
	}

	#[test]
	fn renders_typed_destructor_slots_for_both_abis() {
		let fixture = r#"
			struct Destroyed {
				virtual ~Destroyed() = default;
			};
			"#;
		let index = records(fixture);
		assert_eq!(vtable_probe_stem("Destroyed"), "__crys_vtable_Destroyed");

		let itanium = probe(&index, "Destroyed", CppAbi::Itanium);
		assert!(itanium.source.contains(
			"using __crys_vtable_Destroyed_slot_000_type = auto (*)(::Destroyed *) -> void;"
		));
		assert!(itanium.source.contains("Destroyed_complete_destructor"));
		assert!(itanium.source.contains("Destroyed_deleting_destructor"));

		let msvc = probe(&index, "Destroyed", CppAbi::Msvc);
		assert!(msvc.source.contains(
			"using __crys_vtable_Destroyed_slot_000_type = auto (*)(::Destroyed *, unsigned int) -> void *;"
		));
		assert!(msvc.source.contains("Destroyed_destructor"));

		// The probe is C++ source, rather than only a diagnostic description.
		// Have libclang parse both renderings to guard their function-pointer syntax.
		let _ = records(&format!("{fixture}\n{}", itanium.source));
		let _ = records(&format!("{fixture}\n{}", msvc.source));
	}

	#[test]
	fn renders_msvc_x86_slots_with_thiscall() {
		let fixture = r#"
			struct Win32Interface {
				virtual int call(double value) const = 0;
				virtual void log(const char *message, ...) = 0;
				virtual ~Win32Interface() = default;
			};
			"#;
		let index = records(fixture);
		let probe = probe(&index, "Win32Interface", CppAbi::MsvcX86);

		assert!(
			probe
				.source
				.contains("auto (__thiscall *)(const ::Win32Interface *, double) -> int;")
		);
		assert!(
			probe
				.source
				.contains("auto (__cdecl *)(::Win32Interface *, const char *, ...) -> void;")
		);
		assert!(
			probe
				.source
				.contains("auto (__thiscall *)(::Win32Interface *, unsigned int) -> void *;")
		);
		assert_eq!(
			probe.field_names,
			[
				"Win32Interface_call",
				"Win32Interface_log",
				"Win32Interface_destructor"
			]
		);

		let source = format!("{fixture}\n{}", probe.source);
		let bindings = bindgen::builder()
			.header_contents("cpp_vtable_win32_fixture.hpp", &source)
			.clang_args([
				"-x",
				"c++",
				"-std=c++17",
				"--target=i686-pc-windows-msvc",
				"-fms-extensions",
			])
			.allowlist_type("__crys_vtable_Win32Interface")
			.generate()
			.unwrap()
			.to_string();

		assert!(
			bindings.contains("extern \"thiscall\" fn"),
			"generated bindings:\n{bindings}"
		);
		assert!(
			bindings.contains("extern \"C\" fn"),
			"generated bindings:\n{bindings}"
		);
	}

	#[test]
	fn derived_destructor_replaces_the_inherited_destructor_group() {
		let index = records(
			r#"
			struct Base {
				virtual ~Base() = default;
			};
			struct Derived : Base {
				~Derived() override = default;
			};
			"#,
		);
		let derived = index.record("Derived").unwrap();
		assert_eq!(derived.virtual_methods.len(), 1);
		assert_eq!(
			derived.virtual_methods[0].kind,
			VirtualMethodKind::Destructor
		);

		let layout = index.vtable_layout("Derived", CppAbi::Itanium).unwrap();
		assert_eq!(layout.slots.len(), 2);
		assert!(
			layout
				.slots
				.iter()
				.all(|slot| slot.method.qualified_name == "Derived::~Derived")
		);
	}

	#[test]
	fn distinguishes_nonpolymorphic_inheritance_from_inherited_virtual_slots() {
		let records = records(
			r#"
			struct PlainLeft {};
			struct PlainRight {};
			struct PlainMultiple : PlainLeft, PlainRight {};
			struct PlainVirtual : virtual PlainLeft {};
			struct PlainChain : PlainMultiple {};
			struct Polymorphic { virtual void call() = 0; };
			struct InheritedPolymorphic : PlainLeft, Polymorphic {};
			"#,
		);

		for record in [
			"PlainLeft",
			"PlainRight",
			"PlainMultiple",
			"PlainVirtual",
			"PlainChain",
		] {
			assert!(
				!records.has_virtual_slots(record).unwrap(),
				"{record} unexpectedly has virtual slots"
			);
		}
		assert!(records.has_virtual_slots("Polymorphic").unwrap());
		assert!(records.has_virtual_slots("InheritedPolymorphic").unwrap());
	}

	#[test]
	fn reports_multiple_and_virtual_inheritance_as_unsupported() {
		let records = records(
			r#"
			struct Left { virtual void left() = 0; };
			struct Right { virtual void right() = 0; };
			struct Multiple : Left, Right {};
			struct Virtual : virtual Left {};
			struct PlainBase { int value; };
			struct DataFirst : PlainBase, Left { virtual void own() = 0; };
			"#,
		);

		for record in ["Multiple", "Virtual"] {
			for abi in [CppAbi::Itanium, CppAbi::Msvc] {
				assert!(
					matches!(
						records.vtable_model(record, abi).unwrap(),
						VtableModel::Unsupported { .. }
					),
					"{record} {abi:?}"
				);
			}
		}

		let multiple = records
			.vtable_layout("Multiple", CppAbi::Itanium)
			.unwrap_err();
		assert!(
			matches!(multiple, CppVtableError::Unsupported { ref reason, .. } if reason.contains("more than one polymorphic base"))
		);

		let virtual_base = records
			.vtable_layout("Virtual", CppAbi::Itanium)
			.unwrap_err();
		assert!(
			matches!(virtual_base, CppVtableError::Unsupported { ref reason, .. } if reason.contains("virtual base"))
		);

		// A single polymorphic base supplies the primary vtable in both ABIs,
		// even after a non-polymorphic base.
		let data_first = records.vtable_layout("DataFirst", CppAbi::Itanium).unwrap();
		assert_eq!(
			data_first
				.slots
				.iter()
				.map(|slot| slot.method.qualified_name.as_str())
				.collect::<Vec<_>>(),
			["Left::left", "DataFirst::own"]
		);
	}
	#[test]
	fn keys_explicit_specializations_by_their_template_arguments() {
		let records = records(
			r#"
			enum fieldtype_t { FIELD_INTEGER, FIELD_VECTOR, FIELD_POSITION_VECTOR };
			class Vector { public: float x, y, z; };
			class CBaseEntityOutput { public: ~CBaseEntityOutput(); protected: int m_Value; };

			template <class Type, fieldtype_t fieldType>
			class CEntityOutputTemplate : public CBaseEntityOutput {
			public:
				void Init(Type value) {}
				class Iterator { int index; };
			};

			template <>
			class CEntityOutputTemplate<class Vector, FIELD_VECTOR> : public CBaseEntityOutput {
			public:
				void Get(Vector &vec) {}
				class Iterator { virtual void next(); };
			};

			template <>
			class CEntityOutputTemplate<class Vector, FIELD_POSITION_VECTOR> : public CBaseEntityOutput {
			public:
				void Get(Vector &vec) {}
			};

			typedef CEntityOutputTemplate<int, FIELD_INTEGER> COutputInt;
			typedef CEntityOutputTemplate<Vector, FIELD_VECTOR> COutputVector;

			template class CEntityOutputTemplate<float, FIELD_INTEGER>;
			"#,
		);

		let roles = [
			("CEntityOutputTemplate", TemplateRole::Pattern),
			(
				"CEntityOutputTemplate::Iterator",
				TemplateRole::DependentMember,
			),
			(
				"CEntityOutputTemplate<class Vector, FIELD_VECTOR>",
				TemplateRole::ExplicitSpecialization,
			),
			(
				"CEntityOutputTemplate<class Vector, FIELD_VECTOR>::Iterator",
				TemplateRole::None,
			),
			(
				"CEntityOutputTemplate<class Vector, FIELD_POSITION_VECTOR>",
				TemplateRole::ExplicitSpecialization,
			),
			(
				"CEntityOutputTemplate<float, FIELD_INTEGER>",
				TemplateRole::ExplicitInstantiation,
			),
			("CBaseEntityOutput", TemplateRole::None),
		];

		for (record, role) in roles {
			assert_eq!(
				records.record(record).map(|record| record.template),
				Some(role),
				"{record}"
			);
		}

		for record in [
			"CEntityOutputTemplate",
			"CEntityOutputTemplate<class Vector, FIELD_VECTOR>",
			"CBaseEntityOutput",
		] {
			assert_eq!(
				records.vtable_model(record, CppAbi::Itanium).unwrap(),
				VtableModel::NotPolymorphic,
				"{record}"
			);
		}

		// A member of an explicit specialization is a concrete record.
		assert_eq!(
			slot_names(
				&records,
				"CEntityOutputTemplate<class Vector, FIELD_VECTOR>::Iterator",
				CppAbi::Msvc
			),
			["next()"]
		);
	}

	#[test]
	fn records_typedef_names_of_unnamed_records() {
		let records = records(
			r#"
			typedef union { char size[40]; long align; } pthread_mutex_t;
			typedef struct { int x; } plain_anon_t;
			typedef struct named_s { int y; } named_t;
			namespace ns { typedef struct { int z; } nested_t; }
			"#,
		);

		assert_eq!(
			records
				.typedef_named_anonymous_records()
				.iter()
				.map(String::as_str)
				.collect::<Vec<_>>(),
			["nested_t", "plain_anon_t", "pthread_mutex_t"]
		);
	}

	#[test]
	fn includes_union_scopes_in_qualified_names() {
		let records = records(
			r#"
			class CSteamID {
				union SteamID_t {
					struct SteamIDComponent_t { unsigned int m_unAccountID; } m_comp;
					unsigned long long m_unAll64Bits;
				} m_steamid;
			};
			"#,
		);

		assert!(
			records
				.record("CSteamID::SteamID_t::SteamIDComponent_t")
				.is_some()
		);
		assert!(records.record("CSteamID::SteamIDComponent_t").is_none());
	}

	#[test]
	fn resolves_bases_through_non_polymorphic_template_instantiations() {
		let records = records(
			r#"
			struct IFace {
				virtual ~IFace() {}
				virtual void run() = 0;
			};

			// Source's `CNetworkVarBase` shape: a non-polymorphic CRTP template.
			template <class Type, class Changer> class NetworkVarBase { protected: Type m_Value; };

			// A template adding data, but no virtual functions, to a concrete base.
			template <class T> class Holder : public IFace { public: T value; };

			class Entity : public IFace {
			public:
				class NetworkVar_health : public NetworkVarBase<int, NetworkVar_health> {
				public:
					virtual void changed();
				};

				void run() override;
				virtual void think();
			};

			class Held : public Holder<int> {
			public:
				void run() override;
				virtual void extra();
			};

			template class Holder<float>;
			class HeldExplicit : public Holder<float> { public: virtual void extra(); };
			"#,
		);

		assert_eq!(
			slot_names(&records, "Entity::NetworkVar_health", CppAbi::Itanium),
			["changed()"]
		);
		assert_eq!(
			slot_names(&records, "Held", CppAbi::Itanium),
			["~complete", "~deleting", "run()", "extra()"]
		);
		assert_eq!(
			slot_names(&records, "Held", CppAbi::Msvc),
			["~scalar", "run()", "extra()"]
		);
		assert_eq!(
			slot_names(&records, "HeldExplicit", CppAbi::Msvc),
			["~scalar", "run()", "extra()"]
		);
		assert_eq!(
			records
				.vtable_layout("Held", CppAbi::Itanium)
				.unwrap()
				.slots[2]
				.method
				.qualified_name,
			"Held::run"
		);

		// The template itself cannot have a probe: its layout is generic.
		assert!(matches!(
			records.vtable_model("Holder", CppAbi::Itanium).unwrap(),
			VtableModel::Unsupported { .. }
		));
	}

	#[test]
	fn reports_template_dependent_vtables_as_unsupported() {
		let records = records(
			r#"
			struct Plain { int value; };
			template <class T> struct Factory { virtual T *create() = 0; };
			struct UsesFactory : Factory<Plain> { Plain *create() override; };

			template <class Base> struct Mixin : Base { };
			struct UsesMixin : Mixin<Plain> { virtual void run(); };

			template <class T> struct Outer { struct Inner { virtual void run(); T value; }; };
			"#,
		);

		for record in ["UsesFactory", "UsesMixin", "Factory", "Outer::Inner"] {
			assert!(
				matches!(
					records.vtable_model(record, CppAbi::Itanium).unwrap(),
					VtableModel::Unsupported { .. }
				),
				"{record}"
			);
		}

		assert_eq!(
			records.vtable_model("Outer", CppAbi::Itanium).unwrap(),
			VtableModel::NotPolymorphic
		);
	}

	#[test]
	fn reports_covariant_overriders_as_unsupported() {
		let records = records(
			r#"
			struct Result { int value; };
			struct Derived : Result { virtual void other(); };
			struct Base { virtual Result *get(); };
			struct Covariant : Base { Derived *get() override; };
			"#,
		);

		assert!(matches!(
			records.vtable_model("Covariant", CppAbi::Itanium).unwrap(),
			VtableModel::Unsupported { ref reason } if reason.contains("covariant")
		));
	}

	/// Expected orders are Clang's own `-fdump-vtable-layouts` output for
	/// `x86_64-pc-windows-msvc` and `x86_64-unknown-linux-gnu`.
	#[test]
	fn microsoft_abi_groups_and_reverses_new_overloads() {
		let fixture = r#"
			struct Vector { float x, y, z; };
			struct IServerToolsLike {
				virtual ~IServerToolsLike();
				virtual bool GetKeyValue(const char *field) = 0;
				virtual bool SetKeyValue(const char *field, const char *value) = 0;
				virtual bool SetKeyValue(const char *field, float value) = 0;
				virtual bool SetKeyValue(const char *field, const Vector &value) = 0;
				virtual void Other() = 0;
			};

			struct S {
				void f(double);
				virtual void g();
				virtual void f(int);
				virtual ~S();
				virtual void a();
				int h;
				virtual void h2();
			};

			struct B { virtual void f(int); virtual void z(); };
			struct D : B {
				virtual void f(double);
				virtual void y();
				virtual void f(char);
				void f(int) override;
				using B::z;
				virtual void z(int);
			};
			"#;
		let index = records(fixture);

		assert_eq!(
			slot_names(&index, "IServerToolsLike", CppAbi::Msvc),
			[
				"~scalar",
				"GetKeyValue(const char *)",
				"SetKeyValue(const char *, const Vector &)",
				"SetKeyValue(const char *, float)",
				"SetKeyValue(const char *, const char *)",
				"Other()",
			]
		);
		assert_eq!(
			slot_names(&index, "IServerToolsLike", CppAbi::Itanium),
			[
				"~complete",
				"~deleting",
				"GetKeyValue(const char *)",
				"SetKeyValue(const char *, const char *)",
				"SetKeyValue(const char *, float)",
				"SetKeyValue(const char *, const Vector &)",
				"Other()",
			]
		);

		assert_eq!(
			slot_names(&index, "S", CppAbi::Msvc),
			["f(int)", "g()", "~scalar", "a()", "h2()"]
		);
		assert_eq!(
			slot_names(&index, "S", CppAbi::Itanium),
			["g()", "f(int)", "~complete", "~deleting", "a()", "h2()"]
		);

		assert_eq!(
			slot_names(&index, "D", CppAbi::Msvc),
			["f(int)", "z()", "f(char)", "f(double)", "y()", "z(int)"]
		);
		assert_eq!(
			slot_names(&index, "D", CppAbi::Itanium),
			["f(int)", "z()", "f(double)", "y()", "f(char)", "z(int)"]
		);

		let _ = records(&format!(
			"{fixture}\n{}",
			probe(&index, "D", CppAbi::MsvcX86).source
		));
	}

	#[test]
	fn overload_field_names_follow_declaration_order_under_every_abi() {
		let records = records(
			r#"
			struct Vector { float x, y, z; };
			struct Tools {
				virtual bool SetKeyValue(const char *field, const char *value) = 0;
				virtual bool SetKeyValue(const char *field, float value) = 0;
				virtual bool SetKeyValue(const char *field, const Vector &value) = 0;
			};
			struct MoreTools : Tools { virtual bool SetKeyValue(const char *field, int value) = 0; };
			"#,
		);

		let named = |abi| {
			let probe = probe(&records, "MoreTools", abi);
			probe
				.field_names
				.iter()
				.cloned()
				.zip(
					probe
						.layout
						.slots
						.iter()
						.map(|slot| slot.method.display_name.clone()),
				)
				.collect::<BTreeMap<_, _>>()
		};

		let itanium = named(CppAbi::Itanium);
		assert_eq!(itanium, named(CppAbi::Msvc));
		assert_eq!(
			itanium["MoreTools_SetKeyValue"],
			"SetKeyValue(const char *, const char *)"
		);
		assert_eq!(
			itanium["MoreTools_SetKeyValue2"],
			"SetKeyValue(const char *, const Vector &)"
		);
		assert_eq!(
			itanium["MoreTools_SetKeyValue3"],
			"SetKeyValue(const char *, int)"
		);

		assert_eq!(
			probe(&records, "MoreTools", CppAbi::Msvc).field_names,
			[
				"MoreTools_SetKeyValue2",
				"MoreTools_SetKeyValue1",
				"MoreTools_SetKeyValue",
				"MoreTools_SetKeyValue3"
			]
		);
	}
}
