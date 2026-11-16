//! A Rust source file as `syn` parses it: every attribute and every use of the `unsafe` keyword.
//!
//! The unsafe rule and the lint-attribute rule both ask what the language's own syntax tree holds, so
//! one visitor answers both. Attributes are found wherever `syn` finds them — on a crate root, a module,
//! an item, an expression, a field, a parameter — and `unsafe` as `syn` finds it: blocks, functions,
//! impls, traits, extern blocks, modules, function-pointer types and attributes. A macro's body is only
//! tokens to `syn`, since it means nothing before expansion, so those tokens are walked as the tree the
//! lexer made of them: an attribute there is read as it would apply to the module or function written
//! after it, a metavariable standing in for the name, and as covering a module wherever the tokens do not
//! show what it applies to, which includes any item in the tokens of a macro call, since the macro may
//! move it. A `macro_rules!` matcher is a pattern, not code, so only what each arm writes is read for
//! attributes. A file `syn` cannot parse, or one nested deeper than it can read, is an
//! error naming it, never a pass and never a crash.

use crate::attributes::{self, Allowance, Place, is_punct};
use proc_macro2::{Delimiter, Group, Span, TokenStream, TokenTree};
use quote::ToTokens;
use std::collections::BTreeSet;
use std::str::FromStr;
use syn::ext::IdentExt;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{AttrStyle, Attribute, ItemFn, ItemMod, Meta};

/// One use of the `unsafe` keyword.
#[derive(Debug)]
pub(crate) struct UnsafeUse {
    /// The 1-based line of the keyword.
    pub(crate) line: usize,
    /// The free function this use is an outer `#[unsafe(no_mangle)]` on, which is the attribute exactly:
    /// the one form of the keyword a crate that exports a symbol by name must write.
    pub(crate) exports: Option<String>,
}

/// What a file holds that the rules read.
#[derive(Debug)]
pub(crate) struct Facts {
    pub(crate) allowances: Vec<Allowance>,
    pub(crate) unsafe_uses: Vec<UnsafeUse>,
}

/// How deep brackets may nest in a file the rules read. `syn` copies a file's tokens into its buffer and
/// parses what they hold by recursion, a level of the stack for each level of brackets, so a file nested
/// past what the stack holds would abort the gate with no file named; real code nests under twenty.
const MAX_DEPTH: usize = 64;

/// How many tokens may sit on one path from a file's top level into its innermost group, counting every
/// token of each group the path passes through. Below the brackets, `syn` also recurses once for each
/// nested generic, reference, prefix operator, closure or `else if`, and each of those takes at least one
/// token of the group it sits in, so this bounds every level of recursion its parse, its visitor and the
/// tree's drop can make. The heaviest real file holds about a fifth of it.
const MAX_PATH: usize = 8_192;

/// The stack the parse runs on: in the debug build the gate runs, one level of a nested generic type
/// takes `syn` about 53 KiB over its three tokens and one level of `&` 32 KiB over its one, so twice the
/// worst per token, for `MAX_PATH` tokens. It is address space reserved, not memory used: a real file
/// touches a few hundred KiB of it.
const PARSE_STACK: usize = MAX_PATH * 64 * 1024;

/// Parse `source` and read it. `name` is how a message names the file.
pub(crate) fn read(source: &str, name: &str) -> Result<Facts, String> {
    bounded(source, name)?;
    let (owned, label) = (source.to_string(), name.to_string());
    let worker = std::thread::Builder::new()
        .name("rust-syntax".to_string())
        .stack_size(PARSE_STACK)
        .spawn(move || parse(&owned, &label))
        .map_err(|error| format!("{name}: could not start the thread that reads it: {error}"))?;
    worker.join().unwrap_or_else(|_| Err(format!("{name}: the thread that reads it stopped before it finished")))
}

