//! Scope tracking and name resolution shared by every `resolve::*` module.
//!
//! Two modes, matching `knot_syntax`'s own `parse` vs `parse_decls` split:
//!
//! - **Module mode** (`Env::for_module`, used by `canonicalize_module`): the
//!   module's real `import` list is known, so a qualified reference's
//!   qualifier (`Foo` in `Foo.bar`) must match a real import's alias or module
//!   path — always checkable locally, no other module's contents needed.
//! - **Snippet mode** (`Env::for_decls`, used by `canonicalize_decls`, for the
//!   same bare-declaration-list fixtures `knot-syntax`'s own corpus harness
//!   parses via `parse_decls`): there is no import list at all, so a qualified
//!   reference is trusted at face value — otherwise every existing syntax-only
//!   corpus fixture that writes e.g. `Map.fromList` with no surrounding module
//!   header would spuriously fail name resolution. Unqualified names are still
//!   checked in both modes; only qualifier trust differs.
//!
//! Beyond a module's own imports, nothing here has real visibility into what
//! other modules actually export — there's no project-wide module loader yet.
//! The optional `ModuleRegistry` is the extension point for when one exists;
//! without it, cross-module questions this crate can't answer on its own
//! (does module X really export Y?) are resolved permissively rather than
//! rejected, documented at each such spot below.

use std::collections::{HashMap, HashSet};

use knot_syntax::ast::decl::{ExposedItem, Exposing, Import};

use crate::ast::Ref;
use crate::prelude;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Namespace {
    Value,
    Type,
    Ctor,
}

#[derive(Debug, Clone)]
pub struct CtorInfo {
    pub arity: usize,
    pub type_name: String,
}

/// What another module actually exports, when known. `constructors` maps a
/// *type* name to its variant names (for expanding a `Foo(..)` exposing item).
pub struct ModuleInterface {
    pub values: HashSet<String>,
    pub types: HashSet<String>,
    pub constructors: HashMap<String, Vec<String>>,
}

pub trait ModuleRegistry {
    fn exports(&self, module: &[String]) -> Option<ModuleInterface>;
}

pub enum ExposedLookup {
    Found(Vec<String>),
    Ambiguous(Vec<Vec<String>>),
    NotFound,
}

/// Tracks, for one namespace, which modules bring a given unqualified name
/// into scope. `wildcard_fallback` holds modules whose `exposing (..)` (module
/// header or import) couldn't be expanded into concrete names because no
/// `ModuleRegistry` was supplied — see module docs. It's a deliberately
/// last-resort, imprecise bucket: with no registry, an unresolved unqualified
/// name that could plausibly have come from *some* wildcard import is trusted
/// against the first such import instead of being rejected, matching the
/// same "permissive without real project knowledge" stance qualified
/// references already take.
#[derive(Default)]
struct ExposedTable {
    explicit: HashMap<String, Vec<Vec<String>>>,
    wildcard_fallback: Vec<Vec<String>>,
}

impl ExposedTable {
    fn add_explicit(&mut self, name: String, module: Vec<String>) {
        self.explicit.entry(name).or_default().push(module);
    }

    fn add_wildcard_fallback(&mut self, module: Vec<String>) {
        self.wildcard_fallback.push(module);
    }

    fn resolve(&self, name: &str) -> ExposedLookup {
        if let Some(modules) = self.explicit.get(name) {
            return if modules.len() == 1 {
                ExposedLookup::Found(modules[0].clone())
            } else {
                ExposedLookup::Ambiguous(modules.clone())
            };
        }
        match self.wildcard_fallback.first() {
            Some(m) => ExposedLookup::Found(m.clone()),
            None => ExposedLookup::NotFound,
        }
    }
}

pub struct Env<'a> {
    /// Local (lambda/let/case/do-bind) scopes, innermost last. Only values are
    /// ever locally scoped -- types and constructors are always module- or
    /// import-level.
    scopes: Vec<HashSet<String>>,
    top_level_values: HashSet<String>,
    top_level_types: HashSet<String>,
    /// Prelude constructors plus every locally-declared ADT's variants,
    /// populated before any declaration body is resolved (see `resolve/decl.rs`).
    constructors: HashMap<String, CtorInfo>,
    /// Qualifier (as written, dot-joined -- an alias, or a full unaliased
    /// module path) -> the real module path it refers to. Empty and never
    /// consulted in snippet mode.
    import_qualifiers: HashMap<String, Vec<String>>,
    exposed_values: ExposedTable,
    exposed_types: ExposedTable,
    exposed_ctors: ExposedTable,
    strict_qualifiers: bool,
    registry: Option<&'a dyn ModuleRegistry>,
}

