//! Rust layouts of bindgen's `repr(C)` output, checked against the C++
//! layouts which bindgen asserts.
//!
//! Bindgen cannot express every C++ layout. It may translate a class template
//! with parameters it never binds, it drops `#pragma pack` from templates, and
//! it cannot reuse the tail padding of a non-POD base as the Itanium ABI does.
//! Rust reports those mistakes only once the generated layout assertions are
//! compiled. This module evaluates the same assertions during generation, so
//! the generator can make exactly the offending types opaque instead.

use proc_macro2::{Delimiter, TokenTree};
use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet};
use syn::parse::{ParseStream, Parser};
use syn::visit::Visit;
use syn::{
	BinOp, Expr, File, GenericArgument, GenericParam, Generics, Item, Lit, PathArguments, Stmt,
	Type,
};

/// Target facts which decide the size and alignment of Rust primitives.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) struct TargetLayout {
	pub pointer_width: u64,
	pub c_long_width: u64,

	/// Alignment of `u64`, `i64`, and `f64`, which is 4 on i686 Linux.
	pub int64_align: u64,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct Layout {
	size: u64,
	align: u64,
}

impl Layout {
	const fn new(size: u64, align: u64) -> Self {
		Self { size, align }
	}
}

/// C++ facts which bindgen asserted for one Rust type.
#[derive(Debug, Clone, Default)]
struct Asserted {
	size: Option<u64>,
	align: Option<u64>,
	offsets: BTreeMap<String, u64>,
}

impl Asserted {
	fn layout(&self) -> Option<Layout> {
		Some(Layout::new(self.size?, self.align?))
	}
}

/// Generated items whose Rust definition does not describe their C++ layout,
/// keyed by item name, with the first mismatch found for each.
pub(crate) type LayoutMismatches = BTreeMap<String, String>;

/// Finds the generated items whose C++ declarations must become opaque for
/// every layout assertion in `syntax` to hold and every name to resolve.
///
/// A type whose own assertions fail is reported; a type that is only wrong
/// because it contains such a type is not, because the evaluation continues
/// with the asserted C++ layout of each reported type. An item naming an
/// undefined type (a template parameter bindgen never bound) is reported by
/// its own name, which may be a nested type alias of the record to blame.
pub(crate) fn find_layout_mismatches(syntax: &File, target: TargetLayout) -> LayoutMismatches {
	let mut unresolved = LayoutMismatches::new();

	for (item, name) in unresolved_type_names(syntax) {
		unresolved
			.entry(item.clone())
			.or_insert_with(|| format!("`{item}` refers to `{name}`, which is not defined"));
	}

	// Records which become opaque for an unbound name are left out of the
	// evaluation, so nothing containing them is blamed in their stead.
	let mut evaluator = Evaluator::new(syntax, target, unresolved.keys().cloned().collect());
	let keys = evaluator.asserted.keys().cloned().collect::<Vec<_>>();

	for key in keys {
		let type_ = evaluator.asserted_types[&key].clone();
		let _ = evaluator.layout_of(&type_);
	}

	let mut mismatches = evaluator.mismatches;
	mismatches.extend(unresolved);
	mismatches
}

/// Removes type aliases which name an undefined type and which no other item
/// refers to, returning their names.
///
/// Bindgen still emits the member typedefs of a class template it was told to
/// keep opaque, such as `CUtlMap<K, T>::CTree`, with the template's parameters
/// unbound. Once nothing uses them, they are only dead, uncompilable code.
pub(crate) fn remove_unreferenced_broken_aliases(syntax: &mut File) -> Vec<String> {
	let mut removed = Vec::new();

	loop {
		let broken = unresolved_type_names(syntax)
			.into_iter()
			.map(|(item, _)| item)
			.filter(|item| {
				syntax
					.items
					.iter()
					.any(|candidate| matches!(candidate, Item::Type(alias) if alias.ident == item))
			})
			.collect::<BTreeSet<_>>();

		let referenced = referenced_elsewhere(syntax, &broken);
		let unreferenced = broken
			.difference(&referenced)
			.cloned()
			.collect::<BTreeSet<_>>();

		if unreferenced.is_empty() {
			return removed;
		}

		syntax.items.retain(
			|item| !matches!(item, Item::Type(alias) if unreferenced.contains(&alias.ident.to_string())),
		);
		removed.extend(unreferenced);
	}
}