/// Lex `source` without recursion, as `proc_macro2` does, and refuse it when it nests past what `syn`
/// can read on [`PARSE_STACK`].
fn bounded(source: &str, name: &str) -> Result<(), String> {
    let tokens = TokenStream::from_str(source).map_err(|error| format!("{name}:{}: not Rust that syn can parse: {error}", error.span().start().line))?;
    let top: Vec<TokenTree> = tokens.into_iter().collect();
    let too_heavy = |weight: usize, line: usize| {
        (weight > MAX_PATH).then(|| format!("{name}:{line}: more than {MAX_PATH} tokens sit on one path into the code here, more than this check reads"))
    };
    if let Some(error) = too_heavy(top.len(), 1) {
        return Err(error);
    }
    let weight = top.len();
    let mut open: Vec<(std::vec::IntoIter<TokenTree>, usize, usize)> = vec![(top.into_iter(), 0, weight)];
    while let Some((mut trees, depth, weight)) = open.pop() {
        let Some(tree) = trees.next() else { continue };
        open.push((trees, depth, weight));
        let TokenTree::Group(group) = tree else { continue };
        let inner: Vec<TokenTree> = group.stream().into_iter().collect();
        let (depth, weight) = (depth + 1, weight.saturating_add(inner.len()));
        let line = group.span_open().start().line;
        if depth > MAX_DEPTH {
            return Err(format!("{name}:{line}: brackets nest more than {MAX_DEPTH} levels deep, more than this check reads"));
        }
        if let Some(error) = too_heavy(weight, line) {
            return Err(error);
        }
        open.push((inner.into_iter(), depth, weight));
    }
    Ok(())
}

fn parse(source: &str, name: &str) -> Result<Facts, String> {
    let file = syn::parse_file(source).map_err(|error| format!("{name}:{}: not Rust that syn can parse: {error}", error.span().start().line))?;
    let mut reader = Reader::default();
    reader.visit_file(&file);
    Ok(Facts { allowances: reader.allowances, unsafe_uses: reader.unsafe_uses })
}

/// Words that may sit between an attribute and the `mod` or `fn` it applies to.
const QUALIFIERS: &[&str] = &["pub", "unsafe", "safe", "async", "const", "extern", "default"];

/// The words an item starts with after its visibility, which a `$vis` metavariable may stand before.
const ITEM_WORDS: &[&str] = &["mod", "fn", "struct", "enum", "union", "trait", "impl", "type", "static", "use"];

/// What part of a macro the tokens being walked are.
#[derive(Clone, Copy)]
enum Part<'a> {
    /// A `macro_rules!` matcher: a pattern, whose `#[…]` is no attribute on anything.
    Matcher,
    /// Code a macro writes or is handed, with what the arm's matcher binds (none outside a macro's own
    /// code). `handed` is set for the tokens of a macro call, which the macro may take apart and put
    /// together anywhere, so an attribute written there lands where the macro puts it, not where it is
    /// written.
    Code { arm: &'a Bindings, handed: bool },
}

/// What a `macro_rules!` arm's matcher binds that the code it writes may use.
#[derive(Default)]
struct Bindings {
    /// The metavariables bound inside an attribute (`a` of `#[$a:meta]`), so `#[$a]` forwards one its
    /// caller wrote, and the walk of the caller's tokens reads it there.
    in_attributes: BTreeSet<String>,
    /// The metavariables bound as one token tree (`$t:tt`), which may be any token: the `!` of a macro call
    /// among them.
    token_trees: BTreeSet<String>,
}

#[derive(Default)]
struct Reader {
    allowances: Vec<Allowance>,
    unsafe_uses: Vec<UnsafeUse>,
}

impl Reader {
    fn unsafe_at(&mut self, span: Span, exports: Option<String>) {
        self.unsafe_uses.push(UnsafeUse { line: span.start().line, exports });
    }

