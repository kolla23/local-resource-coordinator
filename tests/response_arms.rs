//! The check of issues #238 and #242: a fallback (`_`, a binding, `else`, a method) on an answer
//! of the coordinator names its error, or says why one line above: `// response-fallback: <why>`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Attribute, Expr, Pat, Token};

const MARKER: &str = "// response-fallback:";

/// The `Client` methods that give an answer of the coordinator. With each free function that
/// takes a `Client`, they are the readers of #242; a function that only calls one is not seen.
const ANSWERS: [&str; 5] = ["call", "call_within", "can", "recv", "recv_opt"];

/// The `Client` methods that give no answer. A method in neither list fails the real check.
const NO_ANSWER: [&str; 4] = ["send", "set_read_timeout", "wait_readable", "write_quietly"];

/// The methods that turn the `Err` of a reader into a default. Not seen: a `Result` held in a
/// variable, a closure with no reader call in it, a named `ok` arm in a shared reporter (#250).
const DROPS: [&str; 10] = [
    "is_err",
    "is_err_and",
    "is_ok",
    "is_ok_and",
    "map_or",
    "map_or_else",
    "ok",
    "unwrap_or",
    "unwrap_or_default",
    "unwrap_or_else",
];

const ARM: &str = "this arm takes the Err of a coordinator reader unread";
const UNREAD: &str = "leaves the Err of a coordinator reader unread";
const LET_WILD: &str = "this let _ drops the answer of a coordinator reader";
const READER_MACRO: &str =
    "syn cannot read the body of this macro, which calls a coordinator reader";

/// What the patterns of one site name.
#[derive(Default)]
struct Names {
    response: bool,
    error: bool,
    ok_response: bool,
    err: bool,
}

/// How much one pattern takes: nothing more than it names, every `Response`, or every value.
#[derive(Clone, Copy, PartialEq, PartialOrd)]
enum Takes {
    Named,
    EveryResponse,
    Everything,
}

/// A pattern under a guard names nothing for the rule: its arm takes only some values.
fn read_path(path: &syn::Path, guarded: bool, names: &mut Names) {
    let mut segments = path.segments.iter().map(|s| s.ident.to_string());
    if segments.any(|s| s == "Response") {
        if let Some(variant) = segments.next() {
            names.response = true;
            names.error |= variant == "Error" && !guarded;
        }
    }
}

fn read_pat(pat: &Pat, guarded: bool, names: &mut Names) {
    match pat {
        Pat::Path(p) => read_path(&p.path, guarded, names),
        Pat::Struct(p) => {
            read_path(&p.path, guarded, names);
            p.fields
                .iter()
                .for_each(|f| read_pat(&f.pat, guarded, names));
        }
        Pat::TupleStruct(p) => {
            read_path(&p.path, guarded, names);
            let mut inside = Names::default();
            p.elems
                .iter()
                .for_each(|e| read_pat(e, guarded, &mut inside));
            names.ok_response |= inside.ok_response || (p.path.is_ident("Ok") && inside.response);
            names.err |= inside.err || (p.path.is_ident("Err") && !guarded);
            names.response |= inside.response;
            names.error |= inside.error;
        }
        Pat::Tuple(p) => p.elems.iter().for_each(|e| read_pat(e, guarded, names)),
        Pat::Or(p) => p.cases.iter().for_each(|e| read_pat(e, guarded, names)),
        Pat::Paren(p) => read_pat(&p.pat, guarded, names),
        Pat::Reference(p) => read_pat(&p.pat, guarded, names),
        Pat::Guard(p) => read_pat(&p.pat, true, names),
        Pat::Ident(p) => {
            if let Some((_, sub)) = &p.subpat {
                read_pat(sub, guarded, names);
            }
        }
        _ => {}
    }
}

fn takes(pat: &Pat) -> Takes {
    match pat {
        Pat::Wild(_) => Takes::Everything,
        Pat::Ident(p) => match &p.subpat {
            Some((_, sub)) => takes(sub),
            // `None` and a constant are idents too; a binding starts with a small letter.
            None if is_a_binding(&p.ident.to_string()) => Takes::Everything,
            None => Takes::Named,
        },
        Pat::Paren(p) => takes(&p.pat),
        Pat::Reference(p) => takes(&p.pat),
        Pat::Guard(p) => takes(&p.pat),
        Pat::Or(p) => p
            .cases
            .iter()
            .map(takes)
            .fold(Takes::Named, |a, b| if b > a { b } else { a }),
        Pat::TupleStruct(p)
            if (p.path.is_ident("Ok") || p.path.is_ident("Some"))
                && p.elems.len() == 1
                && takes(&p.elems[0]) != Takes::Named =>
        {
            Takes::EveryResponse
        }
        _ => Takes::Named,
    }
}

fn is_a_binding(ident: &str) -> bool {
    ident.starts_with(|c: char| c.is_lowercase() || c == '_')
}

fn is_cfg_test(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg")
            && a.parse_args::<syn::Path>()
                .is_ok_and(|p| p.is_ident("test"))
    })
}

/// What a pattern does with an `Err`: `None` takes none, `Some(true)` names the fault
/// (`Err(e)`), `Some(false)` takes it unread (`Err(_)`, `_`, a binding).
fn names_the_fault(pat: &Pat) -> Option<bool> {
    match pat {
        Pat::TupleStruct(p) if p.path.is_ident("Err") => Some(match p.elems.first() {
            Some(Pat::Ident(e)) => !e.ident.to_string().starts_with("_"),
            Some(Pat::Wild(_) | Pat::Rest(_)) | None => false,
            Some(_) => true,
        }),
        Pat::Paren(p) => names_the_fault(&p.pat),
        Pat::Reference(p) => names_the_fault(&p.pat),
        Pat::Guard(p) => names_the_fault(&p.pat),
        Pat::Or(p) => p.cases.iter().filter_map(names_the_fault).min(),
        _ if takes(pat) == Takes::Everything => Some(false),
        _ => None,
    }
}