/// The names in `candidates` which an item other than their own definition mentions.
fn referenced_elsewhere(syntax: &File, candidates: &BTreeSet<String>) -> BTreeSet<String> {
	struct Mentions<'a> {
		candidates: &'a BTreeSet<String>,
		found: BTreeSet<String>,
	}

	impl<'ast> Visit<'ast> for Mentions<'_> {
		fn visit_ident(&mut self, ident: &'ast proc_macro2::Ident) {
			let name = ident.to_string();

			if self.candidates.contains(&name) {
				self.found.insert(name);
			}
		}

		fn visit_macro(&mut self, mac: &'ast syn::Macro) {
			for token in mac.tokens.clone() {
				if let TokenTree::Ident(ident) = token {
					self.visit_ident(&ident);
				}
			}

			syn::visit::visit_macro(self, mac);
		}
	}

	let mut found = BTreeSet::new();

	for item in &syntax.items {
		let own = match item {
			Item::Type(alias) => Some(alias.ident.to_string()),
			_ => None,
		};

		let mut mentions = Mentions {
			candidates,
			found: BTreeSet::new(),
		};

		mentions.visit_item(item);

		if let Some(own) = &own {
			mentions.found.remove(own);
		}

		found.extend(mentions.found);
	}

	found
}

#[derive(Clone, Copy)]
enum RecordKind {
	Struct,
	Union,
}

#[derive(Clone)]
struct Record<'a> {
	kind: RecordKind,
	generics: &'a Generics,
	fields: Vec<&'a syn::Field>,
	packed: Option<u64>,
	align: Option<u64>,
}

#[derive(Debug)]
enum Unknown {
	Name(String),
	Shape(String),
	Cycle(String),

	/// Depends on a record which is already being made opaque.
	Deferred,
}

impl std::fmt::Display for Unknown {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Name(name) => write!(f, "`{name}` is not defined"),
			Self::Shape(shape) => write!(f, "the layout of `{shape}` is not computable"),
			Self::Cycle(name) => write!(f, "`{name}` contains itself"),
			Self::Deferred => f.write_str("it depends on a record which is being made opaque"),
		}
	}
}

struct Evaluator<'a> {
	target: TargetLayout,
	records: BTreeMap<String, Record<'a>>,
	aliases: BTreeMap<String, &'a syn::ItemType>,

	/// `pub use self::original as renamed;`, which bindgen emits for typedefs of enums.
	renames: BTreeMap<String, String>,
	asserted: BTreeMap<String, Asserted>,
	asserted_types: BTreeMap<String, Type>,
	deferred: BTreeSet<String>,
	memo: BTreeMap<String, Result<Layout, Option<String>>>,
	in_progress: BTreeSet<String>,
	mismatches: LayoutMismatches,
}

impl<'a> Evaluator<'a> {
	fn new(syntax: &'a File, target: TargetLayout, deferred: BTreeSet<String>) -> Self {
		let mut records = BTreeMap::new();
		let mut aliases = BTreeMap::new();
		let mut renames = BTreeMap::new();
		let mut asserted = BTreeMap::<String, Asserted>::new();
		let mut asserted_types = BTreeMap::new();

		for item in &syntax.items {
			match item {
				Item::Struct(item) => {
					let (packed, align) = repr_modifiers(&item.attrs);
					records.insert(
						item.ident.to_string(),
						Record {
							kind: RecordKind::Struct,
							generics: &item.generics,
							fields: item.fields.iter().collect(),
							packed,
							align,
						},
					);
				}

				Item::Union(item) => {
					let (packed, align) = repr_modifiers(&item.attrs);
					records.insert(
						item.ident.to_string(),
						Record {
							kind: RecordKind::Union,
							generics: &item.generics,
							fields: item.fields.named.iter().collect(),
							packed,
							align,
						},
					);
				}

				Item::Type(item) => {
					aliases.insert(item.ident.to_string(), item);
				}

				Item::Use(item) => collect_local_renames(&item.tree, &mut renames),

				Item::Const(item) if item.ident == "_" => {
					for (type_, fact) in layout_assertions(&item.expr) {
						let key = type_key(&type_);
						let entry = asserted.entry(key.clone()).or_default();

						match fact {
							Fact::Size(size) => entry.size = Some(size),
							Fact::Align(align) => entry.align = Some(align),
							Fact::Offset(field, offset) => {
								entry.offsets.insert(field, offset);
							}
						}

						asserted_types.entry(key).or_insert(type_);
					}
				}

				_ => {}
			}
		}

		Self {
			target,
			records,
			aliases,
			renames,
			asserted,
			asserted_types,
			deferred,
			memo: BTreeMap::new(),
			in_progress: BTreeSet::new(),
			mismatches: BTreeMap::new(),
		}
	}

