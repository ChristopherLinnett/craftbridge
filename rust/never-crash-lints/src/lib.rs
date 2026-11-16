//! The never-crash lints clippy has no lint for (AGENTS.md §3).
//!
//! `collection_indexing` refuses `[]` on a type clippy's own index lints do not read: a map, a `VecDeque`,
//! or any type with an `Index` of its own. Each can panic on a missing key or index, and only the type checker
//! knows what a `[]` indexes, so this is a late lint pass over the typed tree. Slices, arrays and vectors
//! belong to `clippy::indexing_slicing` and strings to `clippy::string_slice`, which the workspace forbids,
//! and so does a collection held by value whose `get` returns what its index yields, which
//! `indexing_slicing` reads too: this lint leaves those to clippy wherever clippy reads them, so the gate
//! reports each index once.
//!
//! A `[]` a macro writes is the workspace's when the macro is: one defined in the crate being checked, or
//! exported by another workspace crate, is refused at its call; one from std or a dependency is not, since
//! nobody here can edit what it expands to. rustc drops a lint inside another crate's macro, so a
//! workspace macro's index is reported at the call of that macro instead; and clippy's index lints read
//! nothing inside another crate's macro, so there this lint refuses a slice's, an array's, a vector's and
//! a string's `[]` too, an array's constant index included: rustc's own bounds check of a constant index
//! reads nothing inside another crate's macro either.
//!
//! The library is gate machinery, so it meets the never-crash lints itself (AGENTS.md §3): the gate runs
//! clippy over it with the toolchain its own rust-toolchain.toml pins, under the workspace's lint table.
//! That is why the dylint entry points are written here directly: dylint's convenience macro expands to a
//! `dylint_version` export carrying an `unwrap()`, and a macro of no crate this gate lints may smuggle a
//! panicking call into a crate it does. dylint documents the exports below — `dylint_version` and
//! `register_lints` — as what a library provides and the macro as a convenience above them; what is written
//! here is that contract, in the shape dylint 6.4.0 states it in for the pinned nightly. The macro also reads
//! two variables so that a change to either relinks the library through `dylint-link`; this library is
//! linked without it, as cargo-dylint copies each build to the name the driver loads, so they are left out.

#![feature(rustc_private)]
#![warn(unused_extern_crates)]

// Including `rustc_driver` links the library against the driver's copy of the compiler's crates, which is
// the copy it must speak to when loaded; it is named for that alone, as the macro that writes this line
// itself allows.
#[allow(unused_extern_crates)]
extern crate rustc_driver;
extern crate rustc_errors;
extern crate rustc_hir;
extern crate rustc_lint;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;

use rustc_errors::{Diag, DiagCtxtHandle, Diagnostic, Level};
use rustc_hir::{Expr, ExprKind};
use rustc_lint::{LateContext, LateLintPass, LintContext};
use rustc_middle::ty::{self, Ty};
use rustc_span::{ExpnKind, FileName, RemapPathScopeComponents, Span};

rustc_session::declare_lint! {
    /// ### What it does
    /// Refuses `[]` on a map, a `VecDeque` or any other type with its own `Index`, reading or writing.
    ///
    /// ### Why is this bad?
    /// The index can panic when the key or the index is missing, and a number or key derived from input
    /// is hostile; only the type's own `Index` knows whether it panics or inserts, so every one is
    /// refused. `get()` (or `get_mut()`, or a map's `entry()`) with the missing case handled is the rule.
    /// `forbid`, so no `allow` of it compiles.
    pub COLLECTION_INDEXING,
    Forbid,
    "`[]` on a map, a `VecDeque` or a type with its own `Index` can panic on a missing key or index: use `get()`"
}

rustc_session::declare_lint_pass!(CollectionIndexing => [COLLECTION_INDEXING]);

/// The dylint protocol version this library speaks, which the driver checks on loading it.
#[allow(unsafe_code)] // the `no_mangle` attribute below is unsafe code (AGENTS.md §4): the driver's contract is the symbol's name, so the export is unavoidable
#[unsafe(no_mangle)]
pub extern "C" fn dylint_version() -> *mut std::os::raw::c_char {
    // The C ABI has no channel to report failure, and the driver frees what this returns as a `CString`,
    // so the string is built infallibly by construction: a literal `CStr` (which cannot hold an interior
    // NUL) copied into an owned one and handed over raw (AGENTS.md §3, where dylint's own macro needed an
    // `unwrap()` here).
    std::ffi::CStr::to_owned(c"0.1.0").into_raw()
}

/// The dylint entry point: the driver registers the lint through it on every compilation it checks.
/// It is written out, not declared by the macro, for the reason in the module documentation; dylint 6.4.0
/// states this exact shape for the pinned nightly, less its config initialiser, which this library needs
/// not read (it reads no dylint configuration at all).
#[allow(unsafe_code)] // as for `dylint_version`: the driver's contract is this symbol's name (AGENTS.md §4)
#[unsafe(no_mangle)]
#[allow(clippy::no_mangle_with_rust_abi)] // the driver's contract is this Rust ABI, not a C one
pub fn register_lints(_sess: &rustc_session::Session, lint_store: &mut rustc_lint::LintStore) {
    lint_store.register_lints(&[COLLECTION_INDEXING]);
    lint_store.register_late_lint_pass(Box::new(|_| Box::new(CollectionIndexing)));
}