/// The free functions that take a `Client`, and the methods of `impl Client` that take `self`.
#[derive(Default)]
struct Readers {
    free: BTreeSet<String>,
    methods: BTreeSet<String>,
}

fn is_client(ty: &syn::Type) -> bool {
    match ty {
        syn::Type::Reference(r) => is_client(&r.elem),
        syn::Type::Path(p) => p.path.segments.last().is_some_and(|s| s.ident == "Client"),
        _ => false,
    }
}

impl<'ast> Visit<'ast> for Readers {
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        let takes_a_client =
            |arg: &syn::FnArg| matches!(arg, syn::FnArg::Typed(t) if is_client(&t.ty));
        if node.sig.inputs.iter().any(takes_a_client) {
            self.free.insert(node.sig.ident.to_string());
        }
        visit::visit_item_fn(self, node);
    }

    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        if node.trait_.is_none() && is_client(&node.self_ty) {
            for item in &node.items {
                if let syn::ImplItem::Fn(f) = item {
                    if f.sig.receiver().is_some() {
                        self.methods.insert(f.sig.ident.to_string());
                    }
                }
            }
        }
        visit::visit_item_impl(self, node);
    }
}

fn parse(text: &str) -> Result<syn::File, String> {
    syn::parse_file(text).map_err(|e| {
        let at = e.span().start();
        format!("{}:{}: syn cannot read this file: {e}", at.line, at.column)
    })
}

/// Adds the readers that one source text declares.
fn read_readers(text: &str, readers: &mut Readers) -> Result<(), String> {
    readers.visit_file(&parse(text)?);
    Ok(())
}

/// The first argument of `matches!` and its pattern.
struct MatchesBody(Expr, Pat);

impl syn::parse::Parse for MatchesBody {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let value: Expr = input.parse()?;
        input.parse::<Token![,]>()?;
        let pat = Pat::parse_multi_with_leading_vert(input)?;
        if input.peek(Token![if]) {
            input.parse::<Token![if]>()?;
            input.parse::<Expr>()?;
        }
        input.parse::<Option<Token![,]>>()?;
        Ok(MatchesBody(value, pat))
    }
}

struct Check<'a> {
    lines: Vec<&'a str>,
    readers: &'a BTreeSet<String>,
    sites: usize,
    calls: usize,
    used: Vec<usize>,
    found: Vec<(usize, String)>,
}

/// True when the tokens of a macro hold the word `Response`, at any depth.
fn holds_the_word_response(tokens: proc_macro2::TokenStream) -> bool {
    tokens.into_iter().any(|tree| match tree {
        proc_macro2::TokenTree::Ident(word) => word == "Response",
        proc_macro2::TokenTree::Group(group) => holds_the_word_response(group.stream()),
        _ => false,
    })
}

impl Check<'_> {
    /// Records one finding at `line`, unless the line above it says why the code is right.
    fn flag(&mut self, line: usize, text: String) {
        let above = line.checked_sub(2).and_then(|i| self.lines.get(i));
        match above.and_then(|text| text.trim().strip_prefix(MARKER)) {
            Some(why) if !why.trim().is_empty() => self.used.push(line - 1),
            Some(_) => {
                self.used.push(line - 1);
                self.found
                    .push((line - 1, "a marker without a reason".into()));
            }
            None => self.found.push((line, text)),
        }
    }

    /// True for the call of a reader: a free function that takes a `Client`, a method of `ANSWERS`.
    fn is_a_reader(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Call(call) => match &*call.func {
                Expr::Path(p) => (p.path.segments.last())
                    .is_some_and(|s| self.readers.contains(&s.ident.to_string())),
                _ => false,
            },
            Expr::MethodCall(m) => ANSWERS.contains(&m.method.to_string().as_str()),
            _ => false,
        }
    }

    /// True when the tokens of a macro call a reader, at any depth: the name of a free reader
    /// or of a method of `ANSWERS`, directly before `(..)`.
    fn calls_a_reader(&self, tokens: proc_macro2::TokenStream) -> bool {
        let trees: Vec<proc_macro2::TokenTree> = tokens.into_iter().collect();
        trees.iter().enumerate().any(|(i, tree)| match tree {
            proc_macro2::TokenTree::Ident(word) => {
                let word = word.to_string();
                let called = matches!(trees.get(i + 1), Some(proc_macro2::TokenTree::Group(g))
                    if g.delimiter() == proc_macro2::Delimiter::Parenthesis);
                called && (ANSWERS.contains(&word.as_str()) || self.readers.contains(&word))
            }
            proc_macro2::TokenTree::Group(group) => self.calls_a_reader(group.stream()),
            _ => false,
        })
    }

    /// True while an expression holds the `Result` of a reader: behind parentheses, a reference
    /// or other methods. A `?` hands the `Err` on, and a method of `DROPS` has dropped it.
    fn holds_a_reader(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Paren(p) => self.holds_a_reader(&p.expr),
            Expr::Reference(r) => self.holds_a_reader(&r.expr),
            Expr::MethodCall(m) if !self.is_a_reader(expr) => {
                !DROPS.contains(&m.method.to_string().as_str()) && self.holds_a_reader(&m.receiver)
            }
            _ => self.is_a_reader(expr),
        }
    }

    /// A finding of #242, unless its line has a finding or a reason already: one line, one marker.
    fn flag_once(&mut self, line: usize, text: String) {
        if !self.found.iter().any(|f| f.0 == line) && !self.used.contains(&(line - 1)) {
            self.flag(line, text);
        }
    }

    /// A match on the call of a reader: each arm that takes an `Err` unread is a finding, up
    /// to the first arm with no guard that takes every `Err`.
    fn arms_of_a_reader(&mut self, node: &syn::ExprMatch) {
        if !self.holds_a_reader(&node.expr) {
            return;
        }
        for arm in &node.arms {
            let Some(named) = names_the_fault(&arm.pat) else {
                continue;
            };
            if !named {
                self.flag_once(arm.span().start().line, ARM.into());
            }
            if !matches!(arm.pat, Pat::Guard(_)) {
                break;
            }
        }
    }

    /// One pattern on a `Response`, on the call of a reader, or on both.
    fn pattern(&mut self, pat: &Pat, value: &Expr, line: usize, kind: &str) {
        self.one_pattern(pat, line, kind);
        if self.holds_a_reader(value) && names_the_fault(pat) != Some(true) {
            self.flag_once(line, format!("{kind} {UNREAD}"));
        }
    }

    /// One pattern with an `else`, or with nothing, for every value that it does not name.
    fn one_pattern(&mut self, pat: &Pat, line: usize, kind: &str) {
        let mut names = Names::default();
        read_pat(pat, false, &mut names);
        if !names.response {
            return;
        }
        self.sites += 1;
        let mut missing = Vec::new();
        if !names.error {
            missing.push("Response::Error");
        }
        if names.ok_response && !names.err {
            missing.push("Err(..)");
        }
        if !missing.is_empty() {
            self.flag(
                line,
                format!("{kind} does not name {}", missing.join(" or ")),
            );
        }
    }

    fn let_in_a_condition(&mut self, cond: &Expr, line: usize, kind: &str) {
        match cond {
            Expr::Let(l) => self.pattern(&l.pat, &l.expr, line, kind),
            Expr::Binary(b) => {
                self.let_in_a_condition(&b.left, line, kind);
                self.let_in_a_condition(&b.right, line, kind);
            }
            Expr::Paren(p) => self.let_in_a_condition(&p.expr, line, kind),
            _ => {}
        }
    }
}