	/// The layout of a concrete type. A type whose Rust layout contradicts
	/// its C++ assertions is recorded, and its asserted layout is used.
	fn layout_of(&mut self, type_: &Type) -> Result<Layout, Unknown> {
		let key = type_key(type_);

		if let Some(result) = self.memo.get(&key) {
			return result
				.clone()
				.map_err(|problem| problem.map_or(Unknown::Deferred, Unknown::Shape));
		}

		if !self.in_progress.insert(key.clone()) {
			return Err(Unknown::Cycle(key));
		}

		let computed = self.compute(type_);
		self.in_progress.remove(&key);

		let result = match self.asserted.get(&key).cloned() {
			None => computed,
			Some(_) if matches!(computed, Err(Unknown::Deferred)) => computed,
			Some(asserted) => {
				let problem = match &computed {
					Err(unknown) => Some(unknown.to_string()),
					Ok(layout) => self.contradiction(type_, *layout, &asserted),
				};

				match problem {
					None => computed,
					Some(problem) => {
						if let Some(owner) = defining_name(type_) {
							self.mismatches.entry(owner).or_insert(problem);
						}

						asserted.layout().ok_or_else(|| Unknown::Shape(key.clone()))
					}
				}
			}
		};

		let memo = match &result {
			Ok(layout) => Ok(*layout),
			Err(Unknown::Deferred) => Err(None),
			Err(unknown) => Err(Some(unknown.to_string())),
		};

		self.memo.insert(key, memo);
		result
	}

	fn contradiction(
		&mut self,
		type_: &Type,
		layout: Layout,
		asserted: &Asserted,
	) -> Option<String> {
		if asserted.size.is_some_and(|size| size != layout.size) {
			return Some(format!(
				"its Rust size is {}, but C++ says {}",
				layout.size,
				asserted.size.unwrap_or_default()
			));
		}

		if asserted.align.is_some_and(|align| align != layout.align) {
			return Some(format!(
				"its Rust alignment is {}, but C++ says {}",
				layout.align,
				asserted.align.unwrap_or_default()
			));
		}

		if asserted.offsets.is_empty() {
			return None;
		}

		let offsets = match self.field_offsets(type_) {
			Ok(offsets) => offsets,
			Err(unknown) => return Some(unknown.to_string()),
		};

		for (field, expected) in &asserted.offsets {
			match offsets.get(field) {
				Some(actual) if actual == expected => {}
				Some(actual) => {
					return Some(format!(
						"its Rust field `{field}` is at offset {actual}, but C++ says {expected}"
					));
				}
				None => return Some(format!("it has no Rust field `{field}`")),
			}
		}

		None
	}

	fn compute(&mut self, type_: &Type) -> Result<Layout, Unknown> {
		let pointer = Layout::new(self.target.pointer_width, self.target.pointer_width);

		match type_ {
			Type::Paren(inner) => self.layout_of(&inner.elem),
			Type::Group(inner) => self.layout_of(&inner.elem),
			Type::Ptr(_) | Type::FnPtr(_) => Ok(pointer),
			Type::Tuple(tuple) if tuple.elems.is_empty() => Ok(Layout::new(0, 1)),

			Type::Array(array) => {
				let element = self.layout_of(&array.elem)?;
				let length = integer(&array.len).ok_or_else(|| Unknown::Shape(type_key(type_)))?;
				Ok(Layout::new(element.size * length, element.align))
			}

			Type::Path(path) if path.qself.is_none() => {
				let segments = path
					.path
					.segments
					.iter()
					.map(|segment| segment.ident.to_string())
					.collect::<Vec<_>>();
				let last = path.path.segments.last().expect("a path has a segment");
				let arguments = type_arguments(&last.arguments);

				if let Some(layout) = self.primitive(&segments) {
					return Ok(layout);
				}

				match (
					path.path.leading_colon.is_some(),
					segments
						.iter()
						.map(String::as_str)
						.collect::<Vec<_>>()
						.as_slice(),
				) {
					(true, ["std", "marker", "PhantomData"]) => Ok(Layout::new(0, 1)),
					(true, ["std", "mem", "ManuallyDrop"] | ["std", "cell", "UnsafeCell"]) => {
						match arguments.as_slice() {
							[inner] => self.layout_of(inner),
							_ => Err(Unknown::Shape(type_key(type_))),
						}
					}
					(true, ["std", "option", "Option"]) => match arguments.as_slice() {
						[Type::FnPtr(_)] => Ok(pointer),
						_ => Err(Unknown::Shape(type_key(type_))),
					},
					(false, [name]) => self.named(name, &arguments),
					_ => Err(Unknown::Name(segments.join("::"))),
				}
			}

			_ => Err(Unknown::Shape(type_key(type_))),
		}
	}