impl<'a> Env<'a> {
    fn empty(strict_qualifiers: bool, registry: Option<&'a dyn ModuleRegistry>) -> Self {
        let mut constructors = HashMap::new();
        for (name, arity, type_name) in prelude::BUILTIN_CONSTRUCTORS {
            constructors.insert(
                (*name).to_string(),
                CtorInfo {
                    arity: *arity,
                    type_name: (*type_name).to_string(),
                },
            );
        }
        Env {
            scopes: Vec::new(),
            top_level_values: HashSet::new(),
            top_level_types: HashSet::new(),
            constructors,
            import_qualifiers: HashMap::new(),
            exposed_values: ExposedTable::default(),
            exposed_types: ExposedTable::default(),
            exposed_ctors: ExposedTable::default(),
            strict_qualifiers,
            registry,
        }
    }

    /// A bare declaration list with no module header/imports (`parse_decls`'s
    /// world) -- qualified references are always trusted (see module docs).
    pub fn for_decls() -> Self {
        Env::empty(false, None)
    }

    /// A full module with a real import list -- qualifiers are checked against
    /// it. `registry`, if given, additionally lets `exposing (..)` expand to
    /// real names and lets qualified/exposed references be checked against
    /// what the target module actually exports.
    pub fn for_module(imports: &[Import], registry: Option<&'a dyn ModuleRegistry>) -> Self {
        let mut env = Env::empty(true, registry);
        for import in imports {
            env.add_import(import);
        }
        env
    }

    fn add_import(&mut self, import: &Import) {
        let qualifier = match &import.alias {
            Some(alias) => alias.clone(),
            None => import.module.join("."),
        };
        self.import_qualifiers
            .insert(qualifier, import.module.clone());

        let known = self.registry.and_then(|r| r.exports(&import.module));

        match &import.exposing {
            None => {}
            Some(Exposing::All) => self.add_wildcard_exposing(&import.module, known.as_ref()),
            Some(Exposing::Some(items)) => {
                for item in items {
                    self.add_exposed_item(&import.module, item, known.as_ref());
                }
            }
        }
    }

    fn add_wildcard_exposing(&mut self, module: &[String], known: Option<&ModuleInterface>) {
        match known {
            Some(iface) => {
                for name in &iface.values {
                    self.exposed_values
                        .add_explicit(name.clone(), module.to_vec());
                }
                for name in &iface.types {
                    self.exposed_types
                        .add_explicit(name.clone(), module.to_vec());
                }
                for names in iface.constructors.values() {
                    for name in names {
                        self.exposed_ctors
                            .add_explicit(name.clone(), module.to_vec());
                    }
                }
            }
            None => {
                self.exposed_values.add_wildcard_fallback(module.to_vec());
                self.exposed_types.add_wildcard_fallback(module.to_vec());
                self.exposed_ctors.add_wildcard_fallback(module.to_vec());
            }
        }
    }

    fn add_exposed_item(
        &mut self,
        module: &[String],
        item: &ExposedItem,
        known: Option<&ModuleInterface>,
    ) {
        match item {
            ExposedItem::Value(name) => {
                self.exposed_values
                    .add_explicit(name.clone(), module.to_vec());
            }
            ExposedItem::TypeOnly(name) => {
                self.exposed_types
                    .add_explicit(name.clone(), module.to_vec());
            }
            ExposedItem::TypeWithVariants(name) => {
                self.exposed_types
                    .add_explicit(name.clone(), module.to_vec());
                match known.and_then(|iface| iface.constructors.get(name)) {
                    Some(variants) => {
                        for variant in variants {
                            self.exposed_ctors
                                .add_explicit(variant.clone(), module.to_vec());
                        }
                    }
                    None => self.exposed_ctors.add_wildcard_fallback(module.to_vec()),
                }
            }
        }
    }

    // -- top-level declaration bookkeeping (populated before any body is resolved) --

    pub fn declare_top_level_value(&mut self, name: &str) {
        self.top_level_values.insert(name.to_string());
    }

    pub fn declare_top_level_type(&mut self, name: &str) {
        self.top_level_types.insert(name.to_string());
    }

    pub fn declare_ctor(&mut self, name: &str, arity: usize, type_name: &str) {
        self.constructors.insert(
            name.to_string(),
            CtorInfo {
                arity,
                type_name: type_name.to_string(),
            },
        );
    }

    pub fn ctor_info(&self, name: &str) -> Option<&CtorInfo> {
        self.constructors.get(name)
    }

    // -- local scoping --

    pub fn push_scope(&mut self) {
        self.scopes.push(HashSet::new());
    }

    pub fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    /// Binds `name` in the innermost scope. Callers that need duplicate-in-one-
    /// pattern detection track their own set across a single pattern's sibling
    /// binders and don't rely on this for that -- shadowing across *different*
    /// lexical layers (e.g. a `let` shadowing an outer `let`) is always fine.
    pub fn bind_local(&mut self, name: &str) {
        self.scopes
            .last_mut()
            .expect("bind_local called with no open scope")
            .insert(name.to_string());
    }

    fn is_local(&self, name: &str) -> bool {
        self.scopes.iter().rev().any(|frame| frame.contains(name))
    }