impl<'ast> Visit<'ast> for Check<'_> {
    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        if !is_cfg_test(&node.attrs) {
            visit::visit_item_mod(self, node);
        }
    }

    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        if !is_cfg_test(&node.attrs) {
            visit::visit_item_fn(self, node);
        }
    }

    fn visit_expr_match(&mut self, node: &'ast syn::ExprMatch) {
        let mut names = Names::default();
        // The first arm that takes every `Response`, and the first that takes an `Err` too.
        let (mut first, mut first_wide) = (None, None);
        for arm in &node.arms {
            read_pat(&arm.pat, false, &mut names);
            let arm_takes = takes(&arm.pat);
            let line = arm.span().start().line;
            if arm_takes != Takes::Named && first.is_none() {
                first = Some(line);
            }
            if arm_takes == Takes::Everything && first_wide.is_none() {
                first_wide = Some(line);
            }
        }
        if names.response {
            self.sites += 1;
            let no_err = names.ok_response && !names.err;
            let kind = "the fallback arm of this match does not name";
            match first {
                Some(line) if !names.error && no_err && first_wide == first => {
                    self.flag(line, format!("{kind} Response::Error or Err(..)"));
                }
                Some(line) => {
                    if !names.error {
                        self.flag(line, format!("{kind} Response::Error"));
                    }
                    if let Some(wide) = first_wide.filter(|_| no_err) {
                        self.flag(wide, format!("{kind} Err(..)"));
                    }
                }
                None => {}
            }
        }
        self.arms_of_a_reader(node);
        visit::visit_expr_match(self, node);
    }

    fn visit_local(&mut self, node: &'ast syn::Local) {
        let line = node.let_token.span.start().line;
        match &node.init {
            Some(init) if init.diverge.is_some() => {
                self.pattern(&node.pat, &init.expr, line, "this let-else");
            }
            Some(init) if matches!(node.pat, Pat::Wild(_)) && self.holds_a_reader(&init.expr) => {
                self.flag_once(line, LET_WILD.into());
            }
            _ => {}
        }
        visit::visit_local(self, node);
    }

    fn visit_expr_if(&mut self, node: &'ast syn::ExprIf) {
        let kind = match node.else_branch {
            Some(_) => "this if-let with an else",
            None => "this if-let with no else",
        };
        self.let_in_a_condition(&node.cond, node.if_token.span.start().line, kind);
        visit::visit_expr_if(self, node);
    }

    fn visit_expr_while(&mut self, node: &'ast syn::ExprWhile) {
        let line = node.while_token.span.start().line;
        self.let_in_a_condition(&node.cond, line, "this while-let");
        visit::visit_expr_while(self, node);
    }

    fn visit_expr(&mut self, node: &'ast Expr) {
        self.calls += usize::from(self.is_a_reader(node));
        visit::visit_expr(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        let method = node.method.to_string();
        if DROPS.contains(&method.as_str()) && self.holds_a_reader(&node.receiver) {
            let line = node.method.span().start().line;
            self.flag_once(
                line,
                format!("this .{method}() drops the Err of a coordinator reader"),
            );
        }
        visit::visit_expr_method_call(self, node);
    }

    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        let line = node.path.span().start().line;
        if node.path.is_ident("matches") {
            if let Ok(MatchesBody(value, pat)) = node.parse_body::<MatchesBody>() {
                self.pattern(&pat, &value, line, "this matches!");
                self.visit_expr(&value);
                return;
            }
        }
        match node.parse_body_with(Punctuated::<Expr, Token![,]>::parse_terminated) {
            Ok(arguments) => arguments.iter().for_each(|e| self.visit_expr(e)),
            // No lenient path: a body that names an answer and that syn cannot read is a finding.
            Err(_) if holds_the_word_response(node.tokens.clone()) => {
                let text = "syn cannot read the body of this macro, which names a Response";
                self.flag(line, text.into());
            }
            // The shape of a drop is not read there, so any reader call in such a body is a finding.
            Err(_) if self.calls_a_reader(node.tokens.clone()) => {
                self.flag(line, READER_MACRO.into());
            }
            Err(_) => {}
        }
    }
}