	fn named(&mut self, name: &str, arguments: &[Type]) -> Result<Layout, Unknown> {
		if self.deferred.contains(name) {
			return Err(Unknown::Deferred);
		}

		if let Some(original) = self.renames.get(name).cloned()
			&& arguments.is_empty()
		{
			return self.named(&original, arguments);
		}

		if let Some(alias) = self.aliases.get(name).copied() {
			let substitutions =
				bind(&alias.generics, arguments).ok_or_else(|| Unknown::Shape(name.to_owned()))?;
			return self.layout_of(&substitute(&alias.ty, &substitutions));
		}

		let Some(record) = self.records.get(name).cloned() else {
			return Err(Unknown::Name(name.to_owned()));
		};

		let substitutions =
			bind(record.generics, arguments).ok_or_else(|| Unknown::Shape(name.to_owned()))?;
		let fields = record
			.fields
			.iter()
			.map(|field| substitute(&field.ty, &substitutions))
			.collect::<Vec<_>>();
		let mut layouts = Vec::with_capacity(fields.len());

		for field in &fields {
			layouts.push(self.layout_of(field)?);
		}

		Ok(match record.kind {
			RecordKind::Struct => place_fields(&layouts, record.packed, record.align).0,
			RecordKind::Union => {
				let pack = record.packed.unwrap_or(u64::MAX);
				let align = layouts
					.iter()
					.map(|layout| layout.align.min(pack))
					.chain(record.align)
					.max()
					.unwrap_or(1);
				let size = layouts.iter().map(|layout| layout.size).max().unwrap_or(0);
				Layout::new(size.next_multiple_of(align), align)
			}
		})
	}

	fn field_offsets(&mut self, type_: &Type) -> Result<BTreeMap<String, u64>, Unknown> {
		let Type::Path(path) = type_ else {
			return Err(Unknown::Shape(type_key(type_)));
		};

		let last = path.path.segments.last().expect("a path has a segment");
		let name = last.ident.to_string();
		let Some(record) = self.records.get(&name).cloned() else {
			return Err(Unknown::Name(name));
		};

		let substitutions = bind(record.generics, &type_arguments(&last.arguments))
			.ok_or_else(|| Unknown::Shape(name.clone()))?;
		let mut layouts = Vec::new();
		let mut names = Vec::new();

		for (index, field) in record.fields.iter().enumerate() {
			layouts.push(self.layout_of(&substitute(&field.ty, &substitutions))?);
			names.push(
				field
					.ident
					.as_ref()
					.map(ToString::to_string)
					.unwrap_or_else(|| index.to_string()),
			);
		}

		let offsets = match record.kind {
			RecordKind::Struct => place_fields(&layouts, record.packed, record.align).1,
			RecordKind::Union => vec![0; layouts.len()],
		};

		Ok(names.into_iter().zip(offsets).collect())
	}

	fn primitive(&self, segments: &[String]) -> Option<Layout> {
		let name = match segments {
			[name] => name.as_str(),
			[std, os, raw, name] if std == "std" && os == "os" && raw == "raw" => name.as_str(),
			[core, ffi, name] if (core == "core" || core == "std") && ffi == "ffi" => name.as_str(),
			_ => return None,
		};

		let int64 = Layout::new(8, self.target.int64_align);
		let pointer = Layout::new(self.target.pointer_width, self.target.pointer_width);

		Some(match name {
			"bool" | "u8" | "i8" | "c_char" | "c_schar" | "c_uchar" => Layout::new(1, 1),
			"u16" | "i16" | "c_short" | "c_ushort" => Layout::new(2, 2),
			"u32" | "i32" | "f32" | "char" | "c_int" | "c_uint" | "c_float" => Layout::new(4, 4),
			"u64" | "i64" | "f64" | "c_longlong" | "c_ulonglong" | "c_double" => int64,
			"c_long" | "c_ulong" if self.target.c_long_width == 8 => int64,
			"c_long" | "c_ulong" => Layout::new(4, 4),
			"u128" | "i128" => Layout::new(16, 16),
			"usize" | "isize" => pointer,
			_ => return None,
		})
	}
}

/// Places fields by C rules, limited by `packed` and raised by `align`.
fn place_fields(fields: &[Layout], packed: Option<u64>, align: Option<u64>) -> (Layout, Vec<u64>) {
	let pack = packed.unwrap_or(u64::MAX);
	let mut offset = 0_u64;
	let mut record_align = align.unwrap_or(1);
	let mut offsets = Vec::with_capacity(fields.len());

	for field in fields {
		let field_align = field.align.min(pack);
		offset = offset.next_multiple_of(field_align);
		offsets.push(offset);
		offset += field.size;
		record_align = record_align.max(field_align);
	}

	(
		Layout::new(offset.next_multiple_of(record_align), record_align),
		offsets,
	)
}

fn repr_modifiers(attributes: &[syn::Attribute]) -> (Option<u64>, Option<u64>) {
	let mut packed = None;
	let mut align = None;

	for attribute in attributes
		.iter()
		.filter(|attribute| attribute.path().is_ident("repr"))
	{
		let syn::Meta::List(list) = &attribute.meta else {
			continue;
		};
		let mut tokens = list.tokens.clone().into_iter().peekable();

		while let Some(token) = tokens.next() {
			let TokenTree::Ident(ident) = token else {
				continue;
			};
			let argument = match tokens.peek() {
				Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Parenthesis => {
					let value = group
						.stream()
						.to_string()
						.trim_end_matches("usize")
						.trim()
						.parse::<u64>()
						.ok();
					tokens.next();
					value
				}
				_ => None,
			};

			if ident == "packed" {
				packed = Some(argument.unwrap_or(1));
			} else if ident == "align" {
				align = argument;
			}
		}
	}

	(packed, align)
}