    // -- resolution --

    /// Splits `name` on its *last* `.` -- everything before is the qualifier
    /// (itself possibly dotted, for a multi-segment module path), everything
    /// after is the unqualified name. `None` if `name` has no `.` at all.
    fn split_qualified(name: &str) -> Option<(&str, &str)> {
        name.rsplit_once('.')
    }

    pub fn resolve_value(&self, name: &str) -> Result<Ref, UnresolvedKind> {
        if let Some((qualifier, unqualified)) = Self::split_qualified(name) {
            return self.resolve_qualified(qualifier, unqualified, Namespace::Value);
        }
        if self.is_local(name) {
            return Ok(Ref::Local(name.to_string()));
        }
        if self.top_level_values.contains(name) {
            return Ok(Ref::TopLevel(name.to_string()));
        }
        if prelude::is_builtin_value(name) {
            return Ok(Ref::Builtin(name.to_string()));
        }
        match self.exposed_values.resolve(name) {
            ExposedLookup::Found(module) => Ok(Ref::Imported {
                module,
                name: name.to_string(),
            }),
            ExposedLookup::Ambiguous(modules) => Err(UnresolvedKind::Ambiguous(modules)),
            ExposedLookup::NotFound => Err(UnresolvedKind::Unbound),
        }
    }

    /// Constructors used as *values* (e.g. `Just` applied like a function) and
    /// constructors used in pattern position share the same namespace/table.
    pub fn resolve_ctor(&self, name: &str) -> Result<(Ref, Option<CtorInfo>), UnresolvedKind> {
        if let Some((qualifier, unqualified)) = Self::split_qualified(name) {
            let r = self.resolve_qualified(qualifier, unqualified, Namespace::Ctor)?;
            let info = self.constructors.get(unqualified).cloned();
            return Ok((r, info));
        }
        if let Some(info) = self.constructors.get(name) {
            let r = if prelude::builtin_constructor(name).is_some() {
                Ref::Builtin(name.to_string())
            } else {
                Ref::TopLevel(name.to_string())
            };
            return Ok((r, Some(info.clone())));
        }
        match self.exposed_ctors.resolve(name) {
            ExposedLookup::Found(module) => Ok((
                Ref::Imported {
                    module,
                    name: name.to_string(),
                },
                None,
            )),
            ExposedLookup::Ambiguous(modules) => Err(UnresolvedKind::Ambiguous(modules)),
            ExposedLookup::NotFound => Err(UnresolvedKind::Unbound),
        }
    }

    pub fn resolve_type(&self, name: &str) -> Result<Ref, UnresolvedKind> {
        if let Some((qualifier, unqualified)) = Self::split_qualified(name) {
            return self.resolve_qualified(qualifier, unqualified, Namespace::Type);
        }
        if self.top_level_types.contains(name) {
            return Ok(Ref::TopLevel(name.to_string()));
        }
        if prelude::is_builtin_type(name) {
            return Ok(Ref::Builtin(name.to_string()));
        }
        match self.exposed_types.resolve(name) {
            ExposedLookup::Found(module) => Ok(Ref::Imported {
                module,
                name: name.to_string(),
            }),
            ExposedLookup::Ambiguous(modules) => Err(UnresolvedKind::Ambiguous(modules)),
            ExposedLookup::NotFound => Err(UnresolvedKind::Unbound),
        }
    }

    fn resolve_qualified(
        &self,
        qualifier: &str,
        unqualified: &str,
        namespace: Namespace,
    ) -> Result<Ref, UnresolvedKind> {
        let module = if self.strict_qualifiers {
            match self.import_qualifiers.get(qualifier) {
                Some(module) => module.clone(),
                None => return Err(UnresolvedKind::UnknownQualifier),
            }
        } else {
            qualifier.split('.').map(str::to_string).collect()
        };
        if let Some(registry) = self.registry {
            if let Some(iface) = registry.exports(&module) {
                let exported = match namespace {
                    Namespace::Value => iface.values.contains(unqualified),
                    Namespace::Type => iface.types.contains(unqualified),
                    Namespace::Ctor => iface
                        .constructors
                        .values()
                        .any(|vs| vs.iter().any(|v| v == unqualified)),
                };
                if !exported {
                    return Err(UnresolvedKind::NotExported { module });
                }
            }
        }
        Ok(Ref::Imported {
            module,
            name: unqualified.to_string(),
        })
    }
}

/// Why a name didn't resolve -- callers (see `resolve/*.rs`) turn this into the
/// right `CanonErrorKind` variant, since the *kind* of name (variable vs.
/// constructor vs. type) determines which `CanonErrorKind` applies but the
/// lookup logic above is identical either way.
pub enum UnresolvedKind {
    Unbound,
    UnknownQualifier,
    Ambiguous(Vec<Vec<String>>),
    NotExported { module: Vec<String> },
}