/// The count of sites that read a `Response`, the count of reader calls, and one
/// `(line, text)` for each finding.
type Checked = (usize, usize, Vec<(usize, String)>);

/// Checks one source text against the free readers of the tree. Gives an `Err` when the text
/// is no Rust that syn can read.
fn check_source(text: &str, readers: &BTreeSet<String>) -> Result<Checked, String> {
    let file = parse(text)?;
    let mut check = Check {
        lines: text.lines().collect(),
        readers,
        sites: 0,
        calls: 0,
        used: Vec::new(),
        found: Vec::new(),
    };
    check.visit_file(&file);
    for (index, line) in check.lines.iter().enumerate() {
        if line.trim().starts_with(MARKER) && !check.used.contains(&(index + 1)) {
            let text = "a marker that is not directly above a fallback it excuses";
            check.found.push((index + 1, text.into()));
        }
    }
    check.found.sort();
    Ok((check.sites, check.calls, check.found))
}

fn rust_files(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, files);
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
}

/// The real check: every `.rs` file below `src/` of this package. A file that syn cannot
/// read ends the test, so no file passes unread.
#[test]
fn no_fallback_in_src_reads_a_response_without_naming_its_error() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    files.sort();
    let texts: Vec<(String, String)> = files
        .iter()
        .map(|path| {
            let name = path.strip_prefix(root).unwrap().display().to_string();
            (name, std::fs::read_to_string(path).unwrap())
        })
        .collect();
    let mut readers = Readers::default();
    for (name, text) in &texts {
        read_readers(text, &mut readers).unwrap_or_else(|e| panic!("{name}:{e}"));
    }
    let (mut sites, mut calls, mut findings) = (0, 0, 0);
    for (name, text) in &texts {
        let checked = check_source(text, &readers.free);
        let (count, read, found) = checked.unwrap_or_else(|e| panic!("{name}:{e}"));
        sites += count;
        calls += read;
        findings += found.len();
        for (line, what) in found {
            println!("{name}:{line}: {what}");
        }
    }
    println!(
        "checked {} files and {sites} sites that read a Response",
        texts.len()
    );
    println!(
        "read {calls} calls of a coordinator reader: {} free functions take a Client",
        readers.free.len()
    );
    assert!(sites > 0, "the check found no site that reads a Response");
    assert!(calls > 0, "the check found no call of a coordinator reader");
    let listed: BTreeSet<String> = ANSWERS
        .iter()
        .chain(&NO_ANSWER)
        .map(|m| m.to_string())
        .collect();
    assert_eq!(
        readers.methods, listed,
        "the methods of Client, and ANSWERS with NO_ANSWER"
    );
    assert_eq!(findings, 0, "the findings are listed above");
}

/// One row: its name, a source text, and each finding that the check must give for it.
type Row = (&'static str, &'static str, &'static [(usize, &'static str)]);

const MATCH_ERROR: &str = "the fallback arm of this match does not name Response::Error";
const MATCH_BOTH: &str = "the fallback arm of this match does not name Response::Error or Err(..)";
const MATCH_ERR: &str = "the fallback arm of this match does not name Err(..)";
const LET_ELSE: &str = "this let-else does not name Response::Error";
const NO_REASON: &str = "a marker without a reason";
const NO_USE: &str = "a marker that is not directly above a fallback it excuses";
const NO_RUST: &str = "syn cannot read this text";

/// The shapes that the fixes of the eight issues of the Gotcha removed.
const HISTORIC: &[Row] = &[
    (
        "#205 and #212: a let-else that bails with its own words",
        r#"fn gc(client: &mut Client) -> Result<i32> {
    let Response::Jobs { jobs } = client.call(&Request::List)? else {
        bail!("the coordinator did not give the job list");
    };
    Ok(0)
}"#,
        &[(2, LET_ELSE)],
    ),
    (
        "#212: a match with no arm for the error",
        r#"fn read_the_queue(client: &mut Client) -> Result<Jobs> {
    match client.call(&Request::Query { filter })? {
        Response::Found { jobs, older, .. } => Ok((jobs, older)),
        _ => bail!("the coordinator did not give the job list"),
    }
}"#,
        &[(4, MATCH_ERROR)],
    ),
    (
        "#218: every fault of a Result gives None",
        r#"fn jobs_running(client: &mut Client) -> Option<usize> {
    match client.call(&Request::Info) {
        Ok(Response::Info { jobs_running, .. }) => Some(jobs_running),
        _ => None,
    }
}"#,
        &[(4, MATCH_BOTH)],
    ),
    (
        "#221, #222 and #234: an Err under a guard, then a default",
        r#"fn coordinator_capabilities(client: &mut Client) -> Result<(String, i32)> {
    let (version, pid) = match client.call(&Request::Info) {
        Ok(Response::Info { version, pid, .. }) => (version, pid),
        Err(e) if is_a_coordinator_timeout(&e) => return Err(e),
        _ => return Ok((String::from("unknown"), 0)),
    };
    Ok((version, pid))
}"#,
        &[(5, MATCH_BOTH)],
    ),
    (
        "#234: an empty list for every other answer",
        r#"impl Client {
    fn can(&mut self, name: &str) -> Result<bool> {
        let names = match self.call(&Request::Capabilities)? {
            Response::Capabilities { names } => names,
            _ => Vec::new(),
        };
        Ok(names.iter().any(|n| n == name))
    }
}"#,
        &[(5, MATCH_ERROR)],
    ),
];