    fn allow(&mut self, attribute: &[TokenTree], line: usize, place: &Place, bound: &BTreeSet<String>) {
        for reading in attributes::readings(attribute) {
            // A whole attribute a metavariable fills in is one its caller wrote only where the matcher
            // binds it inside `#[…]`; any other is a level nobody wrote down, which may be any of them.
            let forwarded = reading.supplied_by.as_ref().is_none_or(|name| bound.contains(name));
            let level = if forwarded { reading.level } else { attributes::UNREADABLE_LEVEL };
            self.allowances.push(Allowance { level, lints: reading.lints, line, place: place.clone(), test_only: reading.test_only });
        }
    }

    /// One attribute `syn` found, on an owner that is `owner` unless the attribute is an inner one.
    fn attribute(&mut self, attribute: &Attribute, owner: &Place) {
        let tokens: Vec<TokenTree> = attribute.meta.to_token_stream().into_iter().collect();
        let place = if matches!(attribute.style, AttrStyle::Inner(_)) { &Place::Inner } else { owner };
        self.allow(&tokens, attribute.pound_token.span().start().line, place, &BTreeSet::new());
        let exported = match (&attribute.style, owner, &attribute.meta) {
            (AttrStyle::Outer, Place::Function(name), Meta::List(list)) if list.path.is_ident("unsafe") && is_single_ident(&list.tokens, "no_mangle") => {
                Some((name.clone(), list.path.span()))
            }
            _ => None,
        };
        match exported {
            Some((name, span)) => self.unsafe_at(span, Some(name)),
            None => self.walk(&tokens, &[], Part::Code { arm: &Bindings::default(), handed: true }),
        }
    }

    /// The tokens of a macro call, or of syntax `syn` leaves unparsed.
    fn loose(&mut self, tokens: &TokenStream, handed: bool) {
        let trees: Vec<TokenTree> = tokens.clone().into_iter().collect();
        self.walk(&trees, &[], Part::Code { arm: &Bindings::default(), handed });
    }

    /// The body of a `macro_rules!`: each arm's matcher read for the keyword alone, and what the arm
    /// writes read as code, knowing which metavariables the matcher binds inside an attribute. A body
    /// that is no list of arms, which the compiler refuses anyway, is read as code throughout.
    fn definition(&mut self, body: &[TokenTree], handed: bool) {
        let Some(arms) = arms(body) else {
            self.walk(body, &[], Part::Code { arm: &Bindings::default(), handed });
            return;
        };
        for (matcher, transcriber) in arms {
            let matcher: Vec<TokenTree> = matcher.stream().into_iter().collect();
            self.walk(&matcher, &[], Part::Matcher);
            let mut arm = Bindings::default();
            bindings(&matcher, false, &mut arm);
            let transcriber: Vec<TokenTree> = transcriber.stream().into_iter().collect();
            self.walk(&transcriber, &[], Part::Code { arm: &arm, handed });
        }
    }

    /// Walk a token tree for the keyword and, in code, for attributes and the macros it defines. `follow`
    /// is what comes after a repetition (`$( … )*`) that holds `trees`, since an attribute a repetition
    /// forwards sits before what follows. [`bounded`] has capped how deep the groups nest.
    fn walk(&mut self, trees: &[TokenTree], follow: &[TokenTree], part: Part<'_>) {
        let mut at = 0usize;
        while let Some(tree) = trees.get(at) {
            match (tree, part) {
                (TokenTree::Ident(ident), _) if ident == "unsafe" => self.unsafe_at(ident.span(), None),
                (TokenTree::Punct(pound), Part::Code { arm, handed }) if pound.as_char() == '#' => {
                    self.macro_attribute(trees, at, follow, &arm.in_attributes, handed);
                }
                (TokenTree::Ident(ident), Part::Code { handed, .. }) if ident == "macro_rules" => {
                    if let (Some(TokenTree::Punct(bang)), Some(TokenTree::Ident(_)), Some(TokenTree::Group(body))) =
                        (trees.get(at + 1), trees.get(at + 2), trees.get(at + 3))
                        && bang.as_char() == '!'
                    {
                        let body: Vec<TokenTree> = body.stream().into_iter().collect();
                        self.definition(&body, handed);
                        at += 4;
                        continue;
                    }
                }
                (TokenTree::Group(group), _) => {
                    let inner: Vec<TokenTree> = group.stream().into_iter().collect();
                    let after = if repeats(trees, at) { trees.get(at + 1..).unwrap_or_default() } else { &[] };
                    // A macro called from inside these tokens (`m! { … }`, `$m!(…)`) is handed its own
                    // group's tokens, which it may take apart as any macro call's are.
                    let part = match part {
                        Part::Code { arm, .. } if calls_macro(trees, at, arm) => Part::Code { arm, handed: true },
                        part => part,
                    };
                    self.walk(&inner, after, part);
                }
                _ => {}
            }
            at += 1;
        }
    }

