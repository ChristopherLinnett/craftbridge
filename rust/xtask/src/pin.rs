//! The one helper that starts a cargo for a tool of this repository, with the pinned toolchain named.
//!
//! The gate builds with one toolchain, the one `rust/rust-toolchain.toml` pins, and AGENTS.md §7 allows
//! exactly one spelling of that fact in the environment: rustup's own variable, naming the pin. It matters
//! because rustup does not start this binary. `tools/ci.sh`, `tools/build-xcframework.sh` and
//! `tools/build-android.sh` build xtask under the pin and then run the binary directly, so the cargo it
//! starts — `env!("CARGO")`, a toolchain's own cargo, not rustup's proxy — carries no `RUSTUP_TOOLCHAIN`.
//! Cargo runs each crate outside the workspace's own folder, which is every registry crate, with that
//! crate's folder as the compiler's place, and rustup reads *that* folder's toolchain file there: a crate
//! whose file names another toolchain — `dashmap` 6.2.1 asks for 1.85, `dynify` for 1.80 — stops a clean
//! machine's build with "toolchain … is not installed", and every other crate gets rustup's default,
//! which equals the pin on a given machine only by chance.
//!
//! So every cargo a tool starts is started here, and so is every program a tool starts that runs a cargo of
//! its own under the environment it is handed, as the bindgen asks cargo about the library's crates: each
//! with the pin named in that variable, read from the file's one definition through
//! [`crate::pins::toolchain_channel`] — never a second copy of the version, and never the switch §7
//! refuses, `RUSTC`. A `tools/` script a tool starts, as the upstream update runs `tools/ci.sh`, needs no
//! variable: it runs each cargo through rustup's proxy in the folder whose pin it builds with, which hands
//! that pin on, and the gate refuses any other toolchain before its first stage. A tool that could not read
//! the pin refuses to start a command rather than run it under whatever the machine defaults to. The one
//! folder whose commands start without the variable is the gate's lint library's, whose own file pins the
//! nightly §7 keeps there: rustup reads that file in that folder, as it does for every command
//! `tools/env.sh` runs there (`craftbridge_in`). `cargo dylint`, started in `rust/` like every other
//! cargo here, takes that nightly from the library's folder itself.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// The rustup variable that names a command's toolchain. The tools may set it to one value: the pin (§7).
pub(crate) const TOOLCHAIN_VARIABLE: &str = "RUSTUP_TOOLCHAIN";

/// What a command's toolchain is decided by: the channel of this tree's own toolchain file, and the
/// folder of the gate's lint library, whose own file decides there.
struct Pin {
    channel: String,
    library: PathBuf,
}

/// The pin of this tree, read once: the same value every command names, and a failure that names the
/// file if it cannot be read.
fn pin() -> Result<&'static Pin, String> {
    static PIN: OnceLock<Result<Pin, String>> = OnceLock::new();
    PIN.get_or_init(|| {
        let root = crate::repository_root()?;
        Ok(Pin { channel: crate::pins::toolchain_channel(&root)?, library: root.join(crate::machinery::LINT_LIBRARY) })
    })
    .as_ref()
    .map_err(Clone::clone)
}

/// A command for `program` — a cargo, or a program that runs one of its own under the environment it is
/// handed, as the bindgen does — run in `folder`, with the pin named to rustup, so every crate a cargo
/// under it compiles, registry crates alike, wherever their folders sit, is compiled with the pinned
/// toolchain, whoever started the tool (§7). In the lint library's folder the command starts without the
/// variable instead, so rustup reads the nightly that folder pins. Every place a tool starts such a program
/// starts it from here.
pub(crate) fn pinned_in(program: impl AsRef<OsStr>, folder: &Path) -> Result<Command, String> {
    let pin = pin()?;
    let mut command = Command::new(program);
    command.current_dir(folder);
    if crate::stands_inside(folder, &pin.library) {
        command.env_remove(TOOLCHAIN_VARIABLE);
    } else {
        command.env(TOOLCHAIN_VARIABLE, &pin.channel);
    }
    Ok(command)
}