enum Fact {
	Size(u64),
	Align(u64),
	Offset(String, u64),
}

/// The facts in one of bindgen's `const _: () = { [..][size_of::<T>() - N]; };` blocks.
fn layout_assertions(expr: &Expr) -> Vec<(Type, Fact)> {
	let Expr::Block(block) = expr else {
		return Vec::new();
	};
	let mut facts = Vec::new();

	for statement in &block.block.stmts {
		let Stmt::Expr(Expr::Index(index), _) = statement else {
			continue;
		};
		let Expr::Binary(binary) = index.index.as_ref() else {
			continue;
		};

		if !matches!(binary.op, BinOp::Sub(_)) {
			continue;
		}

		let Some(expected) = integer(&binary.right) else {
			continue;
		};

		match binary.left.as_ref() {
			Expr::Call(call) if call.args.is_empty() => {
				let Expr::Path(path) = call.func.as_ref() else {
					continue;
				};
				let Some(segment) = path.path.segments.last() else {
					continue;
				};
				let arguments = type_arguments(&segment.arguments);
				let [type_] = arguments.as_slice() else {
					continue;
				};
				let type_ = type_.clone();

				if segment.ident == "size_of" {
					facts.push((type_, Fact::Size(expected)));
				} else if segment.ident == "align_of" {
					facts.push((type_, Fact::Align(expected)));
				}
			}

			Expr::Macro(mac)
				if mac
					.mac
					.path
					.segments
					.last()
					.is_some_and(|segment| segment.ident == "offset_of") =>
			{
				let parser = |input: ParseStream| {
					let type_ = input.parse::<Type>()?;
					input.parse::<syn::Token![,]>()?;
					let field = input.parse::<syn::Member>()?;
					Ok((type_, field))
				};

				if let Ok((type_, field)) = parser.parse2(mac.mac.tokens.clone()) {
					let field = match field {
						syn::Member::Named(ident) => ident.to_string(),
						syn::Member::Unnamed(index) => index.index.to_string(),
					};
					facts.push((type_, Fact::Offset(field, expected)));
				}
			}

			_ => {}
		}
	}

	facts
}

fn integer(expr: &Expr) -> Option<u64> {
	match expr {
		Expr::Lit(literal) => match &literal.lit {
			Lit::Int(integer) => integer.base10_parse().ok(),
			_ => None,
		},
		Expr::Paren(inner) => integer(&inner.expr),
		Expr::Group(inner) => integer(&inner.expr),
		_ => None,
	}
}

fn type_arguments(arguments: &PathArguments) -> Vec<Type> {
	let PathArguments::AngleBracketed(arguments) = arguments else {
		return Vec::new();
	};

	arguments
		.args
		.iter()
		.filter_map(|argument| match argument {
			GenericArgument::Type(type_) => Some(type_.clone()),
			_ => None,
		})
		.collect()
}

fn bind(generics: &Generics, arguments: &[Type]) -> Option<BTreeMap<String, Type>> {
	let parameters = generics
		.params
		.iter()
		.filter_map(|parameter| match parameter {
			GenericParam::Type(parameter) => Some(parameter.ident.to_string()),
			_ => None,
		})
		.collect::<Vec<_>>();

	(parameters.len() == arguments.len()).then(|| {
		parameters
			.into_iter()
			.zip(arguments.iter().cloned())
			.collect()
	})
}

/// Replaces generic parameters wherever they can affect a layout. Pointee and
/// function pointer types are left alone: pointers have a fixed layout.
fn substitute(type_: &Type, substitutions: &BTreeMap<String, Type>) -> Type {
	fn substitute_in_place(type_: &mut Type, substitutions: &BTreeMap<String, Type>) {
		match type_ {
			Type::Path(path) => {
				if path.qself.is_none()
					&& path.path.leading_colon.is_none()
					&& path.path.segments.len() == 1
					&& path.path.segments[0].arguments.is_none()
					&& let Some(replacement) =
						substitutions.get(&path.path.segments[0].ident.to_string())
				{
					*type_ = replacement.clone();
					return;
				}

				for segment in &mut path.path.segments {
					if let PathArguments::AngleBracketed(arguments) = &mut segment.arguments {
						for argument in &mut arguments.args {
							if let GenericArgument::Type(argument) = argument {
								substitute_in_place(argument, substitutions);
							}
						}
					}
				}
			}

			Type::Array(array) => substitute_in_place(&mut array.elem, substitutions),
			Type::Paren(inner) => substitute_in_place(&mut inner.elem, substitutions),
			Type::Group(inner) => substitute_in_place(&mut inner.elem, substitutions),

			Type::Tuple(tuple) => {
				for element in &mut tuple.elems {
					substitute_in_place(element, substitutions);
				}
			}

			_ => {}
		}
	}

	let mut substituted = type_.clone();

	if !substitutions.is_empty() {
		substitute_in_place(&mut substituted, substitutions);
	}

	substituted
}