    /// The attribute whose `#` is `trees[at]`. A `#` that opens no attribute written out here — one a
    /// metavariable follows (`# $brackets`), or one handed to a macro alone (`m!(#)`) — is one a macro may
    /// make an attribute of from tokens it is given apart (`$hash [allow(…)]`, `$hash $brackets`), whose
    /// level and place nobody can read before expansion. Every `#` a macro could use that way is written
    /// somewhere these walks read, so that `#` is read as such an attribute.
    fn macro_attribute(&mut self, trees: &[TokenTree], at: usize, follow: &[TokenTree], bound: &BTreeSet<String>, handed: bool) {
        let Some(TokenTree::Punct(pound)) = trees.get(at) else { return };
        let line = pound.span().start().line;
        let inner = matches!(trees.get(at + 1), Some(TokenTree::Punct(bang)) if bang.as_char() == '!');
        let open = at + 1 + usize::from(inner);
        match trees.get(open) {
            Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Bracket => {
                let tokens: Vec<TokenTree> = group.stream().into_iter().collect();
                let place = match place_after(trees.get(open + 1..).unwrap_or_default(), follow) {
                    _ if inner => Place::Inner,
                    // A macro may take its call's tokens apart and put them together elsewhere
                    // (`$hash $brackets mod inner {}`), with no `#` or brackets written in its own, so an
                    // attribute handed to one is on what the macro makes of it.
                    Place::Function(_) | Place::Other if handed => Place::Unknown,
                    place => place,
                };
                self.allow(&tokens, line, &place, bound);
            }
            _ => self.allowances.push(Allowance {
                level: attributes::UNREADABLE_LEVEL,
                lints: vec![attributes::UNREADABLE.to_string()],
                line,
                place: Place::Unknown,
                test_only: false,
            }),
        }
    }
}

fn is_single_ident(tokens: &TokenStream, name: &str) -> bool {
    let mut trees = tokens.clone().into_iter();
    matches!(trees.next(), Some(TokenTree::Ident(ident)) if ident == name) && trees.next().is_none()
}

/// The arms of a `macro_rules!` body, `(matcher) => {transcriber}` with a `;` between each, as the
/// language writes them, or `None` for a body that is not one.
fn arms(body: &[TokenTree]) -> Option<Vec<(&Group, &Group)>> {
    let mut arms = Vec::new();
    let mut rest = body;
    while let [TokenTree::Group(matcher), equals, arrow, TokenTree::Group(transcriber), tail @ ..] = rest {
        if !(is_punct(equals, '=') && is_punct(arrow, '>')) {
            return None;
        }
        arms.push((matcher, transcriber));
        rest = match tail {
            [semicolon, more @ ..] if is_punct(semicolon, ';') => more,
            _ => tail,
        };
    }
    rest.is_empty().then_some(arms)
}