/// The shapes that those fixes wrote: each passes the rule of #238.
const FIXED: &[Row] = &[
    (
        "an arm for the error, then a fallback that bails",
        r#"fn job_list(client: &mut Client) -> Result<Vec<JobStatus>> {
    match client.call(&Request::List)? {
        Response::Jobs { jobs } => Ok(jobs),
        Response::Error { message, .. } => bail!("{message}"),
        _ => bail!("the coordinator did not give the job list"),
    }
}"#,
        &[],
    ),
    (
        "a fallback under a guard before the arm for the error",
        r#"fn capabilities(client: &mut Client, below: bool) -> Result<Vec<String>> {
    match client.call(&Request::Capabilities)? {
        Response::Capabilities { names } => Ok(names),
        _ if below => Ok(Vec::new()),
        Response::Error { message, .. } => bail!("{message}"),
        _ => bail!("the coordinator did not say what it can do"),
    }
}"#,
        &[],
    ),
    (
        "a Result with an arm for the error and an arm for every Err",
        r#"fn version(client: &mut Client) -> Result<Option<String>> {
    Ok(match client.call(&Request::Info) {
        Ok(Response::Info { version, .. }) => Some(version),
        Ok(Response::Error { message, .. }) => bail!("{message}"),
        Ok(_) => bail!("the coordinator did not give its version"),
        Err(e) if is_a_coordinator_timeout(&e) => return Err(e),
        Err(_) => None,
    })
}"#,
        &[(7, ARM)],
    ),
    (
        "a pattern that reads the error alone",
        r#"fn page(info: &Option<Response>) {
    if let Some(Response::Error { message, .. }) = info {
        eprintln!("fairq: {message}");
    }
    let Response::Error { message, .. } = answer else {
        return;
    };
}"#,
        &[],
    ),
];

/// The edges of the rule.
const RULE: &[Row] = &[
    (
        "a binding is a fallback, in a match and inside Ok",
        r#"fn f(client: &mut Client) -> Result<i32> {
    match client.call(&Request::Cancel { id })? {
        Response::Ok => Ok(0),
        other => report(other),
    }
    match client.call(&Request::Status { id }) {
        Ok(Response::Status { status }) => Some(Some(status)),
        Ok(_answer) => Some(None),
        Err(_) => None,
    }
}"#,
        &[(4, MATCH_ERROR), (8, MATCH_ERROR), (9, ARM)],
    ),
    (
        "an Err under a guard leaves the fallback with the other faults",
        r#"fn version(client: &mut Client) -> Result<Option<String>> {
    Ok(match client.call(&Request::Info) {
        Ok(Response::Info { version, .. }) => Some(version),
        Ok(Response::Error { message, .. }) => bail!("{message}"),
        Ok(_) => bail!("the coordinator did not give its version"),
        Err(e) if is_a_coordinator_timeout(&e) => return Err(e),
        _ => None,
    })
}"#,
        &[(7, MATCH_ERR)],
    ),
    (
        "an error under a guard is not the arm for the error",
        r#"fn f(answer: Response) -> i32 {
    match answer {
        Response::Ok => 0,
        Response::Error { kind, .. } if kind == ErrorKind::NoSuchJob => 127,
        _ => 0,
    }
}"#,
        &[(5, MATCH_ERROR)],
    ),
    (
        "an if-let, with an else and with none, and a while-let",
        r#"fn f(client: &mut Client) -> Result<bool> {
    let mut all_stopped = true;
    if let Response::Status { status } = client.call(&Request::Status { id })? {
        all_stopped = status.state.is_terminal();
    }
    if let Ok(Response::Status { status }) = client.call(&Request::Status { id }) {
        return Ok(true);
    } else {
        all_stopped = false;
    }
    while let Response::Event { event } = client.next()? {
        show(event);
    }
    Ok(all_stopped)
}"#,
        &[
            (3, "this if-let with no else does not name Response::Error"),
            (
                6,
                "this if-let with an else does not name Response::Error or Err(..)",
            ),
            (11, "this while-let does not name Response::Error"),
        ],
    ),
    (
        "a match inside an arm of a match that passes, and an arm of two lines",
        r#"fn f(client: &mut Client) -> Result<Option<JobStatus>> {
    match client.call(&Request::Resolve { handles })? {
        Response::Resolved { found } => {
            Ok(match client.call(&Request::Status { id: found[0] })? {
                Response::Status { status } => Some(*status),
                Response::Ok
                | _ => None,
            })
        }
        Response::Error { message, .. } => bail!("{message}"),
        _ => bail!("the coordinator did not read the ids"),
    }
}"#,
        &[(6, MATCH_ERROR)],
    ),
    (
        "a later fallback in a function whose first match passes",
        r#"fn f(client: &mut Client) -> Result<usize> {
    let jobs = match client.call(&Request::List)? {
        Response::Jobs { jobs } => jobs,
        Response::Error { message, .. } => bail!("{message}"),
        _ => bail!("the coordinator did not give the job list"),
    };
    let Response::Info { jobs_running, .. } = client.call(&Request::Info)? else {
        return Ok(0);
    };
    match client.call(&Request::Capabilities)? {
        Response::Capabilities { names } => Ok(names.len()),
        _ => Ok(0),
    }
}"#,
        &[(7, LET_ELSE), (12, MATCH_ERROR)],
    ),
    (
        "matches!, a match in the arguments of a macro, and a macro that syn cannot read",
        r#"fn f(answer: Response) {
    if matches!(answer, Response::Ok) {
        return;
    }
    assert!(matches!(answer, Response::Error { .. }));
    println!("{}", match answer {
        Response::Ok => 1,
        _ => 0,
    });
    let rows = table!(Response::Ok => 1; _ => 0);
    let other = table!(a => 1; _ => 0);
}"#,
        &[
            (2, "this matches! does not name Response::Error"),
            (8, MATCH_ERROR),
            (
                10,
                "syn cannot read the body of this macro, which names a Response",
            ),
        ],
    ),
    (
        "the code of tests is left alone, and the code after it is not",
        r#"#[cfg(test)]
mod tests {
    fn t() {
        let Response::Ok = answer() else { panic!() };
    }
}
#[cfg(not(test))]
fn g() {
    let Response::Ok = answer() else { return };
}
fn h() {
    let Response::Ok = answer() else { return };
}"#,
        &[(9, LET_ELSE), (12, LET_ELSE)],
    ),
    (
        "a path with its module, a reference, and code that reads no Response",
        r#"fn f(answer: proto::Response, kind: ErrorKind, count: Option<u32>) -> Response {
    match answer {
        proto::Response::Ok => {}
        _ => {}
    }
    match &answer {
        &Response::Ok => {}
        (&_) => {}
    }
    match answer {
        crate::proto::Response::Ok => {}
        crate::proto::Response::Error { .. } => {}
        _ => {}
    }
    match info {
        Some(Response::Info { .. }) => {}
        None => {}
    }
    match kind {
        ErrorKind::NoSuchJob => {}
        _ => {}
    }
    let Some(n) = count else {
        return Response::error(ErrorKind::Internal, "no count");
    };
    Response::Ok
}"#,
        &[(4, MATCH_ERROR), (8, MATCH_ERROR)],
    ),
];