/// Is this `[]` one clippy's forbidden index lints already read, so that the gate reports it once? Its base
/// after the compiler's own adjustments is a slice, an array, `str`, or the `Vec` and `String` they come
/// from; or it is an index on a collection held by value whose type, or one it derefs to on the way to its
/// `Index`, has an inherent `get` returning an `Option` of what the index yields, which clippy's
/// `indexing_slicing` reads as well (a map or a `VecDeque` held by value, not through a reference).
fn read_by_clippy<'tcx>(cx: &LateContext<'tcx>, expr: &Expr<'tcx>, base: &Expr<'tcx>) -> bool {
    let typeck = cx.typeck_results();
    let adjusted = typeck.expr_ty_adjusted(base).peel_refs();
    let slice_or_string = match adjusted.kind() {
        ty::Slice(_) | ty::Array(..) | ty::Str => true,
        // `Vec` is a diagnostic item and `String` a lang item, each the compiler's own name for the type.
        ty::Adt(adt, _) => cx.tcx.is_diagnostic_item(rustc_span::sym::Vec, adt.did()) || cx.tcx.lang_items().string() == Some(adt.did()),
        _ => false,
    };
    let held = typeck.expr_ty(base);
    let yields = typeck.expr_ty(expr).peel_refs();
    let by_value_with_get = matches!(held.kind(), ty::Adt(..))
        && std::iter::once(held).chain(typeck.expr_adjustments(base).iter().map(|step| step.target)).any(|step| get_returns(cx, step.peel_refs(), yields));
    slice_or_string || by_value_with_get
}

/// Does `ty`'s first inherent `get` return an `Option` of `yields`, or of a type parameter, as clippy asks?
fn get_returns<'tcx>(cx: &LateContext<'tcx>, ty: Ty<'tcx>, yields: Ty<'tcx>) -> bool {
    let Some(adt) = ty.ty_adt_def() else { return false };
    let get = cx.tcx.inherent_impls(adt.did()).iter().find_map(|implementation| {
        cx.tcx.associated_items(*implementation).filter_by_name_unhygienic(rustc_span::Symbol::intern("get")).next().filter(|item| item.is_fn())
    });
    let Some(get) = get else { return false };
    let output = cx.tcx.fn_sig(get.def_id).skip_binder().output().skip_binder();
    let ty::Adt(option, arguments) = output.kind() else { return false };
    if !cx.tcx.is_diagnostic_item(rustc_span::sym::Option, option.did()) {
        return false;
    }
    arguments.types().next().is_some_and(|inner| {
        let inner = inner.peel_refs();
        inner == yields || matches!(inner.kind(), ty::Param(_) | ty::Alias(..))
    })
}

/// Was this `[]` written in the workspace? Code outside any macro was; inside one, the innermost macro
/// whose expansion holds it decides. One defined in this crate is the workspace's, and so is one from a
/// crate compiled in this compilation's working directory: Cargo compiles every workspace member from the
/// workspace root and every other package (std's, a registry or git dependency's) from its own folder, so
/// the working directory recorded with the macro's source names whose code it is. A built-in macro has no
/// source of its own and is the compiler's.
fn written_in_the_workspace(cx: &LateContext<'_>, span: Span) -> bool {
    let source_map = cx.sess().source_map();
    let mut span = span;
    while span.from_expansion() {
        let expansion = span.ctxt().outer_expn_data();
        if let ExpnKind::Macro(..) = expansion.kind {
            if expansion.def_site.is_dummy() {
                return false;
            }
            let file = source_map.lookup_source_file(expansion.def_site.lo());
            if !file.is_imported() {
                return true;
            }
            let FileName::Real(name) = &file.name else { return false };
            let scope = RemapPathScopeComponents::DIAGNOSTICS;
            return name.embeddable_name(scope).0 == cx.sess().opts.working_dir.path(scope);
        }
        span = expansion.call_site;
    }
    true
}

/// Where a refusal can be reported: rustc drops a lint whose span lies in another crate's macro, so an
/// index a workspace macro from another crate writes is reported at the macro's call in this crate.
fn reported_at(cx: &LateContext<'_>, span: Span) -> Span {
    let mut at = span;
    while at.in_external_macro(cx.sess().source_map()) {
        at = at.ctxt().outer_expn_data().call_site;
    }
    at
}

/// The refusal, naming the indexed type so the reader knows which `get()` to reach for.
struct Refusal {
    indexed: String,
}

impl<'a> Diagnostic<'a, ()> for Refusal {
    fn into_diag(self, dcx: DiagCtxtHandle<'a>, level: Level) -> Diag<'a, ()> {
        let mut diag = Diag::new(dcx, level, format!("`[]` on `{}` can panic on a missing key or index", self.indexed));
        diag.help("use `get()` and handle the missing case (AGENTS.md §3)");
        diag
    }
}

impl<'tcx> LateLintPass<'tcx> for CollectionIndexing {
    fn check_expr(&mut self, cx: &LateContext<'tcx>, expr: &'tcx Expr<'tcx>) {
        let ExprKind::Index(base, _, _) = expr.kind else { return };
        let indexed = cx.typeck_results().expr_ty_adjusted(base).peel_refs();
        let clippy_reads_it = read_by_clippy(cx, expr, base) && !expr.span.in_external_macro(cx.sess().source_map());
        if !clippy_reads_it && written_in_the_workspace(cx, expr.span) {
            cx.emit_span_lint(COLLECTION_INDEXING, reported_at(cx, expr.span), Refusal { indexed: indexed.to_string() });
        }
    }
}