/// What a matcher binds, at any depth: the metavariables inside an attribute, and those of one token tree.
fn bindings(trees: &[TokenTree], in_attribute: bool, arm: &mut Bindings) {
    for (at, tree) in trees.iter().enumerate() {
        match tree {
            TokenTree::Ident(name) if at.checked_sub(1).and_then(|before| trees.get(before)).is_some_and(|before| is_punct(before, '$')) => {
                if in_attribute {
                    arm.in_attributes.insert(name.to_string());
                }
                if let (Some(colon), Some(TokenTree::Ident(fragment))) = (trees.get(at + 1), trees.get(at + 2))
                    && is_punct(colon, ':')
                    && fragment == "tt"
                {
                    arm.token_trees.insert(name.to_string());
                }
            }
            TokenTree::Group(group) => {
                let opens_attribute = group.delimiter() == Delimiter::Bracket
                    && at.checked_sub(1).and_then(|before| trees.get(before)).is_some_and(|before| {
                        is_punct(before, '#')
                            || (is_punct(before, '!') && at.checked_sub(2).and_then(|pound| trees.get(pound)).is_some_and(|pound| is_punct(pound, '#')))
                    });
                let inner: Vec<TokenTree> = group.stream().into_iter().collect();
                bindings(&inner, in_attribute || opens_attribute, arm);
            }
            _ => {}
        }
    }
}

/// Is `trees[at]` the group of a macro call, or one a macro may make a call's group of?
/// - A `!` before it, and before that the end of the macro's path: a name or a metavariable's (`m!`,
///   `$m!`, `crate::m!`), or the repetition a macro writes one with (`$($path)::*!`).
/// - A metavariable of one token tree before it (`$m $bang { … }`), which may be the `!`, or a repetition
///   (`$($call)* { … }`), which may end in one.
///
/// Any other `!` before a group is no call: `#!` opens an inner attribute, `-> !` is the never type before
/// a body, and after an operator or an opening bracket a `!` negates, so none of those tokens is handed to
/// a macro.
fn calls_macro(trees: &[TokenTree], at: usize, arm: &Bindings) -> bool {
    let back = |by: usize| at.checked_sub(by);
    match back(1).and_then(|before| trees.get(before)) {
        Some(bang) if is_punct(bang, '!') => {
            back(2).is_some_and(|end| matches!(trees.get(end), Some(TokenTree::Ident(word)) if !is_keyword(word)) || ends_repetition(trees, end))
        }
        Some(TokenTree::Ident(name)) => {
            back(2).and_then(|dollar| trees.get(dollar)).is_some_and(|dollar| is_punct(dollar, '$')) && arm.token_trees.contains(&name.to_string())
        }
        Some(_) => back(1).is_some_and(|kleene| ends_repetition(trees, kleene)),
        None => false,
    }
}

/// Is `word` a keyword, which names no macro? A `!` after one negates (`if !{ … }`, `y if !{ … } =>`,
/// `return !(…)`), so its group is no call's. `syn` answers as it parses an identifier: it refuses every
/// strict and reserved keyword, while a weak one (`union`, `default`, `safe`, `auto`), which a macro may
/// carry as its name, and a raw identifier (`r#if`), which is how a macro is named after a keyword, are
/// identifiers.
fn is_keyword(word: &proc_macro2::Ident) -> bool {
    syn::parse2::<syn::Ident>(TokenStream::from(TokenTree::Ident(word.clone()))).is_err()
}

/// Is `trees[at]` the `*`, `+` or `?` that ends a repetition of a macro's body, with the separator, if
/// any, between the `$( … )` and it (`$($path)::*`)?
fn ends_repetition(trees: &[TokenTree], at: usize) -> bool {
    let kleene = trees.get(at).is_some_and(|tree| ['*', '+', '?'].iter().any(|kleene| is_punct(tree, *kleene)));
    // A separator is one token, which proc_macro2 lexes as up to two characters of punctuation (`::`).
    kleene
        && (1..=3).any(|back| {
            at.checked_sub(back).is_some_and(|group| {
                repeats(trees, group) && trees.get(group + 1..at).is_some_and(|between| between.iter().all(|tree| !matches!(tree, TokenTree::Group(_))))
            })
        })
}