/// The cargo that built this binary, run in `folder`, through [`pinned_in`].
pub(crate) fn cargo_in(folder: &Path) -> Result<Command, String> {
    pinned_in(env!("CARGO"), folder)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proc_macro2::TokenTree;
    use syn::visit::{self, Visit};

    /// Every variable a command's own environment sets or removes, in the order `get_envs` gives them.
    fn variables(command: &Command) -> Vec<(&OsStr, Option<&OsStr>)> {
        command.get_envs().collect()
    }

    /// The channel `rust/rust-toolchain.toml` names, read from the file on its own.
    fn channel_read_apart() -> String {
        let document: toml::Value = toml::from_str(&std::fs::read_to_string(crate::repo_root().join(crate::pins::TOOLCHAIN)).unwrap()).unwrap();
        document
            .get("toolchain")
            .and_then(|toolchain| toolchain.get("channel"))
            .and_then(toml::Value::as_str)
            .expect("the repository's file names a channel")
            .to_string()
    }

    /// The pin the helper names is the channel of `rust/rust-toolchain.toml`, read from the file on its
    /// own, in `rust/` and in any folder that stands for it, and it is the only variable the helper hands
    /// on: a second copy of the version, a reader of another file, a value that silently falls back to the
    /// machine's default, or a second variable — the `RUSTC` §7 refuses among them — would all pass a
    /// weaker test.
    #[test]
    fn every_cargo_a_tool_starts_names_the_pin_itself() {
        let channel = channel_read_apart();
        let named = vec![(OsStr::new(TOOLCHAIN_VARIABLE), Some(OsStr::new(&channel)))];
        let rust = crate::repo_root().join("rust");
        let scratch = crate::scratch::Scratch::new("pin-elsewhere");
        for folder in [rust.as_path(), scratch.0.as_path()] {
            let cargo = cargo_in(folder).unwrap();
            assert_eq!(cargo.get_program(), OsStr::new(env!("CARGO")), "the helper starts the cargo that built xtask");
            assert_eq!(cargo.get_current_dir(), Some(folder), "the helper runs the command where it was asked to");
            assert_eq!(
                variables(&cargo),
                named,
                "{}: the cargo a tool starts must name exactly the channel the one file names, and nothing else",
                folder.display()
            );
            assert_eq!(
                variables(&pinned_in("uniffi-bindgen", folder).unwrap()),
                named,
                "a program that starts a cargo of its own must be handed the pin, and nothing else"
            );
        }
    }

    /// A command in the lint library's folder, or below it, starts with the variable taken off and
    /// nothing else changed — not left as the caller left it, and never naming the workspace's pin — so
    /// the library's own file chooses its toolchain there (§7).
    #[test]
    fn a_command_in_the_lint_library_s_folder_starts_beyond_the_pin() {
        let library = crate::repo_root().join(crate::machinery::LINT_LIBRARY);
        for folder in [library.clone(), library.join("src")] {
            assert_eq!(
                variables(&cargo_in(&folder).unwrap()),
                [(OsStr::new(TOOLCHAIN_VARIABLE), None)],
                "{}: a command in the lint library's folder must remove the variable, not name the pin or leave the caller's",
                folder.display()
            );
        }
    }

    /// The names only the helper may write in what xtask ships: the cargo that built it (`env!("CARGO")`,
    /// and the variable cargo sets for what it runs), the variable that names the toolchain, and `RUSTC`,
    /// the switch §7 refuses.
    const HELPER_ONLY: &[&str] = &["CARGO", "RUSTUP_TOOLCHAIN", "RUSTC"];

    /// The programs no tool starts by name: rustup's proxies, which take the toolchain from the folder
    /// they run in.
    const PROXIES: &[&str] = &["cargo", "rustc"];

    /// Whether an item is test code: `#[test]`, or `#[cfg(test)]` as the lint-attribute rule reads that
    /// predicate (`crate::attributes::is_test`).
    fn is_test(attributes: &[syn::Attribute]) -> bool {
        attributes.iter().any(|attribute| {
            attribute.path().is_ident("test")
                || (attribute.path().is_ident("cfg")
                    && attribute.meta.require_list().is_ok_and(|list| crate::attributes::is_test(&list.tokens.clone().into_iter().collect::<Vec<_>>())))
        })
    }

    /// What one file ships that could start a cargo or a rustc beside the helper, as `syn` reads it: each
    /// line writing one of [`HELPER_ONLY`] as a string, in code or in the tokens of a macro call (`env!`,
    /// `format!`), or starting one of [`PROXIES`] by name.
    #[derive(Default)]
    struct Bypasses {
        found: Vec<String>,
    }

    impl Bypasses {
        fn name(&mut self, written: &str, line: usize) {
            if HELPER_ONLY.contains(&written) {
                self.found.push(format!("{line}: writes \"{written}\""));
            }
        }

        fn tokens(&mut self, stream: proc_macro2::TokenStream) {
            for tree in stream {
                match tree {
                    TokenTree::Group(group) => self.tokens(group.stream()),
                    TokenTree::Literal(literal) => {
                        let line = literal.span().start().line;
                        if let syn::Lit::Str(text) = syn::Lit::new(literal) {
                            self.name(&text.value(), line);
                        }
                    }
                    TokenTree::Ident(_) | TokenTree::Punct(_) => {}
                }
            }
        }
    }

    impl<'ast> Visit<'ast> for Bypasses {
        fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
            if !is_test(&item.attrs) {
                visit::visit_item_mod(self, item);
            }
        }
        fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
            if !is_test(&item.attrs) {
                visit::visit_item_fn(self, item);
            }
        }
        fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
            if !is_test(&item.attrs) {
                visit::visit_item_impl(self, item);
            }
        }
        fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
            if !is_test(&item.attrs) {
                visit::visit_impl_item_fn(self, item);
            }
        }
        fn visit_item_const(&mut self, item: &'ast syn::ItemConst) {
            if !is_test(&item.attrs) {
                visit::visit_item_const(self, item);
            }
        }
        fn visit_item_static(&mut self, item: &'ast syn::ItemStatic) {
            if !is_test(&item.attrs) {
                visit::visit_item_static(self, item);
            }
        }
        fn visit_lit_str(&mut self, text: &'ast syn::LitStr) {
            self.name(&text.value(), text.span().start().line);
        }
        fn visit_macro(&mut self, called: &'ast syn::Macro) {
            self.tokens(called.tokens.clone());
            visit::visit_macro(self, called);
        }
        fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
            if let syn::Expr::Path(function) = &*call.func
                && function
                    .path
                    .segments
                    .iter()
                    .map(|segment| segment.ident.to_string())
                    .collect::<Vec<_>>()
                    .ends_with(&["Command".to_string(), "new".to_string()])
                && let Some(syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(program), .. })) = call.args.first()
                && PROXIES.contains(&program.value().as_str())
            {
                self.found.push(format!("{}: starts {} by name", program.span().start().line, program.value()));
            }
            visit::visit_expr_call(self, call);
        }
    }

    /// No code xtask ships starts a cargo beside the helper, or names its toolchain or `RUSTC` beside it:
    /// a `Command::new(env!("CARGO"))` written outside here, in any spelling, would slip past the pin
    /// unnoticed, which is the bug this module is. Each file is read as `syn` parses it, so test code — a
    /// `#[cfg(test)]` or `#[test]` item, wherever in the file it sits — is told from what ships by the
    /// syntax tree; tests start cargo as the test itself is run under it, and may.
    #[test]
    fn only_the_helper_starts_a_tool_s_cargo() {
        let src = crate::repo_root().join("rust/xtask/src");
        let mut offenders: Vec<String> = Vec::new();
        for file in crate::scan::walk(&src, ".rs").unwrap() {
            if file.path.file_name().is_some_and(|name| name == "pin.rs") {
                continue;
            }
            let tree = syn::parse_file(&crate::scan::read_text(&file.path).unwrap()).unwrap();
            if is_test(&tree.attrs) {
                continue;
            }
            let mut bypasses = Bypasses::default();
            bypasses.visit_file(&tree);
            offenders.extend(bypasses.found.into_iter().map(|found| format!("{}:{found}", file.path.display())));
        }
        assert!(offenders.is_empty(), "the pin could be bypassed: {offenders:?}");
    }
}