fn type_key(type_: &Type) -> String {
	type_.to_token_stream().to_string()
}

/// The generated item which defines `type_`: a struct, union, or alias name.
fn defining_name(type_: &Type) -> Option<String> {
	match type_ {
		Type::Path(path) if path.qself.is_none() && path.path.leading_colon.is_none() => path
			.path
			.segments
			.last()
			.map(|segment| segment.ident.to_string()),
		Type::Paren(inner) => defining_name(&inner.elem),
		Type::Group(inner) => defining_name(&inner.elem),
		_ => None,
	}
}

/// `(item, name)` pairs for single-segment type names which neither an item
/// nor a generic parameter in scope defines.
fn unresolved_type_names(syntax: &File) -> Vec<(String, String)> {
	struct Collector<'a> {
		defined: &'a BTreeSet<String>,
		in_scope: BTreeSet<String>,
		unresolved: BTreeSet<String>,
	}

	impl<'ast> Visit<'ast> for Collector<'_> {
		fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
			if path.qself.is_none()
				&& path.path.leading_colon.is_none()
				&& path.path.segments.len() == 1
				&& let Some(segment) = path.path.segments.first()
			{
				let name = segment.ident.to_string();

				if !self.defined.contains(&name)
					&& !self.in_scope.contains(&name)
					&& !is_primitive_name(&name)
				{
					self.unresolved.insert(name);
				}
			}

			syn::visit::visit_type_path(self, path);
		}
	}

	let mut defined = BTreeSet::from(["Self".to_owned()]);

	for item in &syntax.items {
		match item {
			Item::Struct(item) => {
				defined.insert(item.ident.to_string());
			}
			Item::Union(item) => {
				defined.insert(item.ident.to_string());
			}
			Item::Enum(item) => {
				defined.insert(item.ident.to_string());
			}
			Item::Type(item) => {
				defined.insert(item.ident.to_string());
			}
			Item::Use(item) => collect_use_names(&item.tree, &mut defined),
			_ => {}
		}
	}

	let mut unresolved = Vec::new();

	for item in &syntax.items {
		let (name, generics) = match item {
			Item::Struct(item) => (item.ident.to_string(), &item.generics),
			Item::Union(item) => (item.ident.to_string(), &item.generics),
			Item::Type(item) => (item.ident.to_string(), &item.generics),
			_ => continue,
		};

		let mut collector = Collector {
			defined: &defined,
			in_scope: generics
				.params
				.iter()
				.filter_map(|parameter| match parameter {
					GenericParam::Type(parameter) => Some(parameter.ident.to_string()),
					_ => None,
				})
				.collect(),
			unresolved: BTreeSet::new(),
		};

		collector.visit_item(item);
		unresolved.extend(
			collector
				.unresolved
				.into_iter()
				.map(|missing| (name.clone(), missing)),
		);
	}

	unresolved
}

/// `use self::original as renamed;` within the generated file itself.
fn collect_local_renames(tree: &syn::UseTree, renames: &mut BTreeMap<String, String>) {
	match tree {
		syn::UseTree::Path(path) if path.ident == "self" => {
			collect_local_renames(&path.tree, renames)
		}
		syn::UseTree::Rename(rename) => {
			renames.insert(rename.rename.to_string(), rename.ident.to_string());
		}
		syn::UseTree::Group(group) => {
			for tree in &group.items {
				collect_local_renames(tree, renames);
			}
		}
		_ => {}
	}
}

fn collect_use_names(tree: &syn::UseTree, names: &mut BTreeSet<String>) {
	match tree {
		syn::UseTree::Path(path) => collect_use_names(&path.tree, names),
		syn::UseTree::Name(name) => {
			names.insert(name.ident.to_string());
		}
		syn::UseTree::Rename(rename) => {
			names.insert(rename.rename.to_string());
		}
		syn::UseTree::Group(group) => {
			for tree in &group.items {
				collect_use_names(tree, names);
			}
		}
		syn::UseTree::Glob(_) => {}
	}
}

fn is_primitive_name(name: &str) -> bool {
	matches!(
		name,
		"bool"
			| "char"
			| "str"
			| "u8"
			| "u16"
			| "u32"
			| "u64"
			| "u128"
			| "usize"
			| "i8"
			| "i16"
			| "i32"
			| "i64"
			| "i128"
			| "isize"
			| "f32"
			| "f64"
	)
}

#[cfg(test)]
mod tests {
	use super::*;
	use syn::parse_quote;

	const LINUX_64: TargetLayout = TargetLayout {
		pointer_width: 8,
		c_long_width: 8,
		int64_align: 8,
	};