/// The marker, and a text that is no Rust.
const MARKERS: &[Row] = &[
    (
        "a marker with a reason excuses a match, a let-else and an if-let",
        r#"fn f(client: &mut Client) -> Result<i32> {
    match client.call(&Request::Cancel { id })? {
        Response::Ok => {}
        // response-fallback: report names the error
        other => return report(other),
    }
    // response-fallback: a courtesy read; the next question reports the fault
    let Ok(Response::PauseState { queue, .. }) = client.call(&Request::PauseState) else {
        return Ok(0);
    };
    // response-fallback: #197 counts a refused record as kept and says no reason
    if let Response::Ok = client.call(&Request::Clean { id })? {
        deleted += 1;
    }
    Ok(0)
}"#,
        &[],
    ),
    (
        "a marker without a reason is refused",
        r#"fn f(client: &mut Client) -> Result<i32> {
    match client.call(&Request::Cancel { id })? {
        Response::Ok => Ok(0),
        // response-fallback:
        other => report(other),
    }
}"#,
        &[(4, NO_REASON)],
    ),
    (
        "a marker two lines above excuses nothing",
        r#"fn f(client: &mut Client) -> Result<i32> {
    match client.call(&Request::Cancel { id })? {
        Response::Ok => Ok(0),
        // response-fallback: report names the error
        // Every other answer is a fault.
        other => report(other),
    }
}"#,
        &[(4, NO_USE), (6, MATCH_ERROR)],
    ),
    (
        "a marker above a fallback that needs none is refused",
        r#"fn f(client: &mut Client) -> Result<i32> {
    match client.call(&Request::Cancel { id })? {
        Response::Ok => Ok(0),
        Response::Error { message, .. } => bail!("{message}"),
        // response-fallback: no longer needed
        _ => bail!("the coordinator gave another answer"),
    }
}"#,
        &[(5, NO_USE)],
    ),
    (
        "a Result needs one marker for each arm that the rule names",
        r#"fn f(client: &mut Client) -> Result<Option<String>> {
    Ok(match client.call(&Request::Info) {
        Ok(Response::Info { version, .. }) => Some(version),
        // response-fallback: an answer of another kind has no version
        Ok(_) => None,
        Err(e) if is_a_coordinator_timeout(&e) => return Err(e),
        _ => None,
    })
}"#,
        &[(7, MATCH_ERR)],
    ),
    (
        "a text that is no Rust is a finding, and never a pass",
        r#"fn f(answer: Response) -> i32 {
    match answer {
        Response::Ok => 0,
        _ => 1,
"#,
        &[(0, NO_RUST)],
    ),
];

const DROP_OK: &str = "this .ok() drops the Err of a coordinator reader";
const DROP_OR: &str = "this .unwrap_or() drops the Err of a coordinator reader";