/// Is `trees[at]` a parenthesised group a `$` opens, the repetition of a macro matcher or body?
fn repeats(trees: &[TokenTree], at: usize) -> bool {
    let before = at.checked_sub(1).and_then(|before| trees.get(before));
    matches!((before, trees.get(at)), (Some(TokenTree::Punct(dollar)), Some(TokenTree::Group(group)))
        if dollar.as_char() == '$' && group.delimiter() == Delimiter::Parenthesis)
}

/// What the attribute that ends just before `rest` applies to: past further attributes, a visibility and
/// the qualifiers of a function, a `mod` or a `fn` and its name. The name a metavariable stands for is
/// written `$name`, so a module a macro writes is still one. Where a metavariable stands for the item
/// itself, or the tokens end or start no item, the tokens do not show what it applies to.
fn place_after(rest: &[TokenTree], follow: &[TokenTree]) -> Place {
    let marks = |tree: &&TokenTree| matches!(tree, TokenTree::Punct(punct) if matches!(punct.as_char(), ',' | ';' | '*' | '+' | '?'));
    let tail: Vec<&TokenTree> = rest.iter().chain(follow.iter().skip_while(marks)).collect();
    let mut at = 0usize;
    // Past what may sit between the attribute and its item: further attributes, a repetition that
    // forwards some, a visibility or qualifier a metavariable stands for, and the written ones.
    loop {
        let skipped = match (tail.get(at), tail.get(at + 1)) {
            (Some(TokenTree::Punct(pound)), Some(TokenTree::Group(group))) if pound.as_char() == '#' && group.delimiter() == Delimiter::Bracket => 2,
            (Some(TokenTree::Punct(dollar)), Some(TokenTree::Group(group))) if dollar.as_char() == '$' && group.delimiter() == Delimiter::Parenthesis => {
                2 + tail.iter().skip(at + 2).take_while(|tree| marks(tree)).count()
            }
            (Some(TokenTree::Punct(dollar)), Some(TokenTree::Ident(_))) if dollar.as_char() == '$' && at_item_word(&tail, at + 2) => 2,
            (Some(TokenTree::Ident(word)), _) if QUALIFIERS.iter().any(|qualifier| word == qualifier) => 1,
            (Some(TokenTree::Literal(_)), _) => 1,
            (Some(TokenTree::Group(group)), _)
                if group.delimiter() == Delimiter::Parenthesis
                    && at
                        .checked_sub(1)
                        .and_then(|before| tail.get(before))
                        .is_some_and(|before| matches!(before, TokenTree::Ident(word) if word == "pub")) =>
            {
                1
            }
            _ => 0,
        };
        if skipped == 0 {
            break;
        }
        at += skipped;
    }
    let name = match (tail.get(at + 1), tail.get(at + 2)) {
        (Some(TokenTree::Ident(name)), _) => Some(name.unraw().to_string()),
        (Some(TokenTree::Punct(dollar)), Some(TokenTree::Ident(name))) if dollar.as_char() == '$' => Some(format!("${name}")),
        _ => None,
    };
    match (tail.get(at), name) {
        (Some(TokenTree::Ident(keyword)), Some(name)) if keyword == "mod" => Place::Module(name),
        (Some(TokenTree::Ident(keyword)), Some(name)) if keyword == "fn" => Place::Function(name),
        (Some(TokenTree::Ident(keyword)), None) if keyword == "mod" => Place::Unknown,
        (Some(TokenTree::Ident(_) | TokenTree::Literal(_) | TokenTree::Group(_)), _) => Place::Other,
        _ => Place::Unknown,
    }
}

/// Is the word at `tail[at]` one an item starts with after its visibility, or a qualifier before one?
fn at_item_word(tail: &[&TokenTree], at: usize) -> bool {
    matches!(tail.get(at), Some(TokenTree::Ident(word)) if ITEM_WORDS.iter().chain(QUALIFIERS).any(|item| word == item))
}

