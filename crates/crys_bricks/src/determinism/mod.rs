use quote::{ToTokens, quote};
use syn::{ForeignItem, ImplItem, Item, Type};

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct ItemOrder {
	group: u8,
	family: String,
	kind: u8,
	tokens: String,
}

impl ItemOrder {
	pub fn foreign_item_order(item: &ForeignItem) -> ItemOrder {
		let (family, kind, needs_token_tiebreaker) = match item {
			ForeignItem::Fn(item) => (item.sig.ident.to_string(), 0, false),
			ForeignItem::Static(item) => (item.ident.to_string(), 1, false),
			ForeignItem::Type(item) => (item.ident.to_string(), 2, false),
			ForeignItem::Macro(item) => (
				item.mac
					.path
					.segments
					.last()
					.map(|segment| segment.ident.to_string())
					.unwrap_or_default(),
				3,
				true,
			),
			ForeignItem::Verbatim(tokens) => (tokens.to_string(), 4, false),
			_ => (String::new(), u8::MAX, true),
		};

		ItemOrder {
			group: 0,
			family,
			kind,
			tokens: if needs_token_tiebreaker {
				token_key(item)
			} else {
				String::new()
			},
		}
	}

	pub fn impl_item_order(item: &ImplItem) -> ItemOrder {
		let (group, family, needs_token_tiebreaker) = match item {
			ImplItem::Type(item) => (0, item.ident.to_string(), false),
			ImplItem::Const(item) => (1, item.ident.to_string(), false),
			ImplItem::Fn(item) => (2, item.sig.ident.to_string(), false),
			ImplItem::Macro(item) => (
				3,
				item.mac
					.path
					.segments
					.last()
					.map(|segment| segment.ident.to_string())
					.unwrap_or_default(),
				true,
			),
			ImplItem::Verbatim(tokens) => (4, tokens.to_string(), false),
			_ => (u8::MAX, String::new(), true),
		};

		ItemOrder {
			group,
			family,
			kind: 0,
			tokens: if needs_token_tiebreaker {
				token_key(item)
			} else {
				String::new()
			},
		}
	}

	pub fn module_item_order(item: &Item) -> ItemOrder {
		let (group, family, kind, needs_token_tiebreaker) = match item {
			Item::Use(item) => (1, token_key(&item.tree), 0, false),
			Item::Type(item) => (2, item.ident.to_string(), 0, false),
			Item::Const(item) => (3, item.ident.to_string(), 0, item.ident == "_"),
			Item::Static(item) => (4, item.ident.to_string(), 0, false),
			Item::Struct(item) => (5, item.ident.to_string(), 0, false),
			Item::Enum(item) => (5, item.ident.to_string(), 1, false),
			Item::Union(item) => (5, item.ident.to_string(), 2, false),
			Item::Impl(item) => (5, type_family(&item.self_ty), 3, true),
			Item::TraitAlias(item) => (5, item.ident.to_string(), 4, false),
			Item::Trait(item) => (5, item.ident.to_string(), 5, false),
			Item::Fn(item) => (6, item.sig.ident.to_string(), 0, false),
			Item::Mod(item) => (7, item.ident.to_string(), 0, false),
			Item::ExternCrate(item) => (7, item.ident.to_string(), 1, false),
			Item::Macro(item) => (
				7,
				item.ident
					.as_ref()
					.map(ToString::to_string)
					.unwrap_or_default(),
				2,
				true,
			),
			Item::Verbatim(tokens) => (7, tokens.to_string(), 3, false),
			Item::ForeignMod(item) => (8, token_key(&item.abi), 0, false),
			_ => (u8::MAX, String::new(), 0, true),
		};

		ItemOrder {
			group,
			family,
			kind,
			tokens: if needs_token_tiebreaker {
				token_key(item)
			} else {
				String::new()
			},
		}
	}
}

fn token_key(tokens: &impl ToTokens) -> String {
	quote!(#tokens).to_string()
}

fn type_family(ty: &Type) -> String {
	match ty {
		Type::Path(ty) => ty
			.path
			.segments
			.last()
			.map(|segment| segment.ident.to_string())
			.unwrap_or_else(|| token_key(ty)),
		Type::Group(ty) => type_family(&ty.elem),
		Type::Paren(ty) => type_family(&ty.elem),
		Type::Reference(ty) => type_family(&ty.elem),
		_ => token_key(ty),
	}
}