/// The shapes of #242: the `Err` of a coordinator reader that a method or a pattern drops.
const DROPPED: &[Row] = &[
    (
        "#214: the ids of a wait, with .ok() on a reader inside a closure",
        r#"fn resolve_many(client: &mut Client, raws: &[String]) -> Result<Vec<Result<Targets>>> {
    read_the_ids(client.call(&Request::Resolve { handles: raws.to_vec() })?)
}
fn expand_ids(raws: &[String]) -> Result<Vec<String>> {
    let answers = Client::connect_existing().and_then(|mut c| resolve_many(&mut c, raws).ok());
    let Some(answers) = answers else {
        return Ok(raws.to_vec());
    };
    Ok(ids_of(answers))
}"#,
        &[(5, DROP_OK)],
    ),
    (
        "#226: a refused info reads as no job that operates",
        r#"fn jobs_running(client: &mut Client) -> Result<usize> {
    read_the_count(client.call(&Request::Info)?)
}
fn pause(client: &mut Client) -> Result<i32> {
    let running = jobs_running(client).unwrap_or(0);
    Ok(i32::from(running > 0))
}"#,
        &[(5, DROP_OR)],
    ),
    (
        "#214: an arm that reads every fault of a reader as a coordinator that stopped",
        r#"fn resolve_id_for_wait(client: &mut Client, raw_id: &str) -> Result<Option<Uuid>> {
    one_id(client.call(&Request::Resolve { handles: vec![raw_id.into()] })?)
}
fn wait_through_coordinator(client: &mut Client, raw_id: &str) -> Result<Option<WaitOutcome>> {
    let id = match resolve_id_for_wait(client, raw_id) {
        Ok(Some(id)) => id,
        Ok(None) => return Ok(Some(WaitOutcome::NoSuchJob)),
        Err(_) if the_limit_passed() => return Ok(Some(WaitOutcome::TimedOut)),
        Err(_) => return Ok(None),
    };
    let reason = match resolve_id_for_wait(client, raw_id) {
        Ok(Some(id)) => read_the_reason(id),
        _ => None,
    };
    Ok(None)
}"#,
        &[(8, ARM), (9, ARM), (13, ARM)],
    ),
    (
        "each method of the list, on each method of Client and behind other methods",
        r#"fn f(client: &mut Client) -> Option<bool> {
    let a = client.call(&Request::Info).ok()?;
    let b = client.can("resolve").unwrap_or(false);
    let c = client.recv().unwrap_or_default();
    let d = client.recv_opt().unwrap_or_else(|_| None);
    let e = self.call_within(&request, deadline).map_or(0, count);
    let f = client.call(&Request::List).map_or_else(|_| 0, count);
    if client.call(&Request::Ping).is_ok() {
        return None;
    }
    let late = client.call(&Request::Ping).is_err();
    let able = client.can("pause").is_ok_and(|able| able);
    let slow = client.recv().is_err_and(is_a_time_limit);
    let g = client
        .call(&Request::Info)
        .map(count)
        .unwrap_or(0);
    let h = client
        .recv_opt()
        .ok()
        .unwrap_or_default();
    Some((client.can("events")).is_ok())
}"#,
        &[
            (2, DROP_OK),
            (3, DROP_OR),
            (
                4,
                "this .unwrap_or_default() drops the Err of a coordinator reader",
            ),
            (
                5,
                "this .unwrap_or_else() drops the Err of a coordinator reader",
            ),
            (6, "this .map_or() drops the Err of a coordinator reader"),
            (
                7,
                "this .map_or_else() drops the Err of a coordinator reader",
            ),
            (8, "this .is_ok() drops the Err of a coordinator reader"),
            (11, "this .is_err() drops the Err of a coordinator reader"),
            (
                12,
                "this .is_ok_and() drops the Err of a coordinator reader",
            ),
            (
                13,
                "this .is_err_and() drops the Err of a coordinator reader",
            ),
            (17, DROP_OR),
            (20, DROP_OK),
            (22, "this .is_ok() drops the Err of a coordinator reader"),
        ],
    ),
    (
        "a let _ on a reader, and a let that drops no answer",
        r#"fn job_list(client: &mut Client) -> Result<Vec<JobStatus>> {
    read_the_jobs(client.call(&Request::List)?)
}
fn f(client: &mut Client) {
    let _ = client.call(&Request::Shutdown);
    let _ = job_list(client).map(show);
    let _ = client.send(&Request::Wait { id });
    let answer = client.call(&Request::Info);
    let _ = answer;
    let _ = client.call(&Request::Ping)?;
}"#,
        &[(5, LET_WILD), (6, LET_WILD)],
    ),
    (
        "a ? hands the Err on, and so does a pattern that names the fault",
        r#"fn jobs_running(client: &mut Client) -> Result<usize> {
    match client.call(&Request::Info)? {
        Response::Info { jobs_running, .. } => Ok(jobs_running),
        Response::Error { message, .. } => bail!("{message}"),
        _ => bail!("the coordinator did not give the number of jobs that operate"),
    }
}
fn drain(client: &mut Client) -> Result<i32> {
    if jobs_running(client)? == 0 {
        return Ok(0);
    }
    let names = client.call(&Request::Capabilities)?.names().unwrap_or_default();
    let running = match jobs_running(client) {
        Ok(running) => running,
        Err(e) if is_a_coordinator_timeout(&e) => return Err(e),
        Err(e) => bail!("the coordinator did not answer: {e}"),
    };
    if let Err(e) = client.call(&Request::Ping) {
        eprintln!("fairq: {e:#}");
    }
    let Err(fault) = jobs_running(client) else {
        return Ok(0);
    };
    Err(fault)
}"#,
        &[],
    ),
    (
        "what is no reader: other names, two hops, a variable, a method of another type",
        r#"fn jobs_running(client: &mut Client) -> Result<usize> {
    read_the_count(client.call(&Request::Info)?)
}
fn running() -> Result<usize> {
    jobs_running(&mut Client::connect()?)
}
impl Page {
    fn refresh(&mut self, client: &mut Client) -> Result<()> {
        self.show(client.call(&Request::List)?)
    }
}
fn f(page: &mut Page, client: &mut Client, child: &mut Child) -> Result<()> {
    child.kill().ok();
    client.set_read_timeout(None).ok();
    if client.send(&Request::Wait { id }).is_err() {
        return Ok(());
    }
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let status = read_status_on_disk(raw_id).unwrap_or(None);
    let two_hops = running().unwrap_or(0);
    let answer = client.call(&Request::Info);
    let held = answer.is_ok();
    let shown = page.refresh(client).is_ok();
    let late = held_result.unwrap_or_else(|_| client.call(&Request::Info));
    Ok(())
}"#,
        &[],
    ),
    (
        "the arms of a match on a reader, in their order",
        r#"fn f(client: &mut Client) -> Option<usize> {
    let a = match client.can("resolve") {
        Ok(true) => 1,
        Err(e) if is_a_coordinator_timeout(&e) => 2,
        Ok(false) | Err(_) => 3,
    };
    let b = match client.recv_opt() {
        Ok(Some(answer)) => show(answer),
        Err(e) => return report(e),
        _ => 0,
    };
    let c = match (client.recv()) {
        Ok(answer) => show(answer),
        Err(_fault) => 0,
    };
    let d = match &client.call(&Request::Ping) {
        _ if quiet => 0,
        Err(..) => 1,
        other => show(other),
    };
    match client.call(&Request::List).map(count) {
        whole => Some(whole),
    }
}"#,
        &[(5, ARM), (14, ARM), (17, ARM), (18, ARM), (22, ARM)],
    ),
    (
        "one pattern on a reader: a let-else, an if-let, a while-let and matches!",
        r#"fn f(client: &mut Client) -> Result<bool> {
    let Ok(names) = client.call(&Request::Capabilities) else {
        return Ok(false);
    };
    if let Ok(true) = client.can("resolve") {
        return Ok(true);
    }
    if let Err(_) = client.call(&Request::Ping) {
        return Ok(false);
    } else {
        show(names);
    }
    while let Ok(Some(event)) = client.recv_opt() {
        show(event);
    }
    Ok(matches!(client.can("events"), Ok(true)))
}"#,
        &[
            (
                2,
                "this let-else leaves the Err of a coordinator reader unread",
            ),
            (
                5,
                "this if-let with no else leaves the Err of a coordinator reader unread",
            ),
            (
                8,
                "this if-let with an else leaves the Err of a coordinator reader unread",
            ),
            (
                13,
                "this while-let leaves the Err of a coordinator reader unread",
            ),
            (
                16,
                "this matches! leaves the Err of a coordinator reader unread",
            ),
        ],
    ),
    (
        "a marker excuses a method, a let _ and an arm, and needs its reason",
        r#"fn f(client: &mut Client) -> usize {
    // response-fallback: #226 a refused info reads as no job that operates
    let running = client.call(&Request::Info).map(count).unwrap_or(0);
    let later = client
        .call(&Request::Info)
        // response-fallback: a courtesy notice; the next command reports the fault
        .map_or(0, count);
    // response-fallback: the coordinator stops, and may close the connection first
    let _ = client.call(&Request::Shutdown);
    let last = match client.recv() {
        Ok(answer) => show(answer),
        // response-fallback: a closed connection is a coordinator that stopped
        Err(_) => 0,
    };
    // response-fallback:
    let late = client.can("resolve").unwrap_or(false);
    // response-fallback: no reader below this line
    let text = std::fs::read_to_string(path).unwrap_or_default();
    running + later + last
}"#,
        &[(15, NO_REASON), (17, NO_USE)],
    ),
    (
        "the arguments of a macro are read, and the code of tests is left alone",
        r#"fn f(client: &mut Client) {
    println!("{}", client.call(&Request::Info).map_or(0, count));
}
#[cfg(test)]
mod tests {
    fn t(client: &mut Client) {
        let _ = client.call(&Request::Shutdown);
        assert!(client.call(&Request::Ping).is_ok());
    }
}"#,
        &[(2, "this .map_or() drops the Err of a coordinator reader")],
    ),
    (
        "a macro that syn cannot read and that calls a reader is a finding",
        r#"fn jobs_running(client: &mut Client) -> Result<usize> {
    read_the_count(client.call(&Request::Info)?)
}
fn f(client: &mut Client, jobs_running: usize, can: bool) {
    let shown = json!({ "running": jobs_running(client).unwrap_or(0) });
    let other = serde_json::json!({ "can": client.can("pause").is_ok() });
    let plain = json!({ "running": jobs_running, "can": can });
    // response-fallback: a page that shows no count when the coordinator refuses
    let marked = json!({ "running": jobs_running(client).unwrap_or(0) });
}"#,
        &[(5, READER_MACRO), (6, READER_MACRO)],
    ),
];