/// Visit methods for syntax that may hold tokens `syn` leaves unparsed (`Verbatim`), whose attributes and
/// keywords are walked as a macro's tokens are.
macro_rules! unparsed {
    ($($method:ident($node:ident): $type:ident,)*) => {
        $(
            fn $method(&mut self, node: &'ast syn::$type) {
                if let syn::$type::Verbatim(tokens) = node {
                    self.loose(tokens, false);
                }
                visit::$method(self, node);
            }
        )*
    };
}

impl<'ast> Visit<'ast> for Reader {
    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        self.attribute(attribute, &Place::Other);
    }

    fn visit_item_mod(&mut self, module: &'ast ItemMod) {
        let place = Place::Module(module.ident.unraw().to_string());
        for attribute in &module.attrs {
            self.attribute(attribute, &place);
        }
        if let Some(token) = &module.unsafety {
            self.unsafe_at(token.span, None);
        }
        if let Some((_, items)) = &module.content {
            for item in items {
                self.visit_item(item);
            }
        }
    }

    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        let place = Place::Function(function.sig.ident.unraw().to_string());
        for attribute in &function.attrs {
            self.attribute(attribute, &place);
        }
        self.visit_signature(&function.sig);
        self.visit_block(&function.block);
    }

    fn visit_signature(&mut self, signature: &'ast syn::Signature) {
        if let Some(token) = &signature.unsafety {
            self.unsafe_at(token.span, None);
        }
        visit::visit_signature(self, signature);
    }

    fn visit_expr_unsafe(&mut self, block: &'ast syn::ExprUnsafe) {
        self.unsafe_at(block.unsafe_token.span, None);
        visit::visit_expr_unsafe(self, block);
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        if let Some(token) = &item.unsafety {
            self.unsafe_at(token.span, None);
        }
        visit::visit_item_impl(self, item);
    }

    fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
        if let Some(token) = &item.unsafety {
            self.unsafe_at(token.span, None);
        }
        visit::visit_item_trait(self, item);
    }

    fn visit_item_foreign_mod(&mut self, item: &'ast syn::ItemForeignMod) {
        if let Some(token) = &item.unsafety {
            self.unsafe_at(token.span, None);
        }
        visit::visit_item_foreign_mod(self, item);
    }

    fn visit_type_bare_fn(&mut self, item: &'ast syn::TypeBareFn) {
        if let Some(token) = &item.unsafety {
            self.unsafe_at(token.span, None);
        }
        visit::visit_type_bare_fn(self, item);
    }

    fn visit_macro(&mut self, call: &'ast syn::Macro) {
        if call.path.is_ident("macro_rules") {
            let body: Vec<TokenTree> = call.tokens.clone().into_iter().collect();
            self.definition(&body, false);
        } else {
            self.loose(&call.tokens, true);
        }
        visit::visit_macro(self, call);
    }

    unparsed! {
        visit_item(item): Item,
        visit_expr(expr): Expr,
        visit_type(ty): Type,
        visit_pat(pat): Pat,
        visit_foreign_item(item): ForeignItem,
        visit_impl_item(item): ImplItem,
        visit_trait_item(item): TraitItem,
        visit_type_param_bound(bound): TypeParamBound,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_nested_deeper_than_syn_reads_is_an_error_naming_it_not_a_crash() {
        // `syn` overflows any stack on each of these, which aborts the gate naming no file.
        let parens = format!("fn f() -> u8 {{\n    {}1{}\n}}\n", "(".repeat(5_000), ")".repeat(5_000));
        let error = read(&parens, "deep.rs").unwrap_err();
        assert!(error.starts_with("deep.rs:2: brackets nest more than 64 levels deep"), "{error}");
        let tokens = format!("fn f() {{ m!({}{}); }}\n", "[".repeat(100_000), "]".repeat(100_000));
        assert!(read(&tokens, "deep.rs").unwrap_err().contains("brackets nest more than 64 levels deep"));
        let references = format!("type T = {}u8;\n", "& ".repeat(MAX_PATH + 1));
        let error = read(&references, "long.rs").unwrap_err();
        assert!(error.starts_with("long.rs:1: more than 8192 tokens sit on one path"), "{error}");
        let sum = format!("fn f() -> i32 {{ 1{} }}\n", " + 1".repeat(MAX_PATH));
        assert!(read(&sum, "long.rs").unwrap_err().contains("more than 8192 tokens sit on one path"));
    }

    #[test]
    fn nesting_within_the_bounds_is_read_on_a_stack_that_holds_it() {
        // Past what a debug build's 8 MiB main thread holds (about 250 levels of `&`, 150 of `Vec<`).
        for source in [
            format!("type T = {}u8;\n", "& ".repeat(1_000)),
            format!("type T = {}u8{};\n", "Vec<".repeat(500), ">".repeat(500)),
            format!("fn f() -> u8 {{ {}1{} }}\n", "(".repeat(MAX_DEPTH - 1), ")".repeat(MAX_DEPTH - 1)),
        ] {
            let facts = read(&source, "x.rs");
            assert!(facts.is_ok(), "{facts:?}");
        }
    }

    /// Words and brackets the reader treats specially, put together at random, so its own walk of a
    /// macro's tokens meets every shape of them it can.
    fn soup(next: &mut impl FnMut() -> usize, depth: usize, out: &mut String) {
        const WORDS: &[&str] = &[
            "#",
            "!",
            "$",
            "a",
            ":",
            "tt",
            "meta",
            "item",
            "=>",
            ";",
            ",",
            "*",
            "?",
            "allow",
            "expect",
            "cfg_attr",
            "test",
            "rustfmt",
            "::",
            "skip",
            "rustfmt_skip",
            "clippy",
            "expect_used",
            "unsafe_code",
            "mod",
            "fn",
            "pub",
            "unsafe",
            "macro_rules",
            "r#allow",
            "\"#\"",
            "'#'",
            "r#\"x\"#",
            "/* # */",
            "// #\n",
            "1",
            "\n",
        ];
        for _ in 0..next() % 10 {
            if depth < 4 && next().is_multiple_of(6) {
                let (open, close) = [("(", ")"), ("[", "]"), ("{", "}")][next() % 3];
                out.push_str(open);
                soup(next, depth + 1, out);
                out.push_str(close);
            } else {
                out.push_str(WORDS[next() % WORDS.len()]);
            }
            out.push(' ');
        }
    }

    #[test]
    fn the_reader_never_crashes_on_any_input() {
        // A rule checker that crashes on half-written source is worse than one that misses a match: any
        // text is read or reported as unreadable, by name, and never panics, here or in the parse.
        let mut state: u64 = 20261117;
        let mut next = move || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (state >> 33) as usize
        };
        for _ in 0..3_000 {
            let mut body = String::new();
            soup(&mut next, 0, &mut body);
            let text = match next() % 5 {
                0 => body,
                1 => format!("fn f() {{ m! {{ {body} }} }}\n"),
                2 => format!("macro_rules! m {{ ({body}) => {{ {body} }}; }}\n"),
                3 => format!("macro_rules! m {{ {body} }}\nmacro_rules! n {{ () => {{ macro_rules! o {{ {body} }} }}; }}\n"),
                _ => format!("macro_rules! m {{ ($p:tt $b:tt) => {{ {body} }}; }}\nm!({body});\n"),
            };
            match std::panic::catch_unwind(|| read(&text, "x.rs")) {
                Ok(Err(error)) => assert!(error.starts_with("x.rs:") && !error.contains("stopped before it finished"), "{text:?}: {error}"),
                Ok(Ok(_)) => {}
                Err(_) => panic!("the reader panicked on {text:?}"),
            }
        }
    }
}