	const WINDOWS_64: TargetLayout = TargetLayout {
		pointer_width: 8,
		c_long_width: 4,
		int64_align: 8,
	};

	#[test]
	fn accepts_layouts_that_match_their_assertions() {
		let syntax: File = parse_quote! {
			#[repr(C, align(8))]
			pub struct __BindgenOpaqueArray8<T>(pub T);
			#[repr(C)]
			pub struct __BindgenBitfieldUnit<Storage> { storage: Storage }
			pub type _fieldtypes = ::std::os::raw::c_uint;
			pub use self::_fieldtypes as fieldtype_t;
			#[repr(C)]
			pub struct Inner<T> {
				pub _phantom_0: ::std::marker::PhantomData<::std::cell::UnsafeCell<T>>,
				pub value: T,
			}
			#[repr(C)]
			pub struct Outer__bindgen_vtable(::std::os::raw::c_void);
			#[repr(C)]
			pub struct Outer {
				pub vtable_: *const Outer__bindgen_vtable,
				pub flag: bool,
				pub inner: Inner<f64>,
				pub callback: ::std::option::Option<unsafe extern "C" fn(arg1: ::std::os::raw::c_long)>,
				pub blob: __BindgenOpaqueArray8<[u8; 12usize]>,
				pub _bindgen_align: [u32; 0],
				pub _bitfield_1: __BindgenBitfieldUnit<[u8; 2usize]>,
				pub kind: fieldtype_t,
				pub long_value: ::std::os::raw::c_long,
			}
			#[repr(C)]
			pub union Either { pub a: u8, pub b: ::std::mem::ManuallyDrop<[u16; 3usize]> }
			#[repr(C, packed(4))]
			pub struct Packed { pub a: u8, pub b: *mut Outer }
			const _: () = {
				["Size of Outer"][::std::mem::size_of::<Outer>() - 64usize];
				["Alignment of Outer"][::std::mem::align_of::<Outer>() - 8usize];
				["Offset of field: Outer::inner"][::std::mem::offset_of!(Outer, inner) - 16usize];
				["Offset of field: Outer::blob"][::std::mem::offset_of!(Outer, blob) - 32usize];
				["Offset of field: Outer::kind"][::std::mem::offset_of!(Outer, kind) - 52usize];
				["Offset of field: Outer::long_value"][::std::mem::offset_of!(Outer, long_value) - 56usize];
			};
			const _: () = {
				["Size of Either"][::std::mem::size_of::<Either>() - 6usize];
				["Alignment of Either"][::std::mem::align_of::<Either>() - 2usize];
			};
			const _: () = {
				["Size of Packed"][::std::mem::size_of::<Packed>() - 12usize];
				["Alignment of Packed"][::std::mem::align_of::<Packed>() - 4usize];
				["Offset of field: Packed::b"][::std::mem::offset_of!(Packed, b) - 4usize];
			};
			const _: () = {
				["Size of template specialization: Inner_open0_float_close0"][::std::mem::size_of::<Inner<f32>>() - 4usize];
				["Align of template specialization: Inner_open0_float_close0"][::std::mem::align_of::<Inner<f32>>() - 4usize];
			};
		};

		assert_eq!(
			find_layout_mismatches(&syntax, LINUX_64),
			LayoutMismatches::new()
		);
	}

	#[test]
	fn c_long_follows_the_target_data_model() {
		let syntax: File = parse_quote! {
			#[repr(C)]
			pub struct Longs { pub a: ::std::os::raw::c_long, pub b: ::std::os::raw::c_int }
			const _: () = { ["Size of Longs"][::std::mem::size_of::<Longs>() - 8usize]; };
		};

		assert_eq!(
			find_layout_mismatches(&syntax, WINDOWS_64),
			LayoutMismatches::new()
		);
		assert!(find_layout_mismatches(&syntax, LINUX_64).contains_key("Longs"));
	}

	#[test]
	fn reports_tail_padding_reuse_on_the_derived_record_only() {
		// `Derived::extra` lives in the tail padding of the non-POD base.
		let syntax: File = parse_quote! {
			#[repr(C)]
			pub struct Base__bindgen_vtable(::std::os::raw::c_void);
			#[repr(C)]
			pub struct Base { pub vtable_: *const Base__bindgen_vtable, pub value: u32 }
			#[repr(C)]
			pub struct Derived { pub _base: Base, pub extra: u32 }
			#[repr(C)]
			pub struct Holder { pub derived: Derived, pub tail: u8 }
			const _: () = {
				["Size of Base"][::std::mem::size_of::<Base>() - 16usize];
				["Offset of field: Base::value"][::std::mem::offset_of!(Base, value) - 8usize];
			};
			const _: () = {
				["Size of Derived"][::std::mem::size_of::<Derived>() - 16usize];
				["Alignment of Derived"][::std::mem::align_of::<Derived>() - 8usize];
				["Offset of field: Derived::extra"][::std::mem::offset_of!(Derived, extra) - 12usize];
			};
			const _: () = {
				["Size of Holder"][::std::mem::size_of::<Holder>() - 24usize];
				["Offset of field: Holder::tail"][::std::mem::offset_of!(Holder, tail) - 16usize];
			};
		};

		let mismatches = find_layout_mismatches(&syntax, LINUX_64);
		assert_eq!(
			mismatches.keys().collect::<Vec<_>>(),
			["Derived"],
			"{mismatches:?}"
		);
	}