/// One row through both passes of the real check: the readers that its text declares, then
/// the check of that text.
fn check_row(source: &str) -> Result<Checked, String> {
    let mut readers = Readers::default();
    read_readers(source, &mut readers)?;
    check_source(source, &readers.free)
}

/// The check, row by row. A row that fails prints `FAIL` with what the check gave.
#[test]
fn each_fixture_row_gets_its_verdict() {
    let mut failed = Vec::new();
    let rows = [HISTORIC, FIXED, RULE, MARKERS].concat();
    for (index, (name, source, expected)) in rows.iter().chain(DROPPED).enumerate() {
        let (sites, calls, got) = match check_row(source) {
            Ok(checked) => checked,
            Err(_) => (1, 1, vec![(0, NO_RUST.to_string())]),
        };
        // A row with no finding passes only when the check saw what the row is about: a site
        // that reads a Response, or the call of a reader for a row of #242.
        let seen = if index < rows.len() { sites } else { calls };
        let same = seen > 0
            && got.len() == expected.len()
            && got
                .iter()
                .zip(*expected)
                .all(|(g, e)| g.0 == e.0 && g.1 == e.1);
        if same {
            println!("row ok: {name}");
        } else {
            println!("row FAIL: {name}\n    saw {sites} sites and {calls} calls, and gave {got:?}");
            failed.push(name);
        }
    }
    assert!(
        failed.is_empty(),
        "{} rows failed, listed above",
        failed.len()
    );
}