	#[test]
	fn reports_the_template_behind_a_wrong_specialization() {
		// `#pragma pack(4)` is lost on the template, so its instantiation is over-aligned.
		let syntax: File = parse_quote! {
			#[repr(C)]
			pub struct serializedstudioptr_t<T> {
				pub _phantom_0: ::std::marker::PhantomData<::std::cell::UnsafeCell<T>>,
				pub m_pData: *mut T,
			}
			#[repr(C, packed(4))]
			pub struct mstudiomesh_t { pub material: ::std::os::raw::c_int, pub data: serializedstudioptr_t<::std::os::raw::c_void> }
			#[repr(C)]
			pub struct Unpacked { pub first: u32, pub data: serializedstudioptr_t<u8> }
			const _: () = {
				["Size of mstudiomesh_t"][::std::mem::size_of::<mstudiomesh_t>() - 12usize];
				["Alignment of mstudiomesh_t"][::std::mem::align_of::<mstudiomesh_t>() - 4usize];
			};
			const _: () = {
				["Size of Unpacked"][::std::mem::size_of::<Unpacked>() - 12usize];
				["Offset of field: Unpacked::data"][::std::mem::offset_of!(Unpacked, data) - 4usize];
			};
			const _: () = {
				["Size of template specialization: serializedstudioptr_t_open0_unsigned_char_close0"][::std::mem::size_of::<serializedstudioptr_t<u8>>() - 8usize];
				["Align of template specialization: serializedstudioptr_t_open0_unsigned_char_close0"][::std::mem::align_of::<serializedstudioptr_t<u8>>() - 4usize];
			};
		};

		let mismatches = find_layout_mismatches(&syntax, LINUX_64);
		assert_eq!(
			mismatches.keys().collect::<Vec<_>>(),
			["serializedstudioptr_t"],
			"{mismatches:?}"
		);
	}

	#[test]
	fn removes_only_broken_aliases_nothing_uses() {
		let mut syntax: File = parse_quote! {
			#[repr(C)]
			pub struct CUtlRBTree<T, I> { pub root: I, pub elements: *mut T }
			pub type CUtlMap_CTree = CUtlRBTree<T, I>;
			pub type CUtlHash_Buckets = CUtlRBTree<T, A>;
			#[repr(C)]
			pub struct UsesHash { pub buckets: *mut CUtlHash_Buckets }
			pub type Broken = CUtlRBTree<T, u16>;
			pub type NamesBroken = Broken;
			#[repr(C)]
			pub struct CUtlMap { pub _bindgen_opaque_blob: [u64; 5usize] }
		};

		// A broken alias something still names must stay, so the record
		// using it is blamed instead.
		assert_eq!(
			remove_unreferenced_broken_aliases(&mut syntax),
			["CUtlMap_CTree"]
		);
		assert_eq!(syntax.items.len(), 6);
		assert_eq!(
			find_layout_mismatches(&syntax, LINUX_64)
				.keys()
				.collect::<Vec<_>>(),
			["Broken", "CUtlHash_Buckets"]
		);
	}

	#[test]
	fn reports_the_record_owning_an_unbound_template_parameter() {
		let syntax: File = parse_quote! {
			#[repr(C)]
			pub struct CUtlRBTree<T, I> { pub root: I, pub elements: *mut T }
			pub type CUtlMap_CTree = CUtlRBTree<T, I>;
			pub type CUtlMap_ElemType_t<T> = T;
			#[repr(C)]
			pub struct CUtlMap { pub m_Tree: CUtlMap_CTree }
			#[repr(C)]
			pub struct CUtlHash<C> { pub m_Buckets: CUtlVector<T, A>, pub m_Compare: C }
			#[repr(C)]
			pub struct CUtlVector<T, A> { pub memory: A, pub elements: *mut T }
			pub type Linked_const_iterator = Linked_iterator_t<List_t>;
			#[repr(C)]
			pub struct Linked { pub head: u16 }
			#[repr(C)]
			pub struct Linked_iterator_t<List_t> { pub list: *const List_t }
		};

		let mismatches = find_layout_mismatches(&syntax, LINUX_64);
		assert_eq!(
			mismatches.keys().collect::<Vec<_>>(),
			["CUtlHash", "CUtlMap_CTree", "Linked_const_iterator"],
			"{mismatches:?}"
		);
	}
}
